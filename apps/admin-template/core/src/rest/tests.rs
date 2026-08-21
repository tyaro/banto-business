use super::*;
use crate::calendar::CalendarService;
use crate::customers::CustomersService;
use crate::db::migrate_memory;
use crate::expenses::ExpensesService;
use crate::invoices::InvoicesService;
use crate::issuer::IssuerService;
use crate::masters::MastersService;
use crate::payments::PaymentsService;
use crate::projects::ProjectsService;
use crate::trips::TripsService;
use crate::work_logs::WorkLogsService;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use banto_core::BantoError;
// Theme C PR-C4: `rest/mod.rs` no longer needs `Identity` itself (the helpers
// that used it moved to `banto_server::routes`), so the test module imports it
// directly rather than through `use super::*`.
use banto_server::Identity;
use serde_json::json;
use std::path::PathBuf;
use tempfile::tempdir;
use tower::ServiceExt;

const CLIENT_HEADER: (&str, &str) = ("X-Banto-Client", "banto");

/// A `BackupService` for router helpers that do not exercise
/// `/api/backups/*` at all (the overwhelming majority of this module's
/// tests) - `BackupService::new` only stores its arguments, so an
/// on-disk path that is never actually written to is harmless. Tests
/// that DO exercise backups use [`router_with_role_tokens_and_backup`]
/// instead, which points at a real, writable temp directory AND (unlike
/// every other helper here) a real on-disk pool - see that function's
/// doc comment for why the pool matters too.
fn unused_backup_service(db: banto_storage::Db) -> BackupService {
    BackupService::new(
        PathBuf::from("unused-in-tests").join("admin-template.sqlite3"),
        db,
    )
}

/// An `AttachmentsService` for router helpers that never exercise
/// `/api/attachments/*` - same "never actually written to" reasoning as
/// [`unused_backup_service`]. Tests that DO exercise attachments use
/// [`router_with_role_tokens_and_attachments`] instead, which points at
/// a real, writable temp directory.
fn unused_attachments_service(db: banto_storage::Db) -> AttachmentsService {
    AttachmentsService::new(db, PathBuf::from("unused-in-tests").join("attachments"))
}

fn demo_auth() -> AuthState {
    AuthState::new(|u: String, p: String| {
        Box::pin(async move {
            if u == "admin" && p == "admin" {
                Some(Identity {
                    id: "admin".to_string(),
                    name: "管理者".to_string(),
                    role: "admin".to_string(),
                })
            } else {
                None
            }
        })
    })
}

/// Router + one bearer token per role (admin/editor/viewer), for the
/// RBAC tests below (spec M10). Unlike [`demo_auth_with_roles`] (whose
/// login verifier is independent of any `UsersService`), the three
/// accounts here are REAL rows in the same `UsersService`/pool the
/// router's `/api/users/*` routes operate on - required so
/// `users_delete`'s `acting_user` lookup (by the token's username) can
/// actually resolve the admin account performing the delete in
/// `admin_can_create_list_update_reset_password_and_delete_users`
/// below.
async fn router_with_role_tokens() -> (Router, String, String, String) {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);

    users
        .setup_first_user("admin", "password123", "管理者")
        .await
        .expect("setup_first_user");
    users
        .create_user("editor", "password123", "編集者", Role::Editor)
        .await
        .expect("create editor");
    users
        .create_user("viewer", "password123", "閲覧者", Role::Viewer)
        .await
        .expect("create viewer");

    let verify_users = users.clone();
    let auth = AuthState::new(move |u: String, p: String| {
        let users = verify_users.clone();
        Box::pin(async move {
            match users.verify(&u, &p).await {
                Ok(Some(identity)) => Some(Identity {
                    id: identity.username,
                    name: identity.display_name,
                    role: identity.role.to_string(),
                }),
                _ => None,
            }
        })
    });

    let admin_token = auth
        .login("admin", "password123")
        .await
        .expect("admin login");
    let editor_token = auth
        .login("editor", "password123")
        .await
        .expect("editor login");
    let viewer_token = auth
        .login("viewer", "password123")
        .await
        .expect("viewer login");
    let services = Services {
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
    };
    (
        api_router(services, auth, tx, false),
        admin_token,
        editor_token,
        viewer_token,
    )
}

async fn router_with_token() -> (Router, String) {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);
    let auth = demo_auth();
    let token = auth
        .login("admin", "admin")
        .await
        .expect("login should succeed");
    let services = Services {
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
    };
    (api_router(services, auth, tx, false), token)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn domain_routes_are_guarded_without_token() {
    let (router, _token) = router_with_token().await;
    let response = router
        .oneshot(
            HttpRequest::post("/api/customers/list")
                .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
                .header("content-type", "application/json")
                .body(Body::from(json!(ListParams::default()).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "unauthorized");
}

#[tokio::test]
async fn missing_csrf_header_is_forbidden_even_with_a_token() {
    let (router, token) = router_with_token().await;
    let response = router
        .oneshot(
            HttpRequest::get("/api/auth/check")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_via_rest_is_observable_on_the_event_channel() {
    let pool = migrate_memory().await.expect("migrate_memory");
    let (tx, mut rx) = broadcast::channel(16);
    let customers = CustomersService::new(pool.clone()).with_events(tx.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);
    let auth = demo_auth();
    let token = auth.login("admin", "admin").await.unwrap();
    let services = Services {
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
    };
    let router = api_router(services, auth, tx, false);

    let create_response = router
        .clone()
        .oneshot(
            HttpRequest::post("/api/customers")
                .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(customer_payload("C900").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created = body_json(create_response).await;
    rx.try_recv().expect("create should emit an event");
    let id = created["id"].as_i64().unwrap();

    router
        .oneshot(
            HttpRequest::put(format!("/api/customers/{id}"))
                .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "code": "C900",
                        "name": "架空商事（改称後）",
                        "closingDay": 99,
                        "paymentMonthOffset": 1,
                        "paymentDay": 99
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let event = rx.try_recv().expect("update should emit an event");
    assert!(matches!(event, ServerEvent::ResourceChanged { resource } if resource == "customers"));
}

/// Sanity check that `BantoError` variants used elsewhere still map the
/// way this module's tests assume (guards against silent drift if
/// `banto_core::error` changes).
#[test]
fn error_kind_used_in_tests_matches_banto_core() {
    let err = BantoError::NotFound {
        resource: "customers".to_string(),
        id: "1".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&err).unwrap()["kind"],
        json!("not_found")
    );
}

async fn router_with_setup(allow_setup: bool) -> Router {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);
    let auth = demo_auth();
    let services = Services {
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
    };
    api_router(services, auth, tx, allow_setup)
}

fn get(path: &str) -> HttpRequest<Body> {
    HttpRequest::get(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .body(Body::empty())
        .unwrap()
}

fn post_json(path: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::post(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn auth_status_reports_uninitialized_before_any_setup() {
    let router = router_with_setup(true).await;
    let response = router.oneshot(get("/api/auth/status")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["initialized"], false);
}

#[tokio::test]
async fn auth_status_needs_no_bearer_token() {
    // Same assertion as above, phrased to make explicit that omitting
    // Authorization entirely (not just an invalid token) still gets a
    // 200, not a 401 - the login page calls this before any session
    // exists.
    let router = router_with_setup(true).await;
    let request = HttpRequest::get("/api/auth/status")
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_setup_is_forbidden_when_allow_setup_is_false() {
    let router = router_with_setup(false).await;
    let response = router
        .oneshot(post_json(
            "/api/auth/setup",
            json!({ "username": "owner", "password": "password123", "displayName": "オーナー" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_setup_creates_account_and_the_token_works_for_guarded_routes() {
    let router = router_with_setup(true).await;

    let setup_response = router
        .clone()
        .oneshot(post_json(
            "/api/auth/setup",
            json!({ "username": "owner", "password": "password123", "displayName": "オーナー" }),
        ))
        .await
        .unwrap();
    assert_eq!(setup_response.status(), StatusCode::OK);
    let setup_json = body_json(setup_response).await;
    assert_eq!(setup_json["success"], true);
    let token = setup_json["token"].as_str().expect("token").to_string();

    // `initialized` should now be true.
    let status_response = router
        .clone()
        .oneshot(get("/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(body_json(status_response).await["initialized"], true);

    // And the freshly-issued token should work on a guarded route.
    let list_request = HttpRequest::post("/api/customers/list")
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(json!(ListParams::default()).to_string()))
        .unwrap();
    let list_response = router.oneshot(list_request).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_setup_rejects_short_password_with_422_validation() {
    let router = router_with_setup(true).await;
    let response = router
        .oneshot(post_json(
            "/api/auth/setup",
            json!({ "username": "owner", "password": "short", "displayName": "オーナー" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "validation");
    assert_eq!(json["field_errors"][0]["field"], "password");
}

#[tokio::test]
async fn auth_setup_second_call_returns_success_false_already_initialized() {
    let router = router_with_setup(true).await;
    let first = post_json(
        "/api/auth/setup",
        json!({ "username": "owner", "password": "password123", "displayName": "オーナー" }),
    );
    router.clone().oneshot(first).await.unwrap();

    let second = post_json(
        "/api/auth/setup",
        json!({ "username": "someone-else", "password": "password123", "displayName": "誰か" }),
    );
    let response = router.oneshot(second).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("初期化"));
}

async fn setup_and_get_token(router: &Router) -> String {
    let response = router
        .clone()
        .oneshot(post_json(
            "/api/auth/setup",
            json!({ "username": "owner", "password": "password123", "displayName": "オーナー" }),
        ))
        .await
        .unwrap();
    body_json(response).await["token"]
        .as_str()
        .expect("token")
        .to_string()
}

#[tokio::test]
async fn auth_change_password_requires_a_bearer_token() {
    let router = router_with_setup(true).await;
    setup_and_get_token(&router).await;

    let response = router
        .oneshot(post_json(
            "/api/auth/change-password",
            json!({ "currentPassword": "password123", "newPassword": "newpassword1" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_change_password_rejects_wrong_current_password() {
    let router = router_with_setup(true).await;
    let token = setup_and_get_token(&router).await;

    let request = HttpRequest::post("/api/auth/change-password")
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "currentPassword": "not-the-password", "newPassword": "newpassword1" })
                .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response).await;
    assert_eq!(json["field_errors"][0]["field"], "currentPassword");
}

/// Builds a router whose `/api/auth/login` verifier is backed by the
/// SAME `UsersService`/pool as `/api/auth/setup` and
/// `/api/auth/change-password` - mirrors how `banto-serve`/`src-tauri`
/// wire things in production (unlike `router_with_setup` above, whose
/// `demo_auth()` login verifier is intentionally independent, matching
/// the other tests in this module that only care about RBAC/CSRF
/// behavior). Also returns the `AuditLogService` sharing the router's
/// pool, so M14 tests can assert on what got recorded.
async fn router_with_real_login(allow_setup: bool) -> (Router, AuditLogService) {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);
    let auth = AuthState::new(audited_credential_verifier(users.clone(), audit.clone()));
    let services = Services {
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
        audit: audit.clone(),
        backup,
        attachments,
        system_info,
    };
    (api_router(services, auth, tx, allow_setup), audit)
}

#[tokio::test]
async fn auth_change_password_success_then_relogin_with_new_password() {
    let (router, _audit) = router_with_real_login(true).await;
    let token = setup_and_get_token(&router).await;

    let change_request = HttpRequest::post("/api/auth/change-password")
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "currentPassword": "password123", "newPassword": "newpassword1" }).to_string(),
        ))
        .unwrap();
    let change_response = router.clone().oneshot(change_request).await.unwrap();
    assert_eq!(change_response.status(), StatusCode::OK);
    assert_eq!(body_json(change_response).await["success"], true);

    // The old password must no longer work.
    let old_login = router
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            json!({ "username": "owner", "password": "password123" }),
        ))
        .await
        .unwrap();
    assert_eq!(body_json(old_login).await["success"], false);

    // The new password must work.
    let new_login = router
        .oneshot(post_json(
            "/api/auth/login",
            json!({ "username": "owner", "password": "newpassword1" }),
        ))
        .await
        .unwrap();
    let json = body_json(new_login).await;
    assert_eq!(json["success"], true);
    assert!(json["token"].as_str().is_some());
}

// --- M10 RBAC ----------------------------------------------------------

fn put_json(path: &str, token: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::put(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_json_auth(path: &str, token: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::post(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// PUT + JSON + 認証。`post_json_auth` の PUT 版（cost-rates の upsert や
/// 各リソースの更新で使う）。
fn put_json_auth(path: &str, token: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::put(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_auth(path: &str, token: &str) -> HttpRequest<Body> {
    HttpRequest::get(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn delete_auth(path: &str, token: &str) -> HttpRequest<Body> {
    HttpRequest::delete(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn only_admin_can_list_users() {
    let (router, admin, editor, viewer) = router_with_role_tokens().await;

    for (token, expected) in [
        (&admin, StatusCode::OK),
        (&editor, StatusCode::FORBIDDEN),
        (&viewer, StatusCode::FORBIDDEN),
    ] {
        let response = router
            .clone()
            .oneshot(get_auth("/api/users", token))
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "token role mismatch");
    }
}

#[tokio::test]
async fn non_admin_users_write_routes_are_forbidden_with_forbidden_kind() {
    let (router, _admin, editor, _viewer) = router_with_role_tokens().await;

    let response = router
        .oneshot(post_json_auth(
            "/api/users",
            &editor,
            json!({
                "username": "newperson",
                "password": "password123",
                "displayName": "New Person",
                "role": "viewer"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "forbidden");
}

#[tokio::test]
async fn admin_can_create_list_update_reset_password_and_delete_users() {
    let (router, admin, _editor, _viewer) = router_with_role_tokens().await;

    let create_response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/users",
            &admin,
            json!({
                "username": "newperson",
                "password": "password123",
                "displayName": "New Person",
                "role": "editor"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let created = body_json(create_response).await;
    assert_eq!(created["role"], "editor");
    let id = created["id"].as_i64().unwrap();

    let list_response = router
        .clone()
        .oneshot(get_auth("/api/users", &admin))
        .await
        .unwrap();
    let list = body_json(list_response).await;
    assert!(list.as_array().unwrap().iter().any(|u| u["id"] == id));

    let update_response = router
        .clone()
        .oneshot(put_json(
            &format!("/api/users/{id}"),
            &admin,
            json!({ "displayName": "Updated Person", "role": "viewer" }),
        ))
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);
    assert_eq!(body_json(update_response).await["role"], "viewer");

    let reset_response = router
        .clone()
        .oneshot(post_json_auth(
            &format!("/api/users/{id}/reset-password"),
            &admin,
            json!({ "newPassword": "resetpassword1" }),
        ))
        .await
        .unwrap();
    assert_eq!(reset_response.status(), StatusCode::OK);
    assert_eq!(body_json(reset_response).await["success"], true);

    let delete_response = router
        .oneshot(delete_auth(&format!("/api/users/{id}"), &admin))
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn users_routes_are_unauthorized_without_a_token() {
    let (router, _admin, _editor, _viewer) = router_with_role_tokens().await;
    let response = router.oneshot(get("/api/users")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// --- M12 per-user UI settings ------------------------------------------

fn put_ui_setting(key: &str, token: &str, value: &str) -> HttpRequest<Body> {
    put_json(
        &format!("/api/ui-settings/{key}"),
        token,
        json!({ "value": value }),
    )
}

#[tokio::test]
async fn ui_settings_round_trip_via_rest() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    // Unset key reads back as {"value": null}.
    let response = router
        .clone()
        .oneshot(get_auth("/api/ui-settings/theme", &viewer))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_json(response).await["value"].is_null());

    // PUT then GET round-trips - and note this is the VIEWER role:
    // writing your own UI settings needs no role floor (unlike
    // `settings_set`/`/api/users`).
    let put_response = router
        .clone()
        .oneshot(put_ui_setting("theme", &viewer, "glass"))
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::NO_CONTENT);

    let response = router
        .oneshot(get_auth("/api/ui-settings/theme", &viewer))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["value"], "glass");
}

#[tokio::test]
async fn ui_settings_are_isolated_per_user() {
    let (router, admin, editor, _viewer) = router_with_role_tokens().await;

    let put_response = router
        .clone()
        .oneshot(put_ui_setting("theme", &admin, "glass"))
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::NO_CONTENT);

    // The admin's value must NOT be visible to the editor's session -
    // each account reads its own `ui.{username}.*` namespace.
    let response = router
        .clone()
        .oneshot(get_auth("/api/ui-settings/theme", &editor))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_json(response).await["value"].is_null());

    // And the admin still sees their own value.
    let response = router
        .oneshot(get_auth("/api/ui-settings/theme", &admin))
        .await
        .unwrap();
    assert_eq!(body_json(response).await["value"], "glass");
}

#[tokio::test]
async fn ui_settings_reject_an_invalid_key_with_422_validation() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    // `%20` decodes to a space in the path param - an invalid key char.
    let response = router
        .clone()
        .oneshot(put_ui_setting("bad%20key!", &viewer, "x"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "validation");
    assert_eq!(json["field_errors"][0]["field"], "key");

    let response = router
        .oneshot(get_auth("/api/ui-settings/bad%20key!", &viewer))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn ui_settings_routes_are_unauthorized_without_a_token() {
    let (router, _admin, _editor, _viewer) = router_with_role_tokens().await;

    let response = router
        .clone()
        .oneshot(get("/api/ui-settings/theme"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = router
        .oneshot(post_json(
            "/api/ui-settings/theme",
            json!({ "value": "glass" }),
        ))
        .await
        .unwrap();
    // POST is not a registered method on this route, but the request
    // must still die at `require_auth` (401), not reach any handler.
    assert!(
        response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::METHOD_NOT_ALLOWED
    );
}

// --- M14 Audit -----------------------------------------------------------

/// Like `router_with_role_tokens`, but also returns the `AuditLogService`
/// sharing the router's pool (so these tests can query
/// `/api/audit-log/list` as the admin token and assert on what got
/// recorded), and wires the login verifier through
/// [`audited_credential_verifier`] so login events are actually recorded
/// - `router_with_role_tokens`'s own verifier predates M14 and stays a
///   plain credential check since none of ITS callers care about audit
///   events.
async fn router_with_role_tokens_and_audit() -> (Router, AuditLogService, String, String, String) {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let attachments = unused_attachments_service(pool.clone());
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);

    users
        .setup_first_user("admin", "password123", "管理者")
        .await
        .expect("setup_first_user");
    users
        .create_user("editor", "password123", "編集者", Role::Editor)
        .await
        .expect("create editor");
    users
        .create_user("viewer", "password123", "閲覧者", Role::Viewer)
        .await
        .expect("create viewer");

    let auth = AuthState::new(audited_credential_verifier(users.clone(), audit.clone()));
    let admin_token = auth
        .login("admin", "password123")
        .await
        .expect("admin login");
    let editor_token = auth
        .login("editor", "password123")
        .await
        .expect("editor login");
    let viewer_token = auth
        .login("viewer", "password123")
        .await
        .expect("viewer login");

    let services = Services {
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
        audit: audit.clone(),
        backup,
        attachments,
        system_info,
    };
    let router = api_router(services, auth, tx, false);
    (router, audit, admin_token, editor_token, viewer_token)
}

/// Like `router_with_role_tokens_and_audit`, but for the M17
/// `/api/backups/*` (and, since both need a real writable temp
/// directory, M20 `/api/attachments/*`) tests, which need services that
/// ACTUALLY WORK end to end (create/list/read/stage a real file), not
/// [`unused_backup_service`]/[`unused_attachments_service`]'s
/// placeholders. Two things every other helper in this module gets to
/// skip:
/// - The router's own pool must be a real ON-DISK sqlite file, not
///   `:memory:` (`migrate_memory()`) - `VACUUM INTO` (which
///   `BackupService::create` uses) silently writes nothing when its
///   SOURCE connection is `:memory:` (see `crate::backup`'s test module
///   doc comment for the empirically-verified reason).
/// - The returned `tempfile::TempDir` guard must be kept alive by the
///   caller for as long as the router is in use - dropping it deletes
///   the directory `backups/`/`restore-pending.sqlite3`/`attachments/`
///   live in.
async fn router_with_role_tokens_and_backup() -> (Router, tempfile::TempDir, String, String, String)
{
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("admin-template.sqlite3");
    let pool = banto_storage::connect_sqlite(&db_path)
        .await
        .expect("connect_sqlite");
    sqlx::migrate!("./migrations-sqlite")
        .run(&pool)
        .await
        .expect("migrate");
    // V2 PR2: services take a backend-agnostic `Db`; wrap the on-disk SQLite
    // pool (a real file is required here so `VACUUM INTO` actually writes).
    let db = banto_storage::Db::Sqlite(pool);
    let customers = CustomersService::new(db.clone());
    let projects = ProjectsService::new(db.clone());
    let masters = MastersService::new(db.clone());
    let work_logs = WorkLogsService::new(db.clone());
    let expenses = ExpensesService::new(db.clone());
    let trips = TripsService::new(db.clone());
    let profitability = ProfitabilityService::new(db.clone());
    let calendar = CalendarService::new(db.clone());
    let invoices = InvoicesService::new(db.clone());
    let payments = PaymentsService::new(db.clone());
    let issuer = IssuerService::new(SettingsService::new(db.clone()));

    let (tx, _rx) = broadcast::channel(16);
    let users = UsersService::new(db.clone());
    let settings = SettingsService::new(db.clone());
    let sync = SyncService::new(db.clone(), settings.clone());
    let backup = BackupService::new(db_path, db.clone());
    let attachments = AttachmentsService::new(db.clone(), dir.path().join("attachments"));
    let system_info = SystemInfoService::new(db.clone());
    let audit = AuditLogService::new(db);

    users
        .setup_first_user("admin", "password123", "管理者")
        .await
        .expect("setup_first_user");
    users
        .create_user("editor", "password123", "編集者", Role::Editor)
        .await
        .expect("create editor");
    users
        .create_user("viewer", "password123", "閲覧者", Role::Viewer)
        .await
        .expect("create viewer");

    let auth = AuthState::new(audited_credential_verifier(users.clone(), audit.clone()));
    let admin_token = auth
        .login("admin", "password123")
        .await
        .expect("admin login");
    let editor_token = auth
        .login("editor", "password123")
        .await
        .expect("editor login");
    let viewer_token = auth
        .login("viewer", "password123")
        .await
        .expect("viewer login");

    let services = Services {
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
    };
    let router = api_router(services, auth, tx, false);
    (router, dir, admin_token, editor_token, viewer_token)
}

/// (a) `/api/audit-log/list` is admin-only: 200 for admin, 403 for
/// editor/viewer.
#[tokio::test]
async fn audit_log_list_is_admin_only() {
    let (router, _audit, admin, editor, viewer) = router_with_role_tokens_and_audit().await;

    let admin_response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);

    for token in [&editor, &viewer] {
        let response = router
            .clone()
            .oneshot(post_json_auth(
                "/api/audit-log/list",
                token,
                json!(ListParams::default()),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "token role mismatch"
        );
    }

    // The list denial keeps `resource: "audit_log"` - it reads the audit log
    // body itself, unlike the config routes whose denial is "settings" (the
    // two-guard split in `audit_log_router`, maintenance-review-2026-08 §5.3
    // H-4). Pinning both sides here guards against the split regressing into a
    // single shared resource tag again.
    let list_rows = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(list_rows).await["rows"].clone();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|r| r["action"] == "denied" && r["resource"] == "audit_log"),
        "expected a denied/audit_log entry, got {rows:?}"
    );
}

#[tokio::test]
async fn audit_log_list_requires_a_token() {
    let (router, _audit, _admin, _editor, _viewer) = router_with_role_tokens_and_audit().await;
    let response = router
        .oneshot(post_json(
            "/api/audit-log/list",
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// `GET /api/audit-log/config` is admin-only: 200 (with the default
/// retention policy) for admin, 403 for editor/viewer.
#[tokio::test]
async fn audit_config_get_is_admin_only() {
    let (router, _audit, admin, editor, viewer) = router_with_role_tokens_and_audit().await;

    let admin_response = router
        .clone()
        .oneshot(get_auth("/api/audit-log/config", &admin))
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);
    let body = body_json(admin_response).await;
    assert_eq!(body["retentionDays"], 90);
    assert_eq!(body["retentionRows"], 100_000);

    for token in [&editor, &viewer] {
        let response = router
            .clone()
            .oneshot(get_auth("/api/audit-log/config", token))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "token role mismatch"
        );
    }
}

/// `GET /api/system/info` (M-review 2026-08 §2.4) is admin-only and needs a
/// token: 200 with the diagnostics payload for admin, 403 for editor/viewer,
/// 401 for no token. Read-only, so nothing is audited.
#[tokio::test]
async fn system_info_is_admin_only() {
    let (router, admin, editor, viewer) = router_with_role_tokens().await;

    let admin_response = router
        .clone()
        .oneshot(get_auth("/api/system/info", &admin))
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);
    let body = body_json(admin_response).await;
    // The migrated in-memory DB reports the sqlite dialect and a version.
    assert_eq!(body["dbDialect"], "sqlite");
    assert!(body["appVersion"].is_string());
    assert!(body["migrationVersion"].is_number());
    assert!(body["activeSessions"].as_u64().unwrap() >= 3); // admin+editor+viewer logged in

    for token in [&editor, &viewer] {
        let response = router
            .clone()
            .oneshot(get_auth("/api/system/info", token))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "system info is admin-only"
        );
    }
}

#[tokio::test]
async fn system_info_requires_a_token() {
    let (router, _admin, _editor, _viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(get_auth("/api/system/info", "not-a-real-token"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// `PUT /api/audit-log/config` (admin) persists the new policy - a
/// following `GET` reflects it - and records a `settings_change` audit
/// entry (spec M14: settings mutations are audited, unlike the read-only
/// `GET`). `editor`/`viewer` are rejected with 403 and the policy is left
/// untouched.
#[tokio::test]
async fn audit_config_apply_persists_and_is_admin_only() {
    let (router, _audit, admin, editor, viewer) = router_with_role_tokens_and_audit().await;

    for token in [&editor, &viewer] {
        let response = router
            .clone()
            .oneshot(put_json(
                "/api/audit-log/config",
                token,
                json!({ "retentionDays": 30, "retentionRows": 5000 }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "token role mismatch"
        );
    }

    let apply_response = router
        .clone()
        .oneshot(put_json(
            "/api/audit-log/config",
            &admin,
            json!({ "retentionDays": 30, "retentionRows": 5000 }),
        ))
        .await
        .unwrap();
    assert_eq!(apply_response.status(), StatusCode::OK);
    let applied = body_json(apply_response).await;
    assert_eq!(applied["retentionDays"], 30);
    assert_eq!(applied["retentionRows"], 5000);

    let get_response = router
        .clone()
        .oneshot(get_auth("/api/audit-log/config", &admin))
        .await
        .unwrap();
    let refetched = body_json(get_response).await;
    assert_eq!(refetched["retentionDays"], 30);
    assert_eq!(refetched["retentionRows"], 5000);

    let list_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(list_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();
    let entry = rows
        .iter()
        .find(|r| r["action"] == "settings_change" && r["resource"] == "settings")
        .unwrap_or_else(|| panic!("expected a settings_change/settings entry, got {rows:?}"));
    assert_eq!(entry["actorUsername"], "admin");
    assert_eq!(entry["origin"], "rest");
    assert_eq!(entry["result"], "ok");

    // The editor/viewer denials above are tagged `resource: "settings"` (NOT
    // "audit_log"): the config routes read/write the audit *settings*, so
    // their denial matches the `settings_change` success entry and the Tauri
    // twin (`audit_config_apply` calls `require_role(.., "settings")`), keeping
    // denied/success filterable under one resource. `audit-log/list` denials
    // keep "audit_log" - see `audit_log_list_is_admin_only`
    // (maintenance-review-2026-08 §5.3 H-4).
    let denial = rows
        .iter()
        .find(|r| r["action"] == "denied" && r["resource"] == "settings")
        .unwrap_or_else(|| panic!("expected a denied/settings entry, got {rows:?}"));
    assert_eq!(denial["origin"], "rest");
    assert_eq!(denial["result"], "denied");
}

/// A successful delete is recorded too (not just create) - a quick
/// sanity check that every mutation, not just the first one wired up, is
/// covered.
#[tokio::test]
async fn customer_delete_is_recorded_in_the_audit_log() {
    let (router, _audit, admin, _editor, _viewer) = router_with_role_tokens_and_audit().await;

    let create_response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/customers",
            &admin,
            customer_payload("C902"),
        ))
        .await
        .unwrap();
    let id = body_json(create_response).await["id"].as_i64().unwrap();

    router
        .clone()
        .oneshot(delete_auth(&format!("/api/customers/{id}"), &admin))
        .await
        .unwrap();

    let list_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(list_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();
    assert!(
        rows.iter().any(|r| r["action"] == "delete"
            && r["resource"] == "customers"
            && r["entityId"] == id.to_string().as_str()),
        "expected a delete/customers entry, got {rows:?}"
    );
}

/// (c) A viewer's rejected write is recorded as `denied`.
#[tokio::test]
async fn viewer_write_denial_is_recorded_as_denied() {
    let (router, _audit, admin, _editor, viewer) = router_with_role_tokens_and_audit().await;

    let response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/customers",
            &viewer,
            customer_payload("C901"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let list_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(list_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();
    let entry = rows
        .iter()
        .find(|r| r["action"] == "denied" && r["resource"] == "customers")
        .unwrap_or_else(|| panic!("expected a denied/customers entry, got {rows:?}"));
    assert_eq!(entry["actorUsername"], "viewer");
    assert_eq!(entry["actorRole"], "viewer");
    assert_eq!(entry["result"], "denied");
}

/// `users` create/reset-password entries must never leak the plaintext
/// password into `detail` (spec M14's hard rule - see
/// `crate::audit`'s module doc comment).
#[tokio::test]
async fn users_create_audit_entry_never_contains_the_password() {
    let (router, _audit, admin, _editor, _viewer) = router_with_role_tokens_and_audit().await;

    router
        .clone()
        .oneshot(post_json_auth(
            "/api/users",
            &admin,
            json!({
                "username": "newperson",
                "password": "supersecret1",
                "displayName": "New Person",
                "role": "viewer"
            }),
        ))
        .await
        .unwrap();

    let list_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(list_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();
    let entry = rows
        .iter()
        .find(|r| r["action"] == "create" && r["resource"] == "users")
        .expect("expected a create/users entry");
    assert_eq!(entry["actorUsername"], "admin");
    let detail = entry["detail"].as_str().expect("detail should be set");
    assert!(
        !detail.contains("supersecret1"),
        "audit detail must never contain the password: {detail}"
    );
    assert!(detail.contains("newperson"));
}

/// (d) A failed login attempt is recorded as `login_failed`. Uses
/// `router_with_real_login` (not `router_with_role_tokens_and_audit`)
/// since it wires `/api/auth/login` through the same
/// `audited_credential_verifier` production code path.
#[tokio::test]
async fn login_failure_is_recorded_as_login_failed() {
    let (router, audit) = router_with_real_login(true).await;
    setup_and_get_token(&router).await; // creates the "owner" admin account

    let response = router
        .oneshot(post_json(
            "/api/auth/login",
            json!({ "username": "owner", "password": "wrong-password" }),
        ))
        .await
        .unwrap();
    assert_eq!(body_json(response).await["success"], false);

    let result = audit.list(ListParams::default()).await.unwrap();
    let entry = result
        .rows
        .iter()
        .find(|r| r.action == "login_failed")
        .unwrap_or_else(|| panic!("expected a login_failed entry, got {:?}", result.rows));
    assert_eq!(entry.actor_username.as_deref(), Some("owner"));
    assert_eq!(entry.actor_role, None);
    assert_eq!(entry.result, "failed");
}

#[tokio::test]
async fn login_success_is_recorded_as_login() {
    let (router, audit) = router_with_real_login(true).await;
    setup_and_get_token(&router).await;

    router
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            json!({ "username": "owner", "password": "password123" }),
        ))
        .await
        .unwrap();

    let result = audit.list(ListParams::default()).await.unwrap();
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.action == "login" && r.actor_username.as_deref() == Some("owner")),
        "expected a login entry, got {:?}",
        result.rows
    );
}

#[tokio::test]
async fn logout_is_recorded() {
    let (router, audit) = router_with_real_login(true).await;
    let token = setup_and_get_token(&router).await;

    router
        .oneshot(
            HttpRequest::post("/api/auth/logout")
                .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let result = audit.list(ListParams::default()).await.unwrap();
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.action == "logout" && r.actor_username.as_deref() == Some("owner")),
        "expected a logout entry, got {:?}",
        result.rows
    );
}

#[tokio::test]
async fn setup_is_recorded() {
    let (router, audit) = router_with_real_login(true).await;
    setup_and_get_token(&router).await;

    let result = audit.list(ListParams::default()).await.unwrap();
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.action == "setup" && r.actor_username.as_deref() == Some("owner")),
        "expected a setup entry, got {:?}",
        result.rows
    );
}

/// Spec M14 (coordinator review): a self-service password change is a
/// security event and must be recorded as `password_change` (actor =
/// entity = the caller) - and the entry must never carry the password.
#[tokio::test]
async fn change_password_is_recorded_as_password_change() {
    let (router, audit) = router_with_real_login(true).await;
    let token = setup_and_get_token(&router).await;

    let change_request = HttpRequest::post("/api/auth/change-password")
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "currentPassword": "password123", "newPassword": "newpassword1" }).to_string(),
        ))
        .unwrap();
    let response = router.oneshot(change_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let result = audit.list(ListParams::default()).await.unwrap();
    let entry = result
        .rows
        .iter()
        .find(|r| r.action == "password_change")
        .unwrap_or_else(|| panic!("expected a password_change entry, got {:?}", result.rows));
    assert_eq!(entry.actor_username.as_deref(), Some("owner"));
    assert_eq!(entry.actor_role.as_deref(), Some("admin"));
    assert_eq!(entry.resource, "users");
    // `setup_first_user` creates the very first row -> id 1.
    assert_eq!(entry.entity_id.as_deref(), Some("1"));
    assert_eq!(entry.origin, "rest");
    assert_eq!(entry.result, "ok");
    assert_eq!(entry.detail, None, "detail must never carry the password");
}

// --- M15: CSV import -----------------------------------------------------

// --- M17: SQLite backup/restore -------------------------------------------

fn post_bytes_auth(path: &str, token: &str, bytes: Vec<u8>) -> HttpRequest<Body> {
    HttpRequest::post(path)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap()
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

/// admin can create a backup, see it in the list, and download the exact
/// same bytes back (spec M17: "バックアップファイルが作成・ダウンロード
/// でき"). `POST /api/backups` is recorded as `action: "backup"`.
#[tokio::test]
async fn admin_can_create_list_and_download_backups() {
    let (router, _dir, admin, _editor, _viewer) = router_with_role_tokens_and_backup().await;

    let create_response = router
        .clone()
        .oneshot(post_bytes_auth("/api/backups", &admin, Vec::new()))
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let created = body_json(create_response).await;
    let file_name = created["fileName"].as_str().expect("fileName").to_string();
    assert!(created["sizeBytes"].as_u64().unwrap() > 0);

    let list_response = router
        .clone()
        .oneshot(get_auth("/api/backups", &admin))
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let listed = body_json(list_response).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["fileName"], file_name);

    let download_response = router
        .oneshot(get_auth(&format!("/api/backups/{file_name}"), &admin))
        .await
        .unwrap();
    assert_eq!(download_response.status(), StatusCode::OK);
    let disposition = download_response
        .headers()
        .get(axum::http::header::CONTENT_DISPOSITION)
        .expect("Content-Disposition header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("attachment"));
    assert!(disposition.contains(&file_name));
    let bytes = body_bytes(download_response).await;
    assert_eq!(&bytes[0..16], b"SQLite format 3\0");
}

/// `editor`/`viewer` cannot reach ANY `/api/backups/*` route (spec M17:
/// "admin以外は全API 403") - checked against both a read route (`GET
/// /api/backups`) and a write route (`POST /api/backups`).
#[tokio::test]
async fn editor_and_viewer_cannot_access_backups_routes() {
    let (router, _dir, _admin, editor, viewer) = router_with_role_tokens_and_backup().await;

    for token in [&editor, &viewer] {
        let list_response = router
            .clone()
            .oneshot(get_auth("/api/backups", token))
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::FORBIDDEN);
        let json = body_json(list_response).await;
        assert_eq!(json["kind"], "forbidden");

        let create_response = router
            .clone()
            .oneshot(post_bytes_auth("/api/backups", token, Vec::new()))
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::FORBIDDEN);
    }
}

/// Uploading garbage bytes to `/api/backups/restore` must be rejected
/// (spec M17: "壊れたファイルのリストアが検証で拒否される") - `Validation`
/// maps to `422` (`banto_server::response::status_for`), and no pending
/// restore is left staged.
#[tokio::test]
async fn restore_upload_of_garbage_bytes_is_rejected_as_validation() {
    let (router, _dir, admin, _editor, _viewer) = router_with_role_tokens_and_backup().await;

    let response = router
        .clone()
        .oneshot(post_bytes_auth(
            "/api/backups/restore",
            &admin,
            b"not a sqlite file".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "validation");

    let pending_response = router
        .oneshot(get_auth("/api/backups/pending-restore", &admin))
        .await
        .unwrap();
    assert_eq!(body_json(pending_response).await, serde_json::Value::Null);
}

/// Full stage-from-existing-backup -> cancel round trip (spec M17),
/// asserting both the `pending-restore` status endpoint AND the
/// `restore_staged`/`restore_cancelled` audit entries it records.
#[tokio::test]
async fn stage_restore_from_existing_backup_then_cancel_is_recorded_in_the_audit_log() {
    let (router, _dir, admin, _editor, _viewer) = router_with_role_tokens_and_backup().await;

    let create_response = router
        .clone()
        .oneshot(post_bytes_auth("/api/backups", &admin, Vec::new()))
        .await
        .unwrap();
    let file_name = body_json(create_response).await["fileName"]
        .as_str()
        .unwrap()
        .to_string();

    let stage_response = router
        .clone()
        .oneshot(post_bytes_auth(
            &format!("/api/backups/{file_name}/restore"),
            &admin,
            Vec::new(),
        ))
        .await
        .unwrap();
    assert_eq!(stage_response.status(), StatusCode::NO_CONTENT);

    let pending_response = router
        .clone()
        .oneshot(get_auth("/api/backups/pending-restore", &admin))
        .await
        .unwrap();
    let pending = body_json(pending_response).await;
    assert!(pending["sizeBytes"].as_u64().unwrap() > 0);

    let cancel_response = router
        .clone()
        .oneshot(delete_auth("/api/backups/pending-restore", &admin))
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), StatusCode::NO_CONTENT);

    let pending_after_cancel = router
        .clone()
        .oneshot(get_auth("/api/backups/pending-restore", &admin))
        .await
        .unwrap();
    assert_eq!(
        body_json(pending_after_cancel).await,
        serde_json::Value::Null
    );

    let audit_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(audit_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["action"] == "backup" && r["resource"] == "backups"),
        "expected a backup entry, got {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r["action"] == "restore_staged" && r["resource"] == "backups"),
        "expected a restore_staged entry, got {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r["action"] == "restore_cancelled" && r["resource"] == "backups"),
        "expected a restore_cancelled entry, got {rows:?}"
    );
}

// --- M20: attachments -------------------------------------------------------

/// Full upload -> list -> download -> thumbnail(404, non-image) -> delete
/// round trip (spec `docs/attachments-plan.md` §3.5/§5): `editor` writes,
/// `viewer` reads. Also checks the `Content-Disposition` header carries
/// both the ASCII `filename=` and RFC 5987 `filename*=` forms.
#[tokio::test]
async fn editor_can_upload_list_download_and_delete_an_attachment() {
    let (router, _dir, _admin, editor, viewer) = router_with_role_tokens_and_backup().await;
    let bytes = b"hello attachment".to_vec();

    let upload_response = router
        .clone()
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=42&fileName=notes.txt",
            &editor,
            bytes.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(upload_response.status(), StatusCode::OK);
    let created = body_json(upload_response).await;
    assert_eq!(created["resource"], "customers");
    assert_eq!(created["resourceId"], "42");
    assert_eq!(created["fileName"], "notes.txt");
    assert_eq!(created["mime"], "application/octet-stream");
    assert_eq!(created["sizeBytes"].as_u64().unwrap() as usize, bytes.len());
    assert_eq!(created["hasThumbnail"], false);
    assert_eq!(created["createdBy"], "editor");
    let id = created["id"].as_i64().unwrap();

    let list_response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/attachments/list",
            &viewer,
            json!({ "resource": "customers", "resourceId": "42" }),
        ))
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let listed = body_json(list_response).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id);

    let download_response = router
        .clone()
        .oneshot(get_auth(
            &format!("/api/attachments/{id}/download"),
            &viewer,
        ))
        .await
        .unwrap();
    assert_eq!(download_response.status(), StatusCode::OK);
    let disposition = download_response
        .headers()
        .get(axum::http::header::CONTENT_DISPOSITION)
        .expect("Content-Disposition header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("attachment"));
    assert!(disposition.contains("filename=\"notes.txt\""));
    assert!(disposition.contains("filename*=UTF-8''notes.txt"));
    let downloaded = body_bytes(download_response).await;
    assert_eq!(downloaded, bytes);

    // Non-image upload: no thumbnail generated, so this 404s (spec §3.5).
    let thumbnail_response = router
        .clone()
        .oneshot(get_auth(
            &format!("/api/attachments/{id}/thumbnail"),
            &viewer,
        ))
        .await
        .unwrap();
    assert_eq!(thumbnail_response.status(), StatusCode::NOT_FOUND);

    let delete_response = router
        .clone()
        .oneshot(delete_auth(&format!("/api/attachments/{id}"), &editor))
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let list_after_delete = router
        .oneshot(post_json_auth(
            "/api/attachments/list",
            &viewer,
            json!({ "resource": "customers", "resourceId": "42" }),
        ))
        .await
        .unwrap();
    let listed_after = body_json(list_after_delete).await;
    assert_eq!(listed_after.as_array().unwrap().len(), 0);
}

/// `viewer` cannot upload or delete attachments (spec §3.5: `editor`+
/// write floor) - both are rejected `403` with `{"kind":"forbidden"}`,
/// same shape as every other RBAC-guarded write route in this module.
#[tokio::test]
async fn viewer_cannot_upload_or_delete_attachments_forbidden_with_forbidden_kind() {
    let (router, _dir, _admin, _editor, viewer) = router_with_role_tokens_and_backup().await;

    let upload_response = router
        .clone()
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=1&fileName=a.txt",
            &viewer,
            b"x".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(upload_response.status(), StatusCode::FORBIDDEN);
    let json = body_json(upload_response).await;
    assert_eq!(json["kind"], "forbidden");

    let delete_response = router
        .oneshot(delete_auth("/api/attachments/1", &viewer))
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);
}

/// `POST /api/attachments/list` needs a bearer token, same as every
/// other `require_auth`-guarded route (spec §3.5: `viewer`+, but
/// AUTHENTICATED viewer+, not anonymous).
#[tokio::test]
async fn attachments_list_route_requires_a_token() {
    let (router, _dir, _admin, _editor, _viewer) = router_with_role_tokens_and_backup().await;
    let response = router
        .oneshot(post_json(
            "/api/attachments/list",
            json!({ "resource": "customers", "resourceId": "1" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Downloading/thumbnailing an id that does not exist is a plain `404`
/// (spec §3.5), same `NotFound` -> `404` mapping every other resource
/// uses (`banto_server::response::status_for`).
#[tokio::test]
async fn nonexistent_attachment_download_and_thumbnail_are_404() {
    let (router, _dir, _admin, _editor, viewer) = router_with_role_tokens_and_backup().await;

    let download_response = router
        .clone()
        .oneshot(get_auth("/api/attachments/999/download", &viewer))
        .await
        .unwrap();
    assert_eq!(download_response.status(), StatusCode::NOT_FOUND);

    let thumbnail_response = router
        .oneshot(get_auth("/api/attachments/999/thumbnail", &viewer))
        .await
        .unwrap();
    assert_eq!(thumbnail_response.status(), StatusCode::NOT_FOUND);
}

/// A body over `MAX_ATTACHMENT_BYTES` but still under the router's
/// `DefaultBodyLimit` (spec §7: 25MB cap, one constant) reaches
/// `AttachmentsService::upload`'s own size check and is rejected as a
/// `422` `Validation` error - the same "service-layer limit, not just a
/// transport-layer one" shape `banto_attachments`'s own crate tests
/// exercise directly (`upload_rejects_bytes_over_the_max_size`).
#[tokio::test]
async fn oversized_attachment_upload_is_rejected_as_validation() {
    let (router, _dir, _admin, editor, _viewer) = router_with_role_tokens_and_backup().await;
    let bytes = vec![0u8; MAX_ATTACHMENT_BYTES + 1];

    let response = router
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=1&fileName=huge.bin",
            &editor,
            bytes,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response).await;
    assert_eq!(json["kind"], "validation");
}

/// A body beyond even the router's `DefaultBodyLimit` (spec §3.5: cap +
/// [`ATTACHMENT_BODY_LIMIT_SLACK_BYTES`] slack) never reaches the
/// handler at all - axum itself rejects it with `413 Payload Too Large`,
/// the transport-layer counterpart to the service-layer `422` above.
#[tokio::test]
async fn attachment_upload_beyond_the_body_limit_is_rejected_with_413() {
    let (router, _dir, _admin, editor, _viewer) = router_with_role_tokens_and_backup().await;
    let bytes = vec![0u8; MAX_ATTACHMENT_BYTES + ATTACHMENT_BODY_LIMIT_SLACK_BYTES + 1];

    let response = router
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=1&fileName=huge.bin",
            &editor,
            bytes,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// Upload/delete each record exactly one audit entry (spec §3.5:
/// `action: "create"`/`"delete"`, `resource: "attachments"`, detail
/// `{fileName,sizeBytes,parentResource,parentId}`) - same "once the
/// service call has already succeeded" convention as the domain routes/`backups`.
#[tokio::test]
async fn attachment_upload_and_delete_are_recorded_in_the_audit_log() {
    let (router, _dir, admin, editor, _viewer) = router_with_role_tokens_and_backup().await;

    let upload_response = router
        .clone()
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=7&fileName=photo.bin",
            &editor,
            b"binary".to_vec(),
        ))
        .await
        .unwrap();
    let id = body_json(upload_response).await["id"].as_i64().unwrap();

    router
        .clone()
        .oneshot(delete_auth(&format!("/api/attachments/{id}"), &editor))
        .await
        .unwrap();

    let audit_response = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(audit_response).await["rows"].clone();
    let rows = rows.as_array().unwrap();

    let create_entry = rows
        .iter()
        .find(|r| r["action"] == "create" && r["resource"] == "attachments")
        .unwrap_or_else(|| panic!("expected a create entry, got {rows:?}"));
    assert_eq!(create_entry["actorUsername"], "editor");
    let create_detail: serde_json::Value = serde_json::from_str(
        create_entry["detail"]
            .as_str()
            .expect("detail should be set"),
    )
    .unwrap();
    assert_eq!(create_detail["fileName"], "photo.bin");
    assert_eq!(create_detail["parentResource"], "customers");
    assert_eq!(create_detail["parentId"], "7");

    let delete_entry = rows
        .iter()
        .find(|r| r["action"] == "delete" && r["resource"] == "attachments")
        .unwrap_or_else(|| panic!("expected a delete entry, got {rows:?}"));
    let delete_detail: serde_json::Value = serde_json::from_str(
        delete_entry["detail"]
            .as_str()
            .expect("detail should be set"),
    )
    .unwrap();
    assert_eq!(delete_detail["fileName"], "photo.bin");
}

/// 経費を消したら、その経費に付いていた領収書（要件 F-E3）も一緒に消える。
/// 監査の `detail` に掃除した件数が入る。Tauri 側の
/// `expenses_delete_body` と同じ振る舞い（conventions §1: 両経路で同じ）。
#[tokio::test]
async fn deleting_an_expense_sweeps_its_attachments_over_rest() {
    let (router, _dir, admin, editor, _viewer) = router_with_role_tokens_and_backup().await;
    let project_id = seed_project(&router, &editor).await;

    let expense = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/expenses",
                &editor,
                json!({
                    "projectId": project_id,
                    "spentOn": "2026-08-20",
                    "expenseCategoryCode": "TRANSPORT",
                    "amount": 1_200,
                    "billable": true
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    let expense_id = expense["id"].as_i64().expect("expense id");

    let upload = router
        .clone()
        .oneshot(post_bytes_auth(
            &format!(
                "/api/attachments?resource=expenses&resourceId={expense_id}&fileName=receipt.txt"
            ),
            &editor,
            b"receipt".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::OK);

    let delete = router
        .clone()
        .oneshot(delete_auth(&format!("/api/expenses/{expense_id}"), &editor))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let listed = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/attachments/list",
                &editor,
                json!({ "resource": "expenses", "resourceId": expense_id.to_string() }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        listed.as_array().expect("array").len(),
        0,
        "領収書が残ってはいけない: {listed:?}"
    );

    let rows = body_json(
        router
            .oneshot(post_json_auth(
                "/api/audit-log/list",
                &admin,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await["rows"]
        .clone();
    let rows = rows.as_array().unwrap();
    let entry = rows
        .iter()
        .find(|r| r["action"] == "delete" && r["resource"] == "expenses")
        .unwrap_or_else(|| panic!("expected a delete/expenses entry, got {rows:?}"));
    let detail: serde_json::Value =
        serde_json::from_str(entry["detail"].as_str().expect("detail present"))
            .expect("detail is json");
    assert_eq!(detail, json!({ "attachmentsRemoved": 1 }));
}

/// Upload/delete each broadcast `ServerEvent::ResourceChanged { resource:
/// "attachments" }` (spec §3.5) - `AttachmentsService` itself has no
/// `ServerEvent` awareness (see this module's doc comment), so this
/// checks the handler-level wiring directly, mirroring the domain routes' own
/// `update_via_rest_is_observable_on_the_event_channel`.
#[tokio::test]
async fn attachment_upload_and_delete_are_observable_on_the_event_channel() {
    let pool = migrate_memory().await.expect("migrate_memory");
    let customers = CustomersService::new(pool.clone());
    let projects = ProjectsService::new(pool.clone());
    let masters = MastersService::new(pool.clone());
    let work_logs = WorkLogsService::new(pool.clone());
    let expenses = ExpensesService::new(pool.clone());
    let trips = TripsService::new(pool.clone());
    let profitability = ProfitabilityService::new(pool.clone());
    let calendar = CalendarService::new(pool.clone());
    let invoices = InvoicesService::new(pool.clone());
    let payments = PaymentsService::new(pool.clone());
    let issuer = IssuerService::new(SettingsService::new(pool.clone()));
    let (tx, mut rx) = broadcast::channel(16);
    let users = UsersService::new(pool.clone());
    let settings = SettingsService::new(pool.clone());
    let sync = SyncService::new(pool.clone(), settings.clone());
    let backup = unused_backup_service(pool.clone());
    let dir = tempdir().expect("tempdir");
    let attachments = AttachmentsService::new(pool.clone(), dir.path().join("attachments"));
    let system_info = SystemInfoService::new(pool.clone());
    let audit = AuditLogService::new(pool);
    let auth = demo_auth();
    let token = auth.login("admin", "admin").await.unwrap();
    let services = Services {
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
    };
    let router = api_router(services, auth, tx, false);

    let upload_response = router
        .clone()
        .oneshot(post_bytes_auth(
            "/api/attachments?resource=customers&resourceId=1&fileName=note.txt",
            &token,
            b"hello".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(upload_response.status(), StatusCode::OK);
    rx.try_recv().expect("upload should emit an event");
    let id = body_json(upload_response).await["id"].as_i64().unwrap();

    router
        .oneshot(delete_auth(&format!("/api/attachments/{id}"), &token))
        .await
        .unwrap();
    let event = rx.try_recv().expect("delete should emit an event");
    assert!(
        matches!(event, ServerEvent::ResourceChanged { resource } if resource == "attachments")
    );
}

// --- Business ドメイン（Phase 2 基本マスター）の両経路対称テスト ---
//
// conventions §1: mutating は REST / Tauri 両経路で同一の認可・監査を通す。
// ここは REST 側の担保（Tauri 側は `src-tauri` の同名テスト）。読み取りは
// 認証のみで通り、監査しない。

/// 顧客の作成に必要なフォーム値（サービス層の検証を通る最小構成）。
fn customer_payload(code: &str) -> serde_json::Value {
    json!({
        "code": code,
        "name": "架空商事",
        "closingDay": 99,
        "paymentMonthOffset": 1,
        "paymentDay": 99
    })
}

#[tokio::test]
async fn viewer_can_read_customers_and_projects() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    for path in ["/api/customers/list", "/api/projects/list"] {
        let response = router
            .clone()
            .oneshot(post_json_auth(path, &viewer, json!(ListParams::default())))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn viewer_cannot_write_customers_or_projects() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    let create = router
        .clone()
        .oneshot(post_json_auth(
            "/api/customers",
            &viewer,
            customer_payload("C001"),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(create).await["kind"], "forbidden");

    let create_project = router
        .clone()
        .oneshot(post_json_auth(
            "/api/projects",
            &viewer,
            json!({ "customerId": 1, "name": "架空案件", "status": "ORDERED" }),
        ))
        .await
        .unwrap();
    assert_eq!(create_project.status(), StatusCode::FORBIDDEN);

    let delete = router
        .oneshot(delete_auth("/api/customers/1", &viewer))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::FORBIDDEN);
}

/// 顧客 → 案件の作成が editor で通り、監査が両方に残ること。
#[tokio::test]
async fn editor_can_create_customer_then_project_and_both_are_audited() {
    let (router, admin, editor, _viewer) = router_with_role_tokens().await;

    let created_customer = router
        .clone()
        .oneshot(post_json_auth(
            "/api/customers",
            &editor,
            customer_payload("C001"),
        ))
        .await
        .unwrap();
    assert_eq!(created_customer.status(), StatusCode::OK);
    let customer = body_json(created_customer).await;
    assert_eq!(customer["code"], "C001");

    let created_project = router
        .clone()
        .oneshot(post_json_auth(
            "/api/projects",
            &editor,
            json!({
                "customerId": customer["id"],
                "name": "架空ライン制御盤更新",
                "status": "ORDERED",
                "contractAmount": 1_200_000
            }),
        ))
        .await
        .unwrap();
    assert_eq!(created_project.status(), StatusCode::OK);
    let project = body_json(created_project).await;
    // 空の code は YYYY-NNN で自動採番される（要件 F-M3）。
    assert!(
        project["code"].as_str().unwrap().ends_with("-001"),
        "auto numbered: {project:?}"
    );

    let audit = router
        .oneshot(post_json_auth(
            "/api/audit-log/list",
            &admin,
            json!(ListParams::default()),
        ))
        .await
        .unwrap();
    let rows = body_json(audit).await["rows"].clone();
    let rows = rows.as_array().expect("rows");
    for resource in ["customers", "projects"] {
        assert!(
            rows.iter()
                .any(|r| r["action"] == "create" && r["resource"] == resource),
            "expected a create/{resource} audit entry, got {rows:?}"
        );
    }
}

/// 顧客の削除は、案件が紐づいている間は 422（Validation）で拒否される。
/// 外部キー任せの 500 にしないことがサービス層の設計判断（`customers.rs`）。
#[tokio::test]
async fn deleting_a_customer_with_projects_is_a_validation_error() {
    let (router, _admin, editor, _viewer) = router_with_role_tokens().await;

    let customer = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/customers",
                &editor,
                customer_payload("C001"),
            ))
            .await
            .unwrap(),
    )
    .await;
    router
        .clone()
        .oneshot(post_json_auth(
            "/api/projects",
            &editor,
            json!({ "customerId": customer["id"], "name": "架空案件", "status": "ORDERED" }),
        ))
        .await
        .unwrap();

    let response = router
        .oneshot(delete_auth(
            &format!("/api/customers/{}", customer["id"]),
            &editor,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["kind"], "validation");
}

// --- Business ドメイン（Phase 3 工数・経費）の両経路対称テスト ---

/// 顧客 → 案件 → レート設定まで済ませ、案件 id を返す。
async fn seed_project(router: &Router, editor: &str) -> i64 {
    let customer = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/customers",
                editor,
                customer_payload("C001"),
            ))
            .await
            .unwrap(),
    )
    .await;
    let project = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/projects",
                editor,
                json!({ "customerId": customer["id"], "name": "架空案件", "status": "IN_PROGRESS" }),
            ))
            .await
            .unwrap(),
    )
    .await;
    // レートは分類コードを id として PUT する（DataProvider の
    // `update(resource, id, values)` に対応。docs/recipes/add-resource.md）。
    for (code, rate) in [("DESIGN", 6000), ("TRAVEL", 3000), ("ONSITE", 6000)] {
        let response = router
            .clone()
            .oneshot(put_json_auth(
                &format!("/api/cost_rates/{code}"),
                editor,
                json!({ "hourlyRate": rate }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "rate {code}");
    }
    project["id"].as_i64().expect("project id")
}

#[tokio::test]
async fn viewer_can_read_masters_but_cannot_set_rates() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    for path in ["/api/work_categories/list", "/api/expense_categories/list"] {
        let response = router
            .clone()
            .oneshot(post_json_auth(path, &viewer, json!(ListParams::default())))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }

    let denied = router
        .oneshot(put_json_auth(
            "/api/cost_rates/DESIGN",
            &viewer,
            json!({ "hourlyRate": 1000 }),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

/// 工数の作成が REST 経路でもレートをスナップショットし、原価を保存する
/// （サービス層と同じ挙動が REST 経由でも成り立つことの確認）。
#[tokio::test]
async fn editor_creates_a_work_log_with_snapshotted_rate() {
    let (router, _admin, editor, _viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;

    let created = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/work_logs",
                &editor,
                json!({
                    "projectId": project_id,
                    "workedOn": "2026-08-20",
                    "workCategoryCode": "DESIGN",
                    "minutes": 90
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(created["appliedRate"], 6000);
    assert_eq!(created["internalCost"], 9000);

    let denied = router
        .oneshot(post_json_auth(
            "/api/work_logs",
            "not-a-token",
            json!({ "projectId": project_id }),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

/// 出張の一括生成が REST 経由で動き、監査に生成件数が残る（要件 F-T1）。
#[tokio::test]
async fn trip_generation_creates_rows_and_records_the_counts() {
    let (router, admin, editor, _viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;

    let result = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/trips",
                &editor,
                json!({
                    "projectId": project_id,
                    "destination": "架空工業 本社工場",
                    "startOn": "2026-09-01",
                    "endOn": "2026-09-03",
                    "onsiteDays": 3,
                    "nights": 2,
                    "generate": {
                        "travelMinutesOneWay": 180,
                        "onsiteMinutesPerDay": 480,
                        "transportAmount": 24000,
                        "lodgingAmountPerNight": 9500
                    }
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(result["travelWorkLogs"], 2);
    assert_eq!(result["onsiteWorkLogs"], 3);
    assert_eq!(result["expenses"], 2);

    let logs = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/work_logs/list",
                &editor,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(logs["totalCount"], 5);

    let audit = body_json(
        router
            .oneshot(post_json_auth(
                "/api/audit-log/list",
                &admin,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    let entry = audit["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["action"] == "create" && r["resource"] == "trips")
        .expect("trips create audit entry")
        .clone();
    let detail: serde_json::Value =
        serde_json::from_str(entry["detail"].as_str().expect("detail")).expect("detail json");
    assert_eq!(detail["generatedWorkLogs"], 5);
    assert_eq!(detail["generatedExpenses"], 2);
}

#[tokio::test]
async fn viewer_cannot_write_work_logs_expenses_or_trips() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    for (path, payload) in [
        (
            "/api/work_logs",
            json!({ "projectId": 1, "workedOn": "2026-08-20", "workCategoryCode": "DESIGN", "minutes": 60 }),
        ),
        (
            "/api/expenses",
            json!({ "projectId": 1, "spentOn": "2026-08-20", "expenseCategoryCode": "TRANSPORT", "amount": 1000 }),
        ),
        (
            "/api/trips",
            json!({ "projectId": 1, "destination": "架空工業", "startOn": "2026-09-01", "endOn": "2026-09-02", "onsiteDays": 1, "nights": 1 }),
        ),
    ] {
        let response = router
            .clone()
            .oneshot(post_json_auth(path, &viewer, payload))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

// --- Business ドメイン（Phase 4 採算管理）の読み取り経路 ---

/// 採算は viewer でも読める（読み取りは認証のみ。conventions §1）。
/// 実質時間単価は移動込み・移動除くが**同時に**返る（要件 F-P2）。
#[tokio::test]
async fn viewer_can_read_project_profitability_with_both_effective_rates() {
    let (router, _admin, editor, viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;

    for (code, minutes) in [("DESIGN", 600), ("TRAVEL", 300)] {
        let response = router
            .clone()
            .oneshot(post_json_auth(
                "/api/work_logs",
                &editor,
                json!({
                    "projectId": project_id,
                    "workedOn": "2026-08-20",
                    "workCategoryCode": code,
                    "minutes": minutes
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{code}");
    }

    let response = router
        .clone()
        .oneshot(get_auth(
            &format!("/api/profitability/{project_id}"),
            &viewer,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    // 設計 600分 × 6,000円/時 = 60,000円 + 移動 300分 × 3,000円/時 = 15,000円
    assert_eq!(body["workCost"], 75_000);
    assert_eq!(body["totalMinutes"], 900);
    assert_eq!(body["excludedMinutes"], 300);
    assert_eq!(body["effectiveRateIncludingTravel"], -5_000);
    assert_eq!(body["effectiveRateExcludingTravel"], -7_500);
}

#[tokio::test]
async fn profitability_of_an_unknown_project_is_not_found() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(get_auth("/api/profitability/9999", &viewer))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// --- Business ドメイン（Phase 7 準備：月カレンダー）の読み取り経路 ---

/// カレンダーは viewer でも読める（読み取りは認証のみ。conventions §1）。
/// 行は日付順で、何も無い日は返らない。
#[tokio::test]
async fn viewer_can_read_the_month_calendar() {
    let (router, _admin, editor, viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;

    for (worked_on, minutes) in [("2026-08-20", 600), ("2026-08-03", 120)] {
        let response = router
            .clone()
            .oneshot(post_json_auth(
                "/api/work_logs",
                &editor,
                json!({
                    "projectId": project_id,
                    "workedOn": worked_on,
                    "workCategoryCode": "DESIGN",
                    "minutes": minutes
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{worked_on}");
    }

    let response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/calendar/list",
            &viewer,
            json!({
                "sort": [],
                "filters": [{ "field": "month", "op": "eq", "value": "2026-08" }]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(body["totalCount"], 2);
    // 日付順（`BTreeMap` のキー順）。行の id は日付そのもの。
    assert_eq!(rows[0]["date"], "2026-08-03");
    assert_eq!(rows[0]["id"], "2026-08-03");
    assert_eq!(rows[0]["workedMinutes"], 120);
    assert_eq!(rows[1]["date"], "2026-08-20");
    assert_eq!(rows[1]["workedMinutes"], 600);
    assert_eq!(rows[1]["projects"][0]["projectId"], project_id);
}

/// 月フィルタが無ければ 422。既定で「今月」に倒すと、指定漏れが黙って
/// 別の月を返す形で表に出る。
#[tokio::test]
async fn the_calendar_requires_a_month_filter() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(post_json_auth(
            "/api/calendar/list",
            &viewer,
            json!({ "sort": [], "filters": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert_eq!(body["kind"], "validation");
}

/// 月として読めない指定も 422（「その月にデータが無い」と区別する）。
#[tokio::test]
async fn a_malformed_calendar_month_is_rejected() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(post_json_auth(
            "/api/calendar/list",
            &viewer,
            json!({
                "sort": [],
                "filters": [{ "field": "month", "op": "eq", "value": "2026-13" }]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// 読み取りルートなのでトークンが要る（`require_auth`）。
#[tokio::test]
async fn the_calendar_route_requires_a_token() {
    let (router, _token) = router_with_token().await;
    let response = router
        .oneshot(
            HttpRequest::post("/api/calendar/list")
                .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "sort": [],
                        "filters": [{ "field": "month", "op": "eq", "value": "2026-08" }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// --- Business ドメイン（Phase 5 請求）の両経路対称テスト ---

/// 請求書の作成 → 確定が REST 経由で動き、監査に採番した番号が残る。
#[tokio::test]
async fn editor_creates_and_issues_an_invoice_over_rest() {
    let (router, admin, editor, viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;
    let customer = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/customers/list",
                &viewer,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    let customer_id = customer["rows"][0]["id"].as_i64().expect("customer id");

    let created = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/invoices",
                &editor,
                json!({
                    "customerId": customer_id,
                    "lines": [
                        {
                            "projectId": project_id,
                            "itemName": "設計",
                            "quantity": 1,
                            "unitPrice": 33_335,
                            "taxCategory": "STANDARD_10"
                        },
                        {
                            "projectId": project_id,
                            "itemName": "現地調整",
                            "quantity": 2,
                            "unitPrice": 33_335,
                            "taxCategory": "STANDARD_10"
                        }
                    ]
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(created["status"], "DRAFT");
    assert!(created["invoiceNumber"].is_null(), "{created}");
    let invoice_id = created["id"].as_i64().expect("invoice id");

    let issued = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                &format!("/api/invoices/{invoice_id}/issue"),
                &editor,
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(issued["status"], "ISSUED");
    // 税率区分ごとに1回だけ端数処理する: floor(100,005 × 10%) = 10,000
    assert_eq!(issued["totalTaxable"], 100_005);
    assert_eq!(issued["totalTax"], 10_000);
    assert_eq!(issued["taxSummaries"][0]["rateBp"], 1_000);
    let number = issued["invoiceNumber"]
        .as_str()
        .expect("number")
        .to_string();

    // viewer は読めるが書けない。
    let read = router
        .clone()
        .oneshot(get_auth(&format!("/api/invoices/{invoice_id}"), &viewer))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let denied = router
        .clone()
        .oneshot(post_json_auth(
            "/api/invoices",
            &viewer,
            json!({ "customerId": customer_id, "lines": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // 確定は監査に番号を残す。
    let audit = body_json(
        router
            .oneshot(post_json_auth(
                "/api/audit-log/list",
                &admin,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    let entry = audit["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["action"] == "issue" && r["resource"] == "invoices")
        .expect("issue audit entry")
        .clone();
    let detail: serde_json::Value =
        serde_json::from_str(entry["detail"].as_str().expect("detail")).expect("detail json");
    assert_eq!(detail["invoiceNumber"], number);
}

/// 発行者情報は admin のみ（登録番号・振込先はアプリ全体の設定）。
#[tokio::test]
async fn issuer_settings_are_admin_only() {
    let (router, admin, editor, viewer) = router_with_role_tokens().await;

    let saved = body_json(
        router
            .clone()
            .oneshot(put_json_auth(
                "/api/issuer",
                &admin,
                json!({
                    "name": "架空設計事務所",
                    "registrationNumber": "T1234567890123",
                    "roundingMode": "FLOOR"
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(saved["registrationNumber"], "T1234567890123");

    for token in [&editor, &viewer] {
        let denied = router
            .clone()
            .oneshot(get_auth("/api/issuer", token))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    }
}

/// 未請求の工数・経費から候補を作る（要件 F-I1）。読み取りなのでロール床は viewer。
#[tokio::test]
async fn viewer_can_build_invoice_candidates() {
    let (router, _admin, editor, viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;
    let customer = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/customers/list",
                &viewer,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    let customer_id = customer["rows"][0]["id"].as_i64().expect("customer id");

    router
        .clone()
        .oneshot(post_json_auth(
            "/api/work_logs",
            &editor,
            json!({
                "projectId": project_id,
                "workedOn": "2026-08-20",
                "workCategoryCode": "DESIGN",
                "minutes": 60
            }),
        ))
        .await
        .unwrap();

    let candidates = body_json(
        router
            .oneshot(post_json_auth(
                "/api/invoices/candidates",
                &viewer,
                json!({ "customerId": customer_id, "from": "2026-08-01", "to": "2026-08-31" }),
            ))
            .await
            .unwrap(),
    )
    .await;
    let rows = candidates.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sourceType"], "WORK_LOG");
    // seed_project の案件は請求単価が未設定なので金額 0 で出る（画面で入力を促す）。
    assert!(rows[0]["billingHourlyRate"].is_null(), "{candidates}");
    assert_eq!(rows[0]["amount"], 0);
}

// --- Business ドメイン（Phase 6 入金管理）の両経路対称テスト ---

/// 入金の登録と消込が REST 経由で動き、請求書の残額・入金状態が導出される。
#[tokio::test]
async fn editor_records_a_payment_and_the_invoice_settles() {
    let (router, _admin, editor, viewer) = router_with_role_tokens().await;
    let project_id = seed_project(&router, &editor).await;
    let customers = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/customers/list",
                &viewer,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    let customer_id = customers["rows"][0]["id"].as_i64().expect("customer id");

    let created = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/invoices",
                &editor,
                json!({
                    "customerId": customer_id,
                    "lines": [{
                        "projectId": project_id,
                        "itemName": "設計",
                        "quantity": 1,
                        "unitPrice": 100_000,
                        "taxCategory": "STANDARD_10"
                    }]
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    let invoice_id = created["id"].as_i64().expect("invoice id");
    let issued = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                &format!("/api/invoices/{invoice_id}/issue"),
                &editor,
                json!({}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(issued["totalAmount"], 110_000);

    // 先方が手数料 660 円を差し引いて入金（決定 C-19: 差額で請求書を閉じる）。
    let payment = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/payments",
                &editor,
                json!({
                    "customerId": customer_id,
                    "paidOn": "2026-09-30",
                    "amount": 109_340,
                    "allocations": [{
                        "invoiceId": invoice_id,
                        "allocatedAmount": 109_340,
                        "differenceReason": "TRANSFER_FEE",
                        "differenceAmount": 660
                    }]
                }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(payment["unallocatedAmount"], 0);

    let settlement = body_json(
        router
            .clone()
            .oneshot(get_auth(&format!("/api/settlements/{invoice_id}"), &viewer))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(settlement["settledAmount"], 110_000);
    assert_eq!(settlement["remainingAmount"], 0);
    assert_eq!(settlement["settlementStatus"], "PAID");
    assert_eq!(settlement["overdue"], false);

    // 完済したので未入金一覧には出ない（要件 F-Y7）。
    let outstanding = body_json(
        router
            .clone()
            .oneshot(post_json_auth(
                "/api/outstanding/list",
                &viewer,
                json!(ListParams::default()),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(outstanding["totalCount"], 0);

    // viewer は読めるが書けない。
    let denied = router
        .oneshot(post_json_auth(
            "/api/payments",
            &viewer,
            json!({ "customerId": customer_id, "paidOn": "2026-09-30", "amount": 1_000 }),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

// --- Phase 8 デバイス間同期（docs/domain/sync.md 11節） ---
//
// PC 側は受けるだけで、この段は**読み取りのみ**。認証は要るがロール床は
// 無く（返る中身は既存の `*_list` と同じ）、監査もしない（読み取りは
// 監査しない、conventions §1）。

#[tokio::test]
async fn sync_requires_authentication() {
    let (router, _admin, _editor, _viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(post_json("/api/sync/pull", json!({ "afterSeq": 0 })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// 読み取りなのでロール床は無い（`viewer` で通る）。
#[tokio::test]
async fn viewer_can_handshake_and_pull() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;

    let response = router
        .clone()
        .oneshot(post_json_auth(
            "/api/sync/handshake",
            &viewer,
            json!({ "peerDeviceId": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["deviceId"], 0);
    assert_eq!(body["outboxHead"], 0);
    assert_eq!(body["receivedThroughSeq"], 0);
    assert_eq!(body["tables"].as_array().expect("tables").len(), 8);

    let response = router
        .oneshot(post_json_auth(
            "/api/sync/pull",
            &viewer,
            json!({ "afterSeq": 0 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["rows"], json!([]));
}

/// **同じデバイス番号を名乗る相手は 422 で断る。** 番号が同じということは
/// id の採番レンジが分かれておらず、そのまま同期すると別々の行が同じ id を
/// 持ったまま混ざる（docs/domain/sync.md 3節）。
#[tokio::test]
async fn a_peer_claiming_the_same_device_number_is_rejected() {
    let (router, _admin, _editor, viewer) = router_with_role_tokens().await;
    let response = router
        .oneshot(post_json_auth(
            "/api/sync/handshake",
            &viewer,
            json!({ "peerDeviceId": 0 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// REST 経由で作った行が、そのまま同期の行として引けること。
/// 列名は DB の綴り（スネークケース）のまま運ばれる。
#[tokio::test]
async fn a_row_created_over_rest_is_pullable() {
    let (router, _admin, editor, viewer) = router_with_role_tokens().await;

    let created = router
        .clone()
        .oneshot(post_json_auth(
            "/api/customers",
            &editor,
            customer_payload("C001"),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let id = body_json(created).await["id"].as_i64().expect("id");

    let response = router
        .oneshot(post_json_auth(
            "/api/sync/pull",
            &viewer,
            json!({ "afterSeq": 0 }),
        ))
        .await
        .unwrap();
    let body = body_json(response).await;
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["table"], "customers");
    assert_eq!(rows[0]["key"], id.to_string());
    assert_eq!(rows[0]["values"]["name"], "架空商事");
    // 未設定の任意項目は素の null（空文字に化けない）。
    assert_eq!(rows[0]["values"]["note"], serde_json::Value::Null);
    assert_eq!(rows[0]["values"]["deleted_at"], serde_json::Value::Null);
    assert!(body["throughSeq"].as_i64().expect("throughSeq") > 0);
    assert_eq!(body["throughSeq"], body["headSeq"]);
}

/// 読み取りは監査しない（conventions §1）。
#[tokio::test]
async fn sync_reads_are_not_audited() {
    let (router, audit, admin, _editor, viewer) = router_with_role_tokens_and_audit().await;

    router
        .clone()
        .oneshot(post_json_auth(
            "/api/sync/handshake",
            &viewer,
            json!({ "peerDeviceId": 1 }),
        ))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post_json_auth(
            "/api/sync/pull",
            &viewer,
            json!({ "afterSeq": 0 }),
        ))
        .await
        .unwrap();

    let _ = admin;
    let entries = audit.list(ListParams::default()).await.expect("audit list");
    assert!(
        entries.rows.iter().all(|entry| entry.resource != "sync"),
        "同期の読み取りは監査に残らないこと: {:?}",
        entries.rows
    );
}
