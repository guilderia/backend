use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0;
use guilderia_permissions::{calculate_channel_permissions, ChannelPermission};
use guilderia_result::{create_error, Result};
use rocket::{serde::json::Json, State};

/// # Fetch Message
///
/// Retrieves a message by its id.
#[openapi(tag = "Messaging")]
#[get("/<target>/messages/<msg>")]
pub async fn fetch(
    db: &State<Database>,
    user: User,
    target: Reference,
    msg: Reference,
) -> Result<Json<v0::Message>> {
    let channel = target.as_channel(db).await?;
    let mut query = DatabasePermissionQuery::new(db, &user).channel(&channel);
    calculate_channel_permissions(&mut query)
        .await
        .throw_if_lacking_channel_permission(ChannelPermission::ViewChannel)?;

    let message = msg.as_message(db).await?;
    if message.channel != channel.id() {
        return Err(create_error!(NotFound));
    }

    Ok(Json(message.into_model(None, None)))
}
