use super::*;
use werc_application::live_contacts::LiveContactPage;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    search_query: Option<String>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn authorize(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("contacts:read")?;
    p.require_account(account)?;
    Ok(())
}
fn request_key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效").into());
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("live-contacts", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn group_names(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require("group-names:read")?;
    p.require_account(&query.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = query.account_id;
    let source_account = account.clone();
    let request = Uuid::new_v4().to_string();
    let names = local_work(state.application, move |app| {
        app.group_names_local(&source_account, &request)
    })
    .await?;
    let current = app.authenticate(&secret)?;
    current.require("group-names:read")?;
    current.require_account(&account)?;
    Ok(Json(json!({"account_id": account, "data": names})))
}

pub(super) async fn friends(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require("friends:read")?;
    p.require_account(&query.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = query.account_id;
    let source_account = account.clone();
    let request = Uuid::new_v4().to_string();
    let friends = local_work(state.application, move |app| {
        app.directory_friends_local(&source_account, &request)
    }).await?;
    let current = app.authenticate(&secret)?;
    current.require("friends:read")?;
    current.require_account(&account)?;
    Ok(Json(json!({"account_id": account, "data_mode": "local_friend_directory", "data": friends})))
}

pub(super) async fn directory(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    const SCOPE: &str = "directory:read";
    p.require(SCOPE)?;
    p.require_account(&query.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = query.account_id;
    let source_account = account.clone();
    let request = Uuid::new_v4().to_string();
    let contacts = local_work(state.application, move |app| {
        app.directory_contacts_local(&source_account, &request)
    }).await?;
    let current = app.authenticate(&secret)?;
    current.require(SCOPE)?;
    current.require_account(&account)?;
    Ok(Json(json!({"account_id": account, "data_mode": "local_categorized_directory", "data": contacts})))
}

pub(super) async fn page(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<LiveContactPage>, ApiError> {
    authorize(&p, &input.account_id)?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.live_contacts_search_local(
            &input.account_id,
            &input.view_id,
            input.cursor.as_deref(),
            input.search_query.as_deref(),
            &request,
        )
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(page))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(input): Json<Cancel>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&p, &input.account_id)?;
    state.application.cancel_browse_local(&request_key(
        &p,
        &input.account_id,
        &input.request_id,
    )?)?;
    Ok(Json(json!({"cancel_requested":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AvatarInput {account_id:String,view_id:String,username:String,request_id:String}
pub(super) async fn avatar(State(state):State<ApiState>,axum::Extension(p):axum::Extension<Principal>,headers:HeaderMap,Json(input):Json<AvatarInput>)->Result<axum::response::Response,ApiError>{
    use axum::response::IntoResponse;
    authorize(&p,&input.account_id)?;p.require("media:read")?;
    let request=request_key(&p,&input.account_id,&input.request_id)?;
    let secret=bearer(&headers)?.to_owned();let account=input.account_id.clone();let app=state.application.clone();
    let avatar=local_work(state.application,move|app|app.contact_avatar_local(&input.account_id,&input.view_id,&input.username,&request)).await?;
    let current=app.authenticate(&secret)?;authorize(&current,&account)?;current.require("media:read")?;
    let bytes=avatar.bytes()?;
    Ok(([("content-type","image/png"),("cache-control","no-store"),("x-content-type-options","nosniff")],bytes.to_vec()).into_response())
}
