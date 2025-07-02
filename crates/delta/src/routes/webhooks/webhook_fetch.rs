use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0::{ResponseWebhook, Webhook};
use guilderia_permissions::{calculate_channel_permissions, ChannelPermission};
use guilderia_result::Result;
use rocket::{serde::json::Json, State};

/// # Gets a webhook
///
/// Gets a webhook
#[openapi(tag = "Webhooks")]
#[get("/<webhook_id>")]
pub async fn webhook_fetch(
    db: &State<Database>,
    webhook_id: Reference,
    user: User,
) -> Result<Json<ResponseWebhook>> {
    let webhook = webhook_id.as_webhook(db).await?;
    let channel = db.fetch_channel(&webhook.channel_id).await?;

    let mut query = DatabasePermissionQuery::new(db, &user).channel(&channel);
    calculate_channel_permissions(&mut query)
        .await
        .throw_if_lacking_channel_permission(ChannelPermission::ViewChannel)?;

    Ok(Json(std::convert::Into::<Webhook>::into(webhook).into()))
}
