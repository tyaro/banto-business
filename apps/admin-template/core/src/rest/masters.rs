use super::*;

/// マスタの読み取り（作業分類・経費分類）。`ListParams` を取らないのは
/// 10 件程度の固定コード表だから（`masters.rs` の doc コメント参照）。
async fn work_categories_list(
    State(masters): State<MastersService>,
    Json(_params): Json<ListParams>,
) -> Result<Json<ListResult<WorkCategory>>, ApiError> {
    // 固定コード表なので絞り込み・ページングは適用しない。`ListParams` を
    // 受けるのは DataProvider の `getList` 契約に合わせるためだけ
    // （packages/admin-core の命名規約: `${resource}_list`）。
    let rows = masters.list_work_categories().await?;
    let total_count = rows.len() as u64;
    Ok(Json(ListResult { rows, total_count }))
}

async fn expense_categories_list(
    State(masters): State<MastersService>,
    Json(_params): Json<ListParams>,
) -> Result<Json<ListResult<ExpenseCategory>>, ApiError> {
    let rows = masters.list_expense_categories().await?;
    let total_count = rows.len() as u64;
    Ok(Json(ListResult { rows, total_count }))
}

#[derive(Clone)]
struct MastersWriteState {
    masters: MastersService,
    audit: AuditLogService,
    auth: AuthState,
}

/// 内部原価レートの設定（upsert）。**採算計算はこのテーブルを参照しない**
/// （CLAUDE.md 1.2）ので、ここを変えても過去の工数原価は動かない。
async fn cost_rates_update(
    State(state): State<MastersWriteState>,
    headers: HeaderMap,
    Path(code): Path<String>,
    Json(input): Json<CostRateValues>,
) -> Result<Json<WorkCategory>, ApiError> {
    let updated = state
        .masters
        .set_cost_rate(CostRateInput {
            work_category_code: code,
            hourly_rate: input.hourly_rate,
        })
        .await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "cost_rates",
        Some(&updated.code),
        Some(json!({ "hourlyRate": updated.hourly_rate })),
    )
    .await;
    Ok(Json(updated))
}

fn masters_read_router(masters: MastersService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/work_categories/list", post(work_categories_list))
        .route(
            "/api/expense_categories/list",
            post(expense_categories_list),
        )
        .with_state(masters)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

/// レート設定は `editor` 以上（他の mutating と同じ床。conventions §1）。
fn masters_write_router(
    masters: MastersService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    let state = MastersWriteState {
        masters,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route(
            "/api/cost_rates/{id}",
            axum::routing::put(cost_rates_update),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "cost_rates",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

pub(super) fn masters_router(
    masters: MastersService,
    audit: AuditLogService,
    auth: AuthState,
) -> Router {
    masters_read_router(masters.clone(), auth.clone())
        .merge(masters_write_router(masters, audit, auth))
}
