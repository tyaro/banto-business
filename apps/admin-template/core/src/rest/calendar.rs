use super::*;

/// 月カレンダーの読み取り（conventions §1: 読み取りは認証のみ・監査しない）。
///
/// 書き込みルートは存在しない — 集計値は保持せず常に導出するので、変更できる
/// 対象そのものが無い（`profitability` と同じ）。
///
/// `ListParams` を取るのは `DataProvider.getList` の契約に合わせるためで、
/// 使うのは `month` フィルタだけ。並び替えとページングは適用しない — 行は
/// 日付順に固定で、1か月ぶんは最大31行しかない。
async fn calendar_list(
    State(calendar): State<CalendarService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<CalendarDay>>, ApiError> {
    // 月が無い／読めない指定は 422 にする。既定で「今月」に倒すと、
    // フロントの指定漏れが黙って別の月を返す形で表に出る。
    let month = month_from_params(&params).ok_or_else(|| {
        ApiError::from(BantoError::Validation {
            field_errors: vec![banto_core::FieldError {
                field: MONTH_FILTER.to_string(),
                message: "month filter is required (YYYY-MM)".to_string(),
            }],
        })
    })?;
    let rows = calendar.month(&month).await?;
    let total_count = rows.len() as u64;
    Ok(Json(ListResult { rows, total_count }))
}

/// `/api/calendar/list`（任意のロール）。
pub(super) fn calendar_router(calendar: CalendarService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/calendar/list", post(calendar_list))
        .with_state(calendar)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}
