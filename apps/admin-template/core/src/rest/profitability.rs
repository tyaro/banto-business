use super::*;

/// 案件採算の読み取り（conventions §1: 読み取りは認証のみ・監査しない）。
///
/// 書き込みルートは存在しない — 採算値は保持せず常に導出する（F-P7）ので、
/// 変更できる対象そのものが無い。
async fn profitability_get(
    State(profitability): State<ProfitabilityService>,
    Path(project_id): Path<i64>,
) -> Result<Json<ProjectProfitability>, ApiError> {
    Ok(Json(profitability.get(project_id).await?))
}

/// `/api/profitability/{projectId}`（任意のロール）。
pub(super) fn profitability_router(profitability: ProfitabilityService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/profitability/{id}", get(profitability_get))
        .with_state(profitability)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}
