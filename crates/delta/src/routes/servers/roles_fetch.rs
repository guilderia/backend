use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0;
use guilderia_permissions::PermissionQuery;
use guilderia_result::{create_error, Result};
use rocket::{serde::json::Json, State};

/// # Fetch Role
///
/// Fetch a role by its id.
#[openapi(tag = "Server Permissions")]
#[get("/<target>/roles/<role_id>")]
pub async fn fetch(
    db: &State<Database>,
    user: User,
    target: Reference,
    role_id: String,
) -> Result<Json<v0::Role>> {
    let mut server = target.as_server(db).await?;
    let mut query = DatabasePermissionQuery::new(db, &user).server(&server);
    if !query.are_we_a_member().await {
        return Err(create_error!(NotFound));
    }

    let role = server.roles.remove(&role_id);

    if let Some(role) = role {
        Ok(Json(role.into()))
    } else {
        Err(create_error!(NotFound))
    }
}
