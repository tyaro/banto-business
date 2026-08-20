use super::*;

/// 入金の読み取り（conventions §1: 読み取りは認証のみ・監査しない）。
async fn payments_list(
    State(payments): State<PaymentsService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Payment>>, ApiError> {
    Ok(Json(payments.list(params).await?))
}

async fn payments_get(
    State(payments): State<PaymentsService>,
    Path(id): Path<i64>,
) -> Result<Json<PaymentDetail>, ApiError> {
    Ok(Json(payments.get(id).await?))
}

/// 請求書1件の入金状況（要件 F-Y4〜F-Y6）。id は**請求書 id**。
/// 残額・入金状態・期限超過はすべて導出値で、列としては持たない。
async fn settlements_get(
    State(payments): State<PaymentsService>,
    Path(invoice_id): Path<i64>,
) -> Result<Json<InvoiceSettlement>, ApiError> {
    Ok(Json(payments.settlement(invoice_id).await?))
}

/// 未入金・期限超過の一覧（要件 F-Y7）。`ListParams` を取るのは DataProvider の
/// `getList` 契約に合わせるためだけで、絞り込み・ページングは適用しない
/// （回収対象は多くても数十件で、全件見せるのが目的のため）。
async fn outstanding_list(
    State(payments): State<PaymentsService>,
    Json(_params): Json<ListParams>,
) -> Result<Json<ListResult<InvoiceSettlement>>, ApiError> {
    let rows = payments.outstanding().await?;
    let total_count = rows.len() as u64;
    Ok(Json(ListResult { rows, total_count }))
}

#[derive(Clone)]
struct PaymentsWriteState {
    payments: PaymentsService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn payments_create(
    State(state): State<PaymentsWriteState>,
    headers: HeaderMap,
    Json(input): Json<PaymentInput>,
) -> Result<Json<PaymentDetail>, ApiError> {
    let detail = state.payments.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "payments",
        Some(&detail.payment.id.to_string()),
        Some(json!({
            "customerId": detail.payment.customer_id,
            "amount": detail.payment.amount,
            "allocations": detail.allocations.len(),
        })),
    )
    .await;
    Ok(Json(detail))
}

async fn payments_update(
    State(state): State<PaymentsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<PaymentInput>,
) -> Result<Json<PaymentDetail>, ApiError> {
    let detail = state.payments.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "payments",
        Some(&detail.payment.id.to_string()),
        Some(json!({
            "customerId": detail.payment.customer_id,
            "amount": detail.payment.amount,
            "allocations": detail.allocations.len(),
        })),
    )
    .await;
    Ok(Json(detail))
}

async fn payments_delete(
    State(state): State<PaymentsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.payments.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "payments",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn payments_read_router(payments: PaymentsService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/payments/list", post(payments_list))
        .route("/api/payments/{id}", get(payments_get))
        .route("/api/settlements/{id}", get(settlements_get))
        .route("/api/outstanding/list", post(outstanding_list))
        .with_state(payments)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

fn payments_write_router(
    payments: PaymentsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = PaymentsWriteState {
        payments,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/payments", post(payments_create))
        .route(
            "/api/payments/{id}",
            axum::routing::put(payments_update).delete(payments_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "payments",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/payments/*` と、導出値の読み取り（`/api/settlements/{id}` /
/// `/api/outstanding/list`）。
pub(super) fn payments_router(
    payments: PaymentsService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    payments_read_router(payments.clone(), auth.clone())
        .merge(payments_write_router(payments, audit, auth))
}
