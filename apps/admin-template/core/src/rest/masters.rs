use super::*;

/// マスタの読み取り（作業分類・経費分類）。`ListParams` を取らないのは
/// 10 件程度の固定コード表だから（`masters.rs` の doc コメント参照）。
async fn work_categories_list(
    State(masters): State<MastersService>,
) -> Result<Json<Vec<WorkCategory>>, ApiError> {
    Ok(Json(masters.list_work_categories().await?))
}

async fn expense_categories_list(
    State(masters): State<MastersService>,
) -> Result<Json<Vec<ExpenseCategory>>, ApiError> {
    Ok(Json(masters.list_expense_categories().await?))
}

#[derive(Clone)]
struct MastersWriteState {
    masters: MastersService,
    audit: AuditLogService,
    auth: AuthState,
}

/// 内部原価レートの設定（upsert）。**採算計算はこのテーブルを参照しない**
/// （CLAUDE.md 1.2）ので、ここを変えても過去の工数原価は動かない。
async fn cost_rates_set(
    State(state): State<MastersWriteState>,
    headers: HeaderMap,
    Json(input): Json<CostRateInput>,
) -> Result<Json<WorkCategory>, ApiError> {
    let updated = state.masters.set_cost_rate(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "cost-rates",
        Some(&updated.code),
        Some(json!({ "hourlyRate": updated.hourly_rate })),
    )
    .await;
    Ok(Json(updated))
}

fn masters_read_router(masters: MastersService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/work-categories", get(work_categories_list))
        .route("/api/expense-categories", get(expense_categories_list))
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
        .route("/api/cost-rates", axum::routing::put(cost_rates_set))
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "cost-rates",
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
