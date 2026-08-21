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

/// 書き込みハンドラの State（他のドメインルータと同型）。
#[derive(Clone)]
struct SyncWriteState {
    sync: SyncService,
    audit: AuditLogService,
    auth: AuthState,
}

/// 相手から送られてきた行を取り込む（editor 以上・監査する）。
///
/// **この経路だけが業務データを書き換える。** 相手が何を持ち込んだかは
/// 後から辿れる必要があるので、他の変更ルートと同じく監査に残す
/// （conventions §1）。detail には件数だけを入れ、行の中身は入れない ——
/// 監査ログに顧客名や金額を写すと、監査ログ自体が業務データの複製になる。
async fn sync_push(
    State(state): State<SyncWriteState>,
    headers: HeaderMap,
    Json(request): Json<PushRequest>,
) -> Result<Json<PushResult>, ApiError> {
    let peer_device_id = request.peer_device_id;
    let result = state.sync.push(request).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "push",
        "sync",
        Some(&peer_device_id.to_string()),
        Some(json!({
            "applied": result.applied,
            "unchanged": result.unchanged,
            "conflicts": result.conflicts.len(),
            "receivedThroughSeq": result.received_through_seq,
        })),
    )
    .await;
    Ok(Json(result))
}

/// 読み取り（任意のロール）。
///
/// ロール床を足さないのは、返す中身が既存の `*_list` / `*_get` で同じロール
/// から読めるものと同じだから。ここだけ床を上げても実際に守れるものが無く、
/// 規約（読み取りは認証のみ）とずれるだけになる。
fn sync_read_router(sync: SyncService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/sync/handshake", post(sync_handshake))
        .route("/api/sync/pull", post(sync_pull))
        .with_state(sync)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// 取り込み（editor 以上）。他の書き込みルータと同じ層順
/// （require_auth → require_role_at_least）。
fn sync_write_router(sync: SyncService, audit: AuditLogService, auth: AuthState) -> Router {
    let state = SyncWriteState {
        sync,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/sync/push", post(sync_push))
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "sync",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/sync/*`（読み取りと取り込みをマージ）。
pub(super) fn sync_router(sync: SyncService, audit: AuditLogService, auth: AuthState) -> Router {
    sync_read_router(sync.clone(), auth.clone()).merge(sync_write_router(sync, audit, auth))
}
