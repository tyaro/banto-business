use super::*;

/// 発行者情報の読み取り。**admin のみ**（`settings` と同じ床。事業者の登録番号・
/// 住所・振込先はアプリ全体の設定であり、閲覧者に配る情報ではない）。
async fn issuer_get(State(issuer): State<IssuerService>) -> Result<Json<IssuerSettings>, ApiError> {
    Ok(Json(issuer.get().await?))
}

#[derive(Clone)]
struct IssuerWriteState {
    issuer: IssuerService,
    audit: AuditLogService,
    auth: AuthState,
}

/// 更新。監査の detail に値そのものは残さない — 登録番号や振込先が監査ログ
/// 経由で広がらないようにする（`CLAUDE.md` 第8章）。
async fn issuer_update(
    State(state): State<IssuerWriteState>,
    headers: HeaderMap,
    Json(input): Json<IssuerInput>,
) -> Result<Json<IssuerSettings>, ApiError> {
    let settings = state.issuer.set(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "issuer",
        None,
        Some(json!({ "roundingMode": settings.rounding_mode })),
    )
    .await;
    Ok(Json(settings))
}

/// `/api/issuer`（admin のみ、読み書きとも）。
pub(super) fn issuer_router(
    issuer: IssuerService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let read = Router::new()
        .route("/api/issuer", get(issuer_get))
        .with_state(issuer.clone());
    let write = Router::new()
        .route("/api/issuer", axum::routing::put(issuer_update))
        .with_state(IssuerWriteState {
            issuer,
            audit: audit.clone(),
            auth: auth.clone(),
        });
    read.merge(write)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Admin,
                resource: "issuer",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}
