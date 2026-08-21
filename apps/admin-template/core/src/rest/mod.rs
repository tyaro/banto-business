//! REST surface for the embedded server (spec §11.1): exposes the same
//! services used by `src-tauri`'s Tauri commands over HTTP, so a
//! LAN browser's `HttpDataProvider` (Phase B,
//! `packages/admin-core/src/providers/tauri.ts` is the wire contract it
//! must match) hits the exact same service layer and DB.
//!
//! ## Route table
//!
//! | Method | Path               | Body           | Response              |
//! |--------|--------------------|----------------|------------------------|
//! | GET    | `/api/auth/status`   | -              | `{initialized}` (NO auth required) |
//! | POST   | `/api/auth/setup`     | `{username,password,displayName}` | `{success,error?,token?}` (needs `allow_setup`) |
//! | POST   | `/api/auth/login`    | `{username,password}` | `{success,error?,token?}` |
//! | POST   | `/api/auth/logout`   | -              | 200                    |
//! | GET    | `/api/auth/check`    | -              | `bool`                 |
//! | GET    | `/api/auth/identity` | -              | `Identity \| null`     |
//! | POST   | `/api/auth/change-password` | `{currentPassword,newPassword}` | `{success}` (auth required) |
//! | GET    | `/api/events`        | -              | SSE stream of `ServerEvent` |
//! | POST   | `/api/customers/list` | `ListParams`  | `ListResult<Customer>` (any role) |
//! | GET    | `/api/customers/{id}` | -            | `Customer` (any role)   |
//! | POST   | `/api/customers`     | `CustomerInput` | `Customer` (editor+)  |
//! | PUT    | `/api/customers/{id}` | `CustomerInput` | `Customer` (editor+) |
//! | DELETE | `/api/customers/{id}` | -            | 204 (editor+)           |
//! | POST   | `/api/projects/list` | `ListParams`   | `ListResult<Project>` (any role) |
//! | GET    | `/api/projects/{id}` | -              | `Project` (any role)    |
//! | POST   | `/api/projects`      | `ProjectInput` | `Project` (editor+)     |
//! | PUT    | `/api/projects/{id}` | `ProjectInput` | `Project` (editor+)     |
//! | DELETE | `/api/projects/{id}` | -              | 204 (editor+)           |
//! | POST   | `/api/work_categories/list` | `ListParams` | `ListResult<WorkCategory>` (any role) |
//! | POST   | `/api/expense_categories/list` | `ListParams` | `ListResult<ExpenseCategory>` (any role) |
//! | PUT    | `/api/cost_rates/{id}` | `CostRateValues` | `WorkCategory` (editor+) |
//! | POST   | `/api/work_logs/list` | `ListParams`  | `ListResult<WorkLog>` (any role) |
//! | GET    | `/api/work_logs/{id}` | -            | `WorkLog` (any role)    |
//! | POST   | `/api/work_logs`     | `WorkLogInput` | `WorkLog` (editor+)     |
//! | PUT    | `/api/work_logs/{id}` | `WorkLogInput` | `WorkLog` (editor+)    |
//! | DELETE | `/api/work_logs/{id}` | -            | 204 (editor+)           |
//! | POST   | `/api/expenses/list` | `ListParams`   | `ListResult<Expense>` (any role) |
//! | GET    | `/api/expenses/{id}` | -              | `Expense` (any role)    |
//! | POST   | `/api/expenses`      | `ExpenseInput` | `Expense` (editor+)     |
//! | PUT    | `/api/expenses/{id}` | `ExpenseInput` | `Expense` (editor+)     |
//! | DELETE | `/api/expenses/{id}` | -              | 204 (editor+)           |
//! | POST   | `/api/trips/list`    | `ListParams`   | `ListResult<Trip>` (any role) |
//! | GET    | `/api/trips/{id}`    | -              | `Trip` (any role)       |
//! | GET    | `/api/trips/{id}/linked-counts` | -  | 紐づく工数・経費の件数 (any role) |
//! | POST   | `/api/trips`         | `TripInput`    | `TripGenerationResult` (editor+, 一括生成) |
//! | PUT    | `/api/trips/{id}`    | `TripInput`    | `Trip` (editor+)        |
//! | DELETE | `/api/trips/{id}`    | -              | 204 (editor+, 生成物は trip_id を NULL 化) |
//! | POST   | `/api/calendar/list` | `ListParams`   | `ListResult<CalendarDay>` (any role, `month` フィルタ必須) |
//! | GET    | `/api/profitability/{id}` | -           | `ProjectProfitability` (any role, id は案件 id) |
//! | POST   | `/api/invoices/list` | `ListParams`   | `ListResult<Invoice>` (any role) |
//! | POST   | `/api/invoices/candidates` | `CandidateQuery` | `CandidateLine[]` (any role, 未請求の工数・経費) |
//! | GET    | `/api/invoices/{id}` | -              | `InvoiceDetail` (any role)  |
//! | POST   | `/api/invoices`      | `InvoiceInput` | `InvoiceDetail` (editor+)   |
//! | PUT    | `/api/invoices/{id}` | `InvoiceInput` | `InvoiceDetail` (editor+, Draft のみ) |
//! | DELETE | `/api/invoices/{id}` | -              | 204 (editor+, Draft のみ)   |
//! | POST   | `/api/invoices/{id}/issue` | -        | `InvoiceDetail` (editor+, 確定・採番) |
//! | POST   | `/api/invoices/{id}/cancel` | -       | `InvoiceDetail` (editor+, 赤伝) |
//! | GET    | `/api/issuer`        | -              | `IssuerSettings` (admin)    |
//! | PUT    | `/api/issuer`        | `IssuerInput`  | `IssuerSettings` (admin)    |
//! | POST   | `/api/payments/list` | `ListParams`   | `ListResult<Payment>` (any role) |
//! | GET    | `/api/payments/{id}` | -              | `PaymentDetail` (any role)  |
//! | POST   | `/api/payments`      | `PaymentInput` | `PaymentDetail` (editor+)   |
//! | PUT    | `/api/payments/{id}` | `PaymentInput` | `PaymentDetail` (editor+)   |
//! | DELETE | `/api/payments/{id}` | -              | 204 (editor+)               |
//! | GET    | `/api/settlements/{id}` | -           | `InvoiceSettlement` (any role, id は請求書 id・導出値) |
//! | POST   | `/api/outstanding/list` | `ListParams` | `ListResult<InvoiceSettlement>` (any role, 未入金一覧) |
//! | POST   | `/api/sync/handshake` | `HandshakeRequest` | `Handshake` (any role, デバイス番号と進捗) |
//! | POST   | `/api/sync/pull`     | `PullRequest`  | `Pull` (any role, PC 側で変わった行) |
//! | GET    | `/api/users`         | -              | `UserSummary[]` (admin) |
//! | POST   | `/api/users`         | `{username,password,displayName,role}` | `UserIdentityResponse` (admin) |
//! | PUT    | `/api/users/{id}`    | `{displayName,role}` | `UserSummary` (admin) |
//! | POST   | `/api/users/{id}/reset-password` | `{newPassword}` | `{success}` (admin) |
//! | DELETE | `/api/users/{id}`    | -              | 204 (admin)             |
//! | GET    | `/api/ui-settings/{key}` | -          | `{value: string \| null}` (any role) |
//! | PUT    | `/api/ui-settings/{key}` | `{value}`  | 204 (any role)          |
//! | POST   | `/api/audit-log/list` | `ListParams`   | `ListResult<AuditLogEntry>` (admin) |
//! | GET    | `/api/audit-log/config` | -            | `AuditSettings` (admin) |
//! | PUT    | `/api/audit-log/config` | `AuditSettings` | `AuditSettings` (admin) |
//! | GET    | `/api/system/info`   | -              | `SystemInfo` (admin, M-review 2026-08 §2.4) |
//! | POST   | `/api/backups`        | -              | `BackupInfo` (admin, spec M17) |
//! | GET    | `/api/backups`        | -              | `BackupInfo[]` (admin)  |
//! | GET    | `/api/backups/{fileName}` | -          | raw bytes, `Content-Disposition: attachment` (admin) |
//! | POST   | `/api/backups/restore?fileName=` | raw bytes (`application/octet-stream`) | 204 (admin) |
//! | POST   | `/api/backups/{fileName}/restore` | -   | 204 (admin)             |
//! | GET    | `/api/backups/pending-restore` | -      | `PendingRestoreInfo \| null` (admin) |
//! | DELETE | `/api/backups/pending-restore` | -      | 204 (admin)             |
//! | POST   | `/api/attachments/list` | `{resource,resourceId}` | `AttachmentMeta[]` (any role, spec M20) |
//! | GET    | `/api/attachments/{id}/download` | -    | raw bytes, `Content-Disposition: attachment` (any role) |
//! | GET    | `/api/attachments/{id}/thumbnail` | -   | `image/jpeg`, 404 if none (any role) |
//! | POST   | `/api/attachments?resource=&resourceId=&fileName=` | raw bytes (`application/octet-stream`) | `AttachmentMeta` (editor+) |
//! | DELETE | `/api/attachments/{id}` | -              | 204 (editor+)           |
//!
//! ## Where each router lives (theme C PR-C4, docs/template-scope.md §7 移行順 ④)
//!
//! The table above stays here - it is the artifact conventions §1 designates
//! as the REST side of the Tauri⇔REST correspondence table, and
//! `scripts/verify-architecture.mjs` rule 8 anchors on it. The router
//! IMPLEMENTATIONS, however, are split by who owns them:
//!
//! - App-specific, and therefore this crate's: the Business ドメイン
//!   (`/api/customers/*` 以下 `/api/payments/*` まで) and `attachments`
//!   (`/api/attachments/*`, M20 - `resource`/`resourceId` are app data).
//! - Domain-agnostic, and therefore `banto_server::routes`: the
//!   `/api/auth/*` extras, `/api/users/*`, `/api/audit-log/*`,
//!   `/api/backups/*` and `/api/ui-settings/*`. Identical in every adopter,
//!   so they are a dependency rather than copied code. The RBAC/audit
//!   helpers the app's own routers use ([`RoleGuard`],
//!   [`require_role_at_least`], [`record_write`], [`actor_identity`]) come
//!   from there too.
//!
//! [`api_router`] below is the assembly: it owns the `.merge()` order,
//! the CSRF layer, and nothing else.
//!
//! `/api/ui-settings/*` (spec M12 SettingsProvider migration): per-user UI
//! settings (theme/preset/dock layout), namespaced by the caller's own
//! `username` (`SettingsService::ui_get`/`ui_set` - see that module for the
//! `ui.{username}.{key}` storage key scheme). Guarded by `require_auth`
//! alone - unlike the domain resources/`users`, there is no role floor: a `viewer` may
//! freely read/write their OWN UI preferences, they just cannot touch
//! anyone else's (there is no way to name another user's key over this
//! wire - `username` always comes from the caller's own bearer token, never
//! a request parameter).
//!
//! `/api/auth/status` and `/api/auth/setup` are deliberately NOT behind
//! `require_auth` - the login page needs `status` before any session exists,
//! and `setup` is how the very first session gets created. `setup` is
//! additionally gated by an `allow_setup` flag (spec §8.2): the Tauri app
//! always passes `false` (desktop first-run goes through the `auth_setup`
//! Tauri command instead, spec §10), while `banto-serve` enables it via
//! `BANTO_ALLOW_SETUP=1` so this REST path is exercisable standalone.
//!
//! `POST /api/customers/list` (rather than `GET` with query-string encoded
//! `ListParams`) is chosen deliberately: `ListParams` (sort/filters/
//! pagination, spec §3.2) is a nested structure, and sending it as a JSON
//! body makes the wire shape byte-for-byte identical to what
//! `DataProvider.getList`'s `HttpDataProvider` implementation (Phase B)
//! sends, with no query-string (de)serialization layer to keep in sync.
//!
//! Every `/api/*` route requires the `X-Banto-Client: banto` header
//! (`banto_server::csrf`) and, except for the auth routes themselves, a
//! valid bearer token (`banto_server::auth::require_auth`).
//!
//! ## RBAC (spec M10, `docs/roadmap.md`)
//!
//! On top of `require_auth` (valid session, any role), mutating domain
//! routes and all `/api/users` routes are additionally gated by
//! [`require_role_at_least`]: it re-resolves the bearer token to an
//! [`banto_server::Identity`], parses `Identity.role` into [`Role`], and rejects with
//! `403 { "kind": "forbidden" }` (`banto_core::ErrorBody::Forbidden`) if the
//! caller's role is not at least the route's minimum. `viewer` can read
//! (domain list/get); `editor` and up can also write; only `admin` can
//! manage other accounts.
//!
//! ## Audit log (spec M14, `docs/roadmap.md`)
//!
//! Every mutating handler above (domain/`users` create/update/delete,
//! password reset, self-service password change) records a
//! `crate::audit::AuditEntry` to [`crate::audit::AuditLogService`] once its
//! underlying service call has already succeeded (`origin: "rest"`);
//! [`require_role_at_least`] records `action: "denied"` when an
//! authenticated caller's role is too low; [`audited_credential_verifier`]
//! records `login`/`login_failed`; [`audit_logout_middleware`] records
//! `logout`; and `auth_setup_handler` records `setup`. Read routes
//! (`list`/`get`) are never audited. The trail itself is only readable via
//! `POST /api/audit-log/list`, `admin`-only.
//!
//! `/api/backups/*` (spec M17): `admin`-only, guarded the same way
//! `/api/users/*`/`/api/audit-log/*` are. `POST /api/backups` records
//! `action: "backup"`; either restore-staging route records
//! `action: "restore_staged"`; `DELETE /api/backups/pending-restore` records
//! `action: "restore_cancelled"` - all `resource: "backups"`. Reads (`GET
//! /api/backups`, the per-file download, `GET .../pending-restore`) are
//! never audited, same "read routes are never audited" convention as
//! everywhere else in this module. `action: "restore_applied"` is
//! deliberately NEVER recorded from here - a staged restore is only ever
//! APPLIED at the next process start, before any REST router (or pool) even
//! exists yet (spec M17: "稼働中のプールの差し替えはしない") - see
//! `crate::backup::BackupService::apply_pending_restore_at_startup`'s doc
//! comment and its callers in `src-tauri`'s `run()`/`bin/banto-serve.rs`'s
//! `main`, which record that entry themselves once a fresh `AuditLogService`
//! exists. `POST /api/backups/restore`'s request body is raw bytes
//! (`Content-Type: application/octet-stream`), not JSON or multipart - this
//! workspace has no multipart dependency (spec M17 design decision:
//! "依存追加はしない") - with the uploaded file's original name passed as a
//! `?fileName=` query parameter purely for the audit `detail`/error
//! messages, never as a filesystem path (the actual bytes are always staged
//! under the service's own fixed `restore-pending.sqlite3` name - see
//! `crate::backup::BackupService::stage_restore_from_bytes`).
//!
//! `/api/sync/*` (Phase 8, `docs/domain/sync.md` 11節): スマホ（Pixel）が
//! 自宅 LAN に戻ったときに PC へ話しかけるための入口。**この段は読み取り
//! だけ**で、PC 側の DB を一切書き換えない —— スマホ側の変更を取り込む
//! push は次段。したがって監査もしない（読み取りは監査しない、conventions
//! §1）。話しかける向きが常にスマホ → PC の一方向なので、Tauri 側に対と
//! なるコマンドは無い（PC は受けるだけ、スマホは HTTP クライアント
//! として呼ぶだけ）。
//!
//! `/api/attachments/*` (spec `docs/attachments-plan.md` §3.5, M20 unit B):
//! same read/write RBAC split as the domain routes (`viewer`+ read,
//! `editor`+ write),
//! backed by `banto_attachments::AttachmentsService`. Upload is raw `Bytes`
//! with metadata on the query string, same "no multipart dependency" design
//! as `/api/backups/restore` above; `?fileName=` here IS actually used
//! (as display/`Content-Disposition` text, never a filesystem path - see
//! `banto_attachments`'s module doc comment). `POST /api/attachments`
//! records `action: "create"`, `DELETE /api/attachments/{id}` records
//! `action: "delete"`, both `resource: "attachments"` with
//! `{fileName,sizeBytes,parentResource,parentId}` detail. Reads (`list`/
//! `download`/`thumbnail`) are never audited. `AttachmentsService` itself
//! has no `ServerEvent`/`banto-server` awareness (a deliberate crate
//! boundary - see that crate's module doc comment), so
//! [`attachments_upload`]/[`attachments_delete`] broadcast
//! `ServerEvent::ResourceChanged { resource: "attachments" }` directly,
//! reusing the same `broadcast::Sender` [`api_router`] already threads
//! through for SSE.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use banto_attachments::{AttachmentMeta, AttachmentsService, NewAttachment, MAX_ATTACHMENT_BYTES};
use banto_core::{BantoError, ListParams, ListResult};
use banto_server::routes::{
    actor_identity, audit_log_router, audit_logout_middleware, backups_router, extra_auth_router,
    record_write, require_role_at_least, system_info_router, ui_settings_router, users_router,
    LogoutAuditState, RoleGuard,
};
use banto_server::{
    auth_routes, require_auth, require_banto_client_header, sse_route, ApiError, AuthState,
    ServerEvent,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;

use crate::audit::AuditLogService;
use crate::backup::BackupService;
use crate::calendar::{month_from_params, CalendarDay, CalendarService, MONTH_FILTER};
use crate::customers::{Customer, CustomerInput, CustomersService};
use crate::expenses::{Expense, ExpenseInput, ExpensesService};
use crate::invoices::{
    CandidateLine, CandidateQuery, Invoice, InvoiceDetail, InvoiceInput, InvoicesService,
};
use crate::issuer::{IssuerInput, IssuerService, IssuerSettings};
use crate::masters::{
    CostRateInput, CostRateValues, ExpenseCategory, MastersService, WorkCategory,
};
use crate::payments::{InvoiceSettlement, Payment, PaymentDetail, PaymentInput, PaymentsService};
use crate::profitability::{ProfitabilityService, ProjectProfitability};
use crate::projects::{Project, ProjectInput, ProjectsService};
use crate::settings::SettingsService;
use crate::sync::protocol::{Handshake, HandshakeRequest, Pull, PullRequest, SyncService};
use crate::system_info::SystemInfoService;
use crate::trips::{Trip, TripGenerationResult, TripInput, TripsService};
use crate::users::{Role, UsersService};
use crate::work_logs::{WorkLog, WorkLogInput, WorkLogsService};

mod attachments;
mod calendar;
mod customers;
mod expenses;
mod invoices;
mod issuer;
mod masters;
mod payments;
mod profitability;
mod projects;
mod sync;
#[cfg(test)]
mod tests;
mod trips;
mod work_logs;

// Theme C PR-C4 (docs/template-scope.md §7 移行順 ④): re-exported so
// `admin_template_core::rest::audited_credential_verifier` - the path
// `bin/banto-serve.rs` and `src-tauri`'s `run()` already use - keeps
// resolving unchanged now that the function itself lives in
// `banto_server::routes::audit`.
pub use banto_server::routes::audited_credential_verifier;

use attachments::attachments_router;
use calendar::calendar_router;
use customers::customers_router;
use expenses::expenses_router;
use invoices::invoices_router;
use issuer::issuer_router;
use masters::masters_router;
use payments::payments_router;
use profitability::profitability_router;
use projects::projects_router;
use sync::sync_router;
use trips::trips_router;
use work_logs::work_logs_router;

/// Slack added on top of `banto_attachments::MAX_ATTACHMENT_BYTES` for
/// [`attachments_write_router`]'s `DefaultBodyLimit` (spec
/// `docs/attachments-plan.md` §3.5): the limit that actually matters is the
/// service-layer check in `AttachmentsService::upload` (which returns a
/// `Validation` error, `422`), not this one - this only needs to be
/// comfortably above `MAX_ATTACHMENT_BYTES` so a request AT the real limit
/// is never rejected by axum's transport-level cap before the service layer
/// even sees it. 1MB of slack is far more than the difference between a
/// file's raw bytes and its (non-existent, this route has no envelope)
/// wire overhead.
const ATTACHMENT_BODY_LIMIT_SLACK_BYTES: usize = 1024 * 1024;

/// The already-cloneable service handles [`api_router`] threads into its
/// route builders (spec §11.1). Bundled into one struct (M-review 2026-08
/// M-13) so adding a service is a single field here rather than a new
/// positional parameter rippled through every `api_router` call site
/// (`bin/banto-serve.rs`, `src-tauri`'s `start_embedded_server`, and the
/// six `rest::tests` router helpers). `scaffold.mjs`'s attachments remover
/// likewise drops one named field instead of a position-dependent argument
/// slot. Every field is an independent handle over the same pool - no
/// ordering or lifetime relationship between them, so the caller may build
/// this in any order.
pub struct Services {
    /// Business ドメイン（Phase 2 基本マスター）。
    pub customers: CustomersService,
    pub projects: ProjectsService,
    /// Business ドメイン（Phase 3 工数・経費）。
    pub masters: MastersService,
    pub work_logs: WorkLogsService,
    pub expenses: ExpensesService,
    pub trips: TripsService,
    /// Business ドメイン（Phase 4 採算管理）。読み取り専用。
    pub profitability: ProfitabilityService,
    /// Business ドメイン（Phase 7 準備：月カレンダー）。読み取り専用。
    pub calendar: CalendarService,
    /// Business ドメイン（Phase 5 請求）。
    pub invoices: InvoicesService,
    pub issuer: IssuerService,
    /// Business ドメイン（Phase 6 入金管理）。
    pub payments: PaymentsService,
    /// Business ドメイン（Phase 8 デバイス間同期）。この段は読み取り専用。
    pub sync: SyncService,
    pub users: UsersService,
    pub settings: SettingsService,
    pub audit: AuditLogService,
    pub backup: BackupService,
    pub attachments: AttachmentsService,
    pub system_info: SystemInfoService,
}

/// Compose the full `/api/*` router (spec §11.1): auth routes (login/
/// logout/check/identity from `banto_server` - wrapped with an audit-log
/// hook for `logout`, spec M14 - plus status/setup/change-password here
/// since those need `UsersService`), SSE events, the Business ドメインの
/// CRUD routes (RBAC-split read/write, spec M10), the `admin`-only `users` management
/// routes (spec M10), the `admin`-only `audit-log` viewer (spec M14), the
/// `admin`-only `backups` routes (spec M17), the `attachments` CRUD routes
/// (RBAC-split read/write, spec `docs/attachments-plan.md` §3.5 M20 unit
/// B), and the per-user `ui-settings` routes (spec M12), all behind the
/// CSRF header check. Mount the result *before*
/// `banto_server::static_files::static_router` so `/api/*` takes priority
/// over the SPA fallback.
pub fn api_router(
    services: Services,
    auth: AuthState,
    events: broadcast::Sender<ServerEvent>,
    allow_setup: bool,
) -> Router {
    let Services {
        customers,
        projects,
        masters,
        work_logs,
        expenses,
        trips,
        profitability,
        calendar,
        invoices,
        issuer,
        payments,
        sync,
        users,
        settings,
        audit,
        backup,
        attachments,
        system_info,
    } = services;

    let audited_auth_routes = auth_routes(auth.clone()).layer(middleware::from_fn_with_state(
        LogoutAuditState {
            auth: auth.clone(),
            audit: audit.clone(),
        },
        audit_logout_middleware,
    ));

    Router::new()
        .merge(audited_auth_routes)
        .merge(extra_auth_router(
            users.clone(),
            auth.clone(),
            audit.clone(),
            allow_setup,
        ))
        .merge(sse_route(auth.clone(), events.clone()))
        .merge(customers_router(customers, audit.clone(), auth.clone()))
        .merge(projects_router(projects, audit.clone(), auth.clone()))
        .merge(masters_router(masters, audit.clone(), auth.clone()))
        .merge(work_logs_router(work_logs, audit.clone(), auth.clone()))
        .merge(expenses_router(
            expenses,
            audit.clone(),
            auth.clone(),
            attachments.clone(),
        ))
        .merge(trips_router(trips, audit.clone(), auth.clone()))
        .merge(profitability_router(profitability, auth.clone()))
        .merge(calendar_router(calendar, auth.clone()))
        .merge(invoices_router(invoices, audit.clone(), auth.clone()))
        .merge(issuer_router(issuer, audit.clone(), auth.clone()))
        .merge(payments_router(payments, audit.clone(), auth.clone()))
        .merge(sync_router(sync, auth.clone()))
        .merge(users_router(users, audit.clone(), auth.clone()))
        .merge(audit_log_router(
            audit.clone(),
            settings.clone(),
            auth.clone(),
        ))
        .merge(backups_router(backup, audit.clone(), auth.clone()))
        .merge(system_info_router(system_info, auth.clone(), audit.clone()))
        .merge(attachments_router(attachments, audit, auth.clone(), events))
        .merge(ui_settings_router(settings, auth))
        .layer(middleware::from_fn(require_banto_client_header))
}
