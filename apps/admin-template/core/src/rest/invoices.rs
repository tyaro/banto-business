use super::*;

/// 請求の読み取り（conventions §1: 読み取りは認証のみ・監査しない）。
async fn invoices_list(
    State(invoices): State<InvoicesService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Invoice>>, ApiError> {
    Ok(Json(invoices.list(params).await?))
}

async fn invoices_get(
    State(invoices): State<InvoicesService>,
    Path(id): Path<i64>,
) -> Result<Json<InvoiceDetail>, ApiError> {
    Ok(Json(invoices.get(id).await?))
}

/// 未請求の工数・経費から明細候補を作る（要件 F-I1）。読み取りなので監査せず、
/// body を使うため POST（`*_list` と同じ扱い）。
async fn invoices_candidates(
    State(invoices): State<InvoicesService>,
    Json(query): Json<CandidateQuery>,
) -> Result<Json<Vec<CandidateLine>>, ApiError> {
    Ok(Json(invoices.candidates(query).await?))
}

#[derive(Clone)]
struct InvoicesWriteState {
    invoices: InvoicesService,
    audit: AuditLogService,
    auth: AuthState,
}

async fn invoices_create(
    State(state): State<InvoicesWriteState>,
    headers: HeaderMap,
    Json(input): Json<InvoiceInput>,
) -> Result<Json<InvoiceDetail>, ApiError> {
    let detail = state.invoices.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "invoices",
        Some(&detail.invoice.id.to_string()),
        Some(json!({
            "customerId": detail.invoice.customer_id,
            "lines": detail.lines.len(),
        })),
    )
    .await;
    Ok(Json(detail))
}

async fn invoices_update(
    State(state): State<InvoicesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<InvoiceInput>,
) -> Result<Json<InvoiceDetail>, ApiError> {
    let detail = state.invoices.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "invoices",
        Some(&detail.invoice.id.to_string()),
        Some(json!({
            "customerId": detail.invoice.customer_id,
            "lines": detail.lines.len(),
        })),
    )
    .await;
    Ok(Json(detail))
}

async fn invoices_delete(
    State(state): State<InvoicesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.invoices.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "invoices",
        Some(&id.to_string()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 確定（要件 F-I7）。監査の detail に採番した番号と合計を残す — 確定は
/// 番号を消費する不可逆な操作なので、いつ何番を出したかを追えるようにする。
async fn invoices_issue(
    State(state): State<InvoicesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<InvoiceDetail>, ApiError> {
    let detail = state.invoices.issue(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "issue",
        "invoices",
        Some(&detail.invoice.id.to_string()),
        Some(json!({
            "invoiceNumber": detail.invoice.invoice_number,
            "totalAmount": detail.invoice.total_amount,
            "issuedOn": detail.invoice.issued_on,
        })),
    )
    .await;
    Ok(Json(detail))
}

/// 取消（赤伝。決定 C-10）。
async fn invoices_cancel(
    State(state): State<InvoicesWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<InvoiceDetail>, ApiError> {
    let detail = state.invoices.cancel(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "cancel",
        "invoices",
        Some(&detail.invoice.id.to_string()),
        Some(json!({ "invoiceNumber": detail.invoice.invoice_number })),
    )
    .await;
    Ok(Json(detail))
}

fn invoices_read_router(invoices: InvoicesService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/invoices/list", post(invoices_list))
        .route("/api/invoices/candidates", post(invoices_candidates))
        .route("/api/invoices/{id}", get(invoices_get))
        .with_state(invoices)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

fn invoices_write_router(
    invoices: InvoicesService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = InvoicesWriteState {
        invoices,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/invoices", post(invoices_create))
        .route(
            "/api/invoices/{id}",
            axum::routing::put(invoices_update).delete(invoices_delete),
        )
        .route("/api/invoices/{id}/issue", post(invoices_issue))
        .route("/api/invoices/{id}/cancel", post(invoices_cancel))
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "invoices",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// `/api/invoices/*`。`/api/invoices/candidates` は読み取り側の POST なので、
/// `{id}` のパスより先に登録して数値以外のセグメントが id 扱いにならないよう
/// にする（axum は静的セグメントを優先するが、意図を明示するため順序も揃える）。
pub(super) fn invoices_router(
    invoices: InvoicesService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    invoices_read_router(invoices.clone(), auth.clone())
        .merge(invoices_write_router(invoices, audit, auth))
}
