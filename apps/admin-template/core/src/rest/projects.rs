use super::*;

/// 案件の読み取りルート（conventions §1: 読み取りは認証のみ・監査しない）。
async fn projects_list(
    State(projects): State<ProjectsService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Project>>, ApiError> {
    Ok(Json(projects.list(params).await?))
}

async fn projects_get(
    State(projects): State<ProjectsService>,
    Path(id): Path<i64>,
) -> Result<Json<Project>, ApiError> {
    Ok(Json(projects.get(id).await?))
}

/// 書き込みハンドラの State（`customers` の `CustomersWriteState` と同型）:
/// 変更そのものに使う `ProjectsService` と、成功後に監査記録を残すための
/// `AuditLogService` / `AuthState`（conventions §1: mutating は両経路で
/// 同一の認可・監査を通す）。
#[derive(Clone)]
struct ProjectsWriteState {
    projects: ProjectsService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn projects_create(
    State(state): State<ProjectsWriteState>,
    headers: HeaderMap,
    Json(input): Json<ProjectInput>,
) -> Result<Json<Project>, ApiError> {
    let customer = state.projects.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "projects",
        Some(&customer.id.to_string()),
        Some(json!({ "code": customer.code })),
    )
    .await;
    Ok(Json(customer))
}

async fn projects_update(
    State(state): State<ProjectsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<ProjectInput>,
) -> Result<Json<Project>, ApiError> {
    let customer = state.projects.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "projects",
        Some(&customer.id.to_string()),
        Some(json!({ "code": customer.code })),
    )
    .await;
    Ok(Json(customer))
}

async fn projects_delete(
    State(state): State<ProjectsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.projects.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "projects",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 読み取り（任意のロール）。
fn projects_read_router(projects: ProjectsService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/projects/list", post(projects_list))
        .route("/api/projects/{id}", get(projects_get))
        .with_state(projects)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// 変更（editor 以上）。`customers_write_router` と同じ層順（require_auth →
/// require_role_at_least）で、セッションが無い要求はロール判定に到達しない。
fn projects_write_router(
    projects: ProjectsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = ProjectsWriteState {
        projects,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/projects", post(projects_create))
        .route(
            "/api/projects/{id}",
            axum::routing::put(projects_update).delete(projects_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "projects",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/projects/*`（読み取りと変更をマージ。`/api/projects/{id}` は
/// メソッドで分かれる）。
pub(super) fn projects_router(
    projects: ProjectsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    projects_read_router(projects.clone(), auth.clone())
        .merge(projects_write_router(projects, audit, auth))
}
