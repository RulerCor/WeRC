use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    selection: werc_domain::live::LiveCalendarSelection,
    request_id: String,
}
pub(super) async fn page(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<werc_application::live_calendar::CalendarPage>, ApiError> {
    p.require("messages:read")?;
    p.require_account(&input.account_id)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.live_calendar_local(
            &input.account_id,
            &input.view_id,
            &input.selection,
            &request,
        )
    })
    .await?;
    let principal = app.authenticate(&secret)?;
    principal.require("messages:read")?;
    principal.require_account(&account)?;
    Ok(Json(page))
}
