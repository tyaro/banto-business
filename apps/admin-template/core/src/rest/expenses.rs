use super::*;

/// 経費の読み取りルート（conventions §1: 読み取りは認証のみ・監査しない）。
async fn expenses_list(
    State(expenses): State<ExpensesService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Expense>>, ApiError> {
    Ok(Json(expenses.list(params).await?))
}

async fn expenses_get(
    State(expenses): State<ExpensesService>,
    Path(id): Path<i64>,
) -> Result<Json<Expense>, ApiError> {
    Ok(Json(expenses.get(id).await?))
}

/// 書き込みハンドラの State（`items` の `ItemsWriteState` と同型）:
/// 変更そのものに使う `ExpensesService` と、成功後に監査記録を残すための
/// `AuditLogService` / `AuthState`（conventions §1: mutating は両経路で
/// 同一の認可・監査を通す）。
#[derive(Clone)]
struct ExpensesWriteState {
    expenses: ExpensesService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn expenses_create(
    State(state): State<ExpensesWriteState>,
    headers: HeaderMap,
    Json(input): Json<ExpenseInput>,
) -> Result<Json<Expense>, ApiError> {
    let expense = state.expenses.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "expenses",
        Some(&expense.id.to_string()),
        Some(json!({ "projectId": expense.project_id, "spentOn": expense.spent_on })),
    )
    .await;
    Ok(Json(expense))
}

async fn expenses_update(
    State(state): State<ExpensesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<ExpenseInput>,
) -> Result<Json<Expense>, ApiError> {
    let expense = state.expenses.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "expenses",
        Some(&expense.id.to_string()),
        Some(json!({ "projectId": expense.project_id, "spentOn": expense.spent_on })),
    )
    .await;
    Ok(Json(expense))
}

async fn expenses_delete(
    State(state): State<ExpensesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.expenses.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "expenses",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 読み取り（任意のロール）。
fn expenses_read_router(expenses: ExpensesService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/expenses/list", post(expenses_list))
        .route("/api/expenses/{id}", get(expenses_get))
        .with_state(expenses)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// 変更（editor 以上）。`items_write_router` と同じ層順（require_auth →
/// require_role_at_least）で、セッションが無い要求はロール判定に到達しない。
fn expenses_write_router(
    expenses: ExpensesService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = ExpensesWriteState {
        expenses,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/expenses", post(expenses_create))
        .route(
            "/api/expenses/{id}",
            axum::routing::put(expenses_update).delete(expenses_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "expenses",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/expenses/*`（読み取りと変更をマージ。`/api/expenses/{id}` は
/// メソッドで分かれる）。
pub(super) fn expenses_router(
    expenses: ExpensesService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    expenses_read_router(expenses.clone(), auth.clone())
        .merge(expenses_write_router(expenses, audit, auth))
}
