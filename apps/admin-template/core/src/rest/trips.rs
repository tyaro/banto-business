use super::*;

/// 出張の読み取り（conventions §1: 読み取りは認証のみ・監査しない）。
async fn trips_list(
    State(trips): State<TripsService>,
    Json(params): Json<ListParams>,
) -> Result<Json<ListResult<Trip>>, ApiError> {
    Ok(Json(trips.list(params).await?))
}

async fn trips_get(
    State(trips): State<TripsService>,
    Path(id): Path<i64>,
) -> Result<Json<Trip>, ApiError> {
    Ok(Json(trips.get(id).await?))
}

/// この出張に紐づく工数・経費の件数（削除前の確認表示用。要件 F-T3）。
async fn trips_linked_counts(
    State(trips): State<TripsService>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (work_logs, expenses) = trips.linked_record_counts(id).await?;
    Ok(Json(json!({ "workLogs": work_logs, "expenses": expenses })))
}

#[derive(Clone)]
struct TripsWriteState {
    trips: TripsService,
    audit: AuditLogService,
    auth: AuthState,
}

/// 出張の登録。`generate` を伴うと工数・経費を一括生成する（要件 F-T1）。
/// 監査の `detail` に生成件数を残すのは、後から「この出張で何行作られたか」
/// を追えるようにするため（生成物は個別編集できるので、作成時点の内訳が
/// 分からないと差分を追えない）。
async fn trips_create(
    State(state): State<TripsWriteState>,
    headers: HeaderMap,
    Json(input): Json<TripInput>,
) -> Result<Json<TripGenerationResult>, ApiError> {
    let result = state.trips.create(input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "create",
        "trips",
        Some(&result.trip.id.to_string()),
        Some(json!({
            "destination": result.trip.destination,
            "generatedWorkLogs": result.travel_work_logs + result.onsite_work_logs,
            "generatedExpenses": result.expenses,
        })),
    )
    .await;
    Ok(Json(result))
}

async fn trips_update(
    State(state): State<TripsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<TripInput>,
) -> Result<Json<Trip>, ApiError> {
    let trip = state.trips.update(id, input).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "update",
        "trips",
        Some(&trip.id.to_string()),
        Some(json!({ "destination": trip.destination })),
    )
    .await;
    Ok(Json(trip))
}

/// 出張の削除。生成物は消さず `trip_id` を NULL 化する（要件 F-T3）。
/// 監査には切り離した件数を残す。
async fn trips_delete(
    State(state): State<TripsWriteState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let (work_logs, expenses) = state.trips.linked_record_counts(id).await?;
    state.trips.delete(id).await?;
    record_write(
        &state.audit,
        &state.auth,
        &headers,
        "delete",
        "trips",
        Some(&id.to_string()),
        Some(json!({ "detachedWorkLogs": work_logs, "detachedExpenses": expenses })),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn trips_read_router(trips: TripsService, auth: AuthState) -> Router {
    Router::new()
        .route("/api/trips/list", post(trips_list))
        .route("/api/trips/{id}", get(trips_get))
        .route("/api/trips/{id}/linked-counts", get(trips_linked_counts))
        .with_state(trips)
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

fn trips_write_router(trips: TripsService, audit: AuditLogService, auth: AuthState) -> Router {
    let state = TripsWriteState {
        trips,
        audit: audit.clone(),
        auth: auth.clone(),
    };
    Router::new()
        .route("/api/trips", post(trips_create))
        .route(
            "/api/trips/{id}",
            axum::routing::put(trips_update).delete(trips_delete),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            RoleGuard {
                auth: auth.clone(),
                min: Role::Editor,
                resource: "trips",
                audit,
            },
            require_role_at_least,
        ))
        .layer(middleware::from_fn_with_state(auth, require_auth))
}

pub(super) fn trips_router(trips: TripsService, audit: AuditLogService, auth: AuthState) -> Router {
    trips_read_router(trips.clone(), auth.clone()).merge(trips_write_router(trips, audit, auth))
}
