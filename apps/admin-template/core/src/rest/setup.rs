use super::*;
use crate::issuer::IssuerService;
use crate::setup::{setup_status, SetupStatus};
use banto_storage::Db;

/// 初期セットアップの道しるべ（`docs/mobile-ui-plan.md` P2-1）。読み取り専用
/// なので mutating な監査は残さない（`issuer_get` と同じ扱い）が、`RoleGuard`
/// 自体は denied（ロール不足）を記録するため `audit` を持つ。
#[derive(Clone)]
struct SetupStatusState {
    db: Db,
    issuer: IssuerService,
}

async fn setup_status_get(
    State(state): State<SetupStatusState>,
) -> Result<Json<SetupStatus>, ApiError> {
    Ok(Json(setup_status(&state.db, &state.issuer).await?))
}

/// `/api/setup-status`（**admin のみ**。発行者情報の有無を含むため `issuer`
/// と同じ床にする）。
pub(super) fn setup_router(
    db: Db,
    issuer: IssuerService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    Router::new()
        .route("/api/setup-status", get(setup_status_get))
        .with_state(SetupStatusState { db, issuer })
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Admin,
                resource: "setup_status",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}
