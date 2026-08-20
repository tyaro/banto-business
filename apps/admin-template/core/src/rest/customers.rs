use super::*;

/// 顧客の読み取りルート（conventions §1: 読み取りは認証のみ・監査しない）。
async fn customers_list(
    State(customers): State<CustomersService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Customer>>, ApiError> {
    Ok(Json(customers.list(params).await?))
}

async fn customers_get(
    State(customers): State<CustomersService>,
    Path(id): Path<i64>,
) -> Result<Json<Customer>, ApiError> {
    Ok(Json(customers.get(id).await?))
}

/// 書き込みハンドラの State（`items` の `ItemsWriteState` と同型）:
/// 変更そのものに使う `CustomersService` と、成功後に監査記録を残すための
/// `AuditLogService` / `AuthState`（conventions §1: mutating は両経路で
/// 同一の認可・監査を通す）。
#[derive(Clone)]
struct CustomersWriteState {
    customers: CustomersService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn customers_create(
    State(state): State<CustomersWriteState>,
    headers: HeaderMap,
    Json(input): Json<CustomerInput>,
) -> Result<Json<Customer>, ApiError> {
    let customer = state.customers.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "customers",
        Some(&customer.id.to_string()),
        Some(json!({ "code": customer.code })),
    )
    .await;
    Ok(Json(customer))
}

async fn customers_update(
    State(state): State<CustomersWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<CustomerInput>,
) -> Result<Json<Customer>, ApiError> {
    let customer = state.customers.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "customers",
        Some(&customer.id.to_string()),
        Some(json!({ "code": customer.code })),
    )
    .await;
    Ok(Json(customer))
}

async fn customers_delete(
    State(state): State<CustomersWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.customers.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "customers",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 読み取り（任意のロール）。
fn customers_read_router(customers: CustomersService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/customers/list", post(customers_list))
        .route("/api/customers/{id}", get(customers_get))
        .with_state(customers)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// 変更（editor 以上）。`items_write_router` と同じ層順（require_auth →
/// require_role_at_least）で、セッションが無い要求はロール判定に到達しない。
fn customers_write_router(
    customers: CustomersService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = CustomersWriteState {
        customers,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/customers", post(customers_create))
        .route(
            "/api/customers/{id}",
            axum::routing::put(customers_update).delete(customers_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "customers",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/customers/*`（読み取りと変更をマージ。`/api/customers/{id}` は
/// メソッドで分かれる）。
pub(super) fn customers_router(
    customers: CustomersService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    customers_read_router(customers.clone(), auth.clone())
        .merge(customers_write_router(customers, audit, auth))
}
