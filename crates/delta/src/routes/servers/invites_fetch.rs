use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0;
use guilderia_permissions::{calculate_server_permissions, ChannelPermission};
use guilderia_result::Result;
use rocket::{serde::json::Json, State};

/// # Fetch Invites
///
/// Fetch all server invites.
#[openapi(tag = "Server Members")]
#[get("/<target>/invites")]
pub async fn invites(
    db: &State<Database>,
    user: User,
    target: Reference,
) -> Result<Json<Vec<v0::Invite>>> {
    let server = target.as_server(db).await?;
    let mut query = DatabasePermissionQuery::new(db, &user).server(&server);
    calculate_server_permissions(&mut query)
        .await
        .throw_if_lacking_channel_permission(ChannelPermission::ManageServer)?;

    db.fetch_invites_for_server(&server.id)
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
        .map(Json)
}
