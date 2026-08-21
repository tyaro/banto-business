use super::*;

/// 同期の確認（conventions §1: 読み取りは認証のみ・監査しない）。
///
/// **PC を書き換えない。** `sync_state` に進捗を刻むのは次段の取り込み
/// （push）で、こちらは読むだけ（`crate::sync::protocol` モジュールの
/// 冒頭を参照）。
async fn sync_handshake(
    State(sync): State<SyncService>,
    Json(request): Json<HandshakeRequest>,
) -> Result<Json<Handshake>, ApiError> {
    Ok(Json(sync.handshake(request).await?))
}

/// PC 側で変わった行を引く（同上、読み取り）。
///
/// `POST` なのは、`after_seq` を本文で受けるため —— 他の `*_list` と同じ
/// 理由（`rest/mod.rs` 冒頭）。
async fn sync_pull(
    State(sync): State<SyncService>,
    Json(request): Json<PullRequest>,
) -> Result<Json<Pull>, ApiError> {
    Ok(Json(sync.pull(request).await?))
}

/// `/api/sync/*`（任意のロール）。
///
/// 読み取りにロール床を足さないのは、返す中身が既存の `*_list` /
/// `*_get` で同じロールから読めるものと同じだから。ここだけ床を上げても
/// 実際に守れるものが無く、規約（読み取りは認証のみ）とずれるだけになる。
pub(super) fn sync_router(sync: SyncService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/sync/handshake", post(sync_handshake))
        .route("/api/sync/pull", post(sync_pull))
        .with_state(sync)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}
