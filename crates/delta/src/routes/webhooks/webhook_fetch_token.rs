use guilderia_database::{util::reference::Reference, Database};
use guilderia_models::v0::Webhook;
use guilderia_result::Result;
use rocket::{serde::json::Json, State};

/// # Gets a webhook
///
/// Gets a webhook with a token
#[openapi(tag = "Webhooks")]
#[get("/<webhook_id>/<token>")]
pub async fn webhook_fetch_token(
    db: &State<Database>,
    webhook_id: Reference,
    token: String,
) -> Result<Json<Webhook>> {
    let webhook = webhook_id.as_webhook(db).await?;
    webhook.assert_token(&token)?;
    Ok(Json(webhook.into()))
}
