use super::*;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Action {
    Status {
        account_id: String,
    },
    Configure {
        account_id: String,
        conversation: String,
        enabled: bool,
    },
    Step {
        account_id: String,
    },
    List {
        account_id: String,
        before: Option<i64>,
        conversation: Option<String>,
        status: Option<String>,
    },
}
pub(super) async fn handle(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let account = match &input {
        Action::Status { account_id }
        | Action::Configure { account_id, .. }
        | Action::Step { account_id }
        | Action::List { account_id, .. } => account_id.clone(),
    };
    let write = matches!(input, Action::Configure { .. } | Action::Step { .. });
    fn access(p: &Principal, a: &str, write: bool) -> Result<(), AppError> {
        p.require("messages:read")?;
        p.require_account(a)?;
        if write {
            p.require("operations:create")?;
        }
        Ok(())
    }
    let secret = bearer(&headers)?.to_owned();
    access(&state.application.authenticate(&secret)?, &account, write)?;
    let app = state.application.clone();
    let auth = secret.clone();
    let bound = account.clone();
    let result = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &bound, write)?;
        match input {
            Action::Status { account_id } => {
                serde_json::to_value(app.message_preservation_status_local(&account_id)?)
            }
            Action::Configure {
                account_id,
                conversation,
                enabled,
            } => serde_json::to_value(app.configure_message_preservation_local(
                &account_id,
                &conversation,
                enabled,
            )?),
            Action::Step { account_id } => {
                serde_json::to_value(app.step_message_preservation_local(&account_id)?)
            }
            Action::List {
                account_id,
                before,
                conversation,
                status,
            } => serde_json::to_value(app.preserved_messages_local(
                &account_id,
                before,
                conversation.as_deref(),
                status.as_deref(),
            )?),
        }
        .map_err(|_| AppError::internal())
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, write)?;
    Ok(Json(result))
}
