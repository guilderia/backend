use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0::Webhook;
use guilderia_permissions::{calculate_channel_permissions, ChannelPermission};
use guilderia_result::Result;
use rocket::{serde::json::Json, State};

/// # Gets all webhooks
///
/// Gets all webhooks inside the channel
#[openapi(tag = "Webhooks")]
#[get("/<channel_id>/webhooks")]
pub async fn fetch_webhooks(
    db: &State<Database>,
    user: User,
    channel_id: Reference,
) -> Result<Json<Vec<Webhook>>> {
    let channel = channel_id.as_channel(db).await?;

    let mut query = DatabasePermissionQuery::new(db, &user).channel(&channel);
    calculate_channel_permissions(&mut query)
        .await
        .throw_if_lacking_channel_permission(ChannelPermission::ManageWebhooks)?;

    Ok(Json(
        db.fetch_webhooks_for_channel(channel.id())
            .await?
            .into_iter()
            .map(|v| v.into())
            .collect::<Vec<Webhook>>(),
    ))
}
