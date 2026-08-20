use super::*;

/// 工数の読み取りルート（conventions §1: 読み取りは認証のみ・監査しない）。
async fn work_logs_list(
    State(work_logs): State<WorkLogsService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<WorkLog>>, ApiError> {
    Ok(Json(work_logs.list(params).await?))
}

async fn work_logs_get(
    State(work_logs): State<WorkLogsService>,
    Path(id): Path<i64>,
) -> Result<Json<WorkLog>, ApiError> {
    Ok(Json(work_logs.get(id).await?))
}

/// 書き込みハンドラの State（`items` の `ItemsWriteState` と同型）:
/// 変更そのものに使う `WorkLogsService` と、成功後に監査記録を残すための
/// `AuditLogService` / `AuthState`（conventions §1: mutating は両経路で
/// 同一の認可・監査を通す）。
#[derive(Clone)]
struct WorkLogsWriteState {
    work_logs: WorkLogsService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn work_logs_create(
    State(state): State<WorkLogsWriteState>,
    headers: HeaderMap,
    Json(input): Json<WorkLogInput>,
) -> Result<Json<WorkLog>, ApiError> {
    let work_log = state.work_logs.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "work-logs",
        Some(&work_log.id.to_string()),
        Some(json!({ "projectId": work_log.project_id, "workedOn": work_log.worked_on })),
    )
    .await;
    Ok(Json(work_log))
}

async fn work_logs_update(
    State(state): State<WorkLogsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<WorkLogInput>,
) -> Result<Json<WorkLog>, ApiError> {
    let work_log = state.work_logs.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "work-logs",
        Some(&work_log.id.to_string()),
        Some(json!({ "projectId": work_log.project_id, "workedOn": work_log.worked_on })),
    )
    .await;
    Ok(Json(work_log))
}

async fn work_logs_delete(
    State(state): State<WorkLogsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.work_logs.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "work-logs",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 読み取り（任意のロール）。
fn work_logs_read_router(work_logs: WorkLogsService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/work-logs/list", post(work_logs_list))
        .route("/api/work-logs/{id}", get(work_logs_get))
        .with_state(work_logs)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// 変更（editor 以上）。`items_write_router` と同じ層順（require_auth →
/// require_role_at_least）で、セッションが無い要求はロール判定に到達しない。
fn work_logs_write_router(
    work_logs: WorkLogsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = WorkLogsWriteState {
        work_logs,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/work-logs", post(work_logs_create))
        .route(
            "/api/work-logs/{id}",
            axum::routing::put(work_logs_update).delete(work_logs_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "work-logs",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/work_logs/*`（読み取りと変更をマージ。`/api/work_logs/{id}` は
/// メソッドで分かれる）。
pub(super) fn work_logs_router(
    work_logs: WorkLogsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    work_logs_read_router(work_logs.clone(), auth.clone())
        .merge(work_logs_write_router(work_logs, audit, auth))
}
