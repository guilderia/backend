use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0;
use guilderia_permissions::PermissionQuery;
use guilderia_result::{create_error, Result};
use rocket::{serde::json::Json, State};

/// # Fetch Server Emoji
///
/// Fetch all emoji on a server.
#[openapi(tag = "Server Customisation")]
#[get("/<target>/emojis")]
pub async fn list_emoji(
    db: &State<Database>,
    user: User,
    target: Reference,
) -> Result<Json<Vec<v0::Emoji>>> {
    let server = target.as_server(db).await?;
    let mut query = DatabasePermissionQuery::new(db, &user).server(&server);
    if !query.are_we_a_member().await {
        return Err(create_error!(NotFound));
    }

    // Fetch all emoji from server if we can view it
    db.fetch_emoji_by_parent_id(&server.id)
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map(Json)
}
