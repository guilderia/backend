use guilderia_database::util::permissions::DatabasePermissionQuery;
use guilderia_database::util::reference::Reference;
use guilderia_database::{Database, User};
use guilderia_models::v0;

use guilderia_permissions::{calculate_user_permissions, UserPermission};
use guilderia_result::{create_error, Result};
use rocket::serde::json::Json;
use rocket::State;

/// # Fetch Mutual Friends And Servers
///
/// Retrieve a list of mutual friends and servers with another user.
#[openapi(tag = "Relationships")]
#[get("/<target>/mutual")]
pub async fn mutual(
    db: &State<Database>,
    user: User,
    target: Reference,
) -> Result<Json<v0::MutualResponse>> {
    if target.id == user.id {
        return Err(create_error!(InvalidOperation));
    }

    let target = target.as_user(db).await?;

    let mut query = DatabasePermissionQuery::new(db, &user).user(&target);
    calculate_user_permissions(&mut query)
        .await
        .throw_if_lacking_user_permission(UserPermission::ViewProfile)?;

    Ok(Json(v0::MutualResponse {
        users: db.fetch_mutual_user_ids(&user.id, &target.id).await?,
        servers: db.fetch_mutual_server_ids(&user.id, &target.id).await?,
    }))
}
