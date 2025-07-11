use guilderia_database::{
    util::{permissions::DatabasePermissionQuery, reference::Reference},
    Database, User,
};
use guilderia_models::v0;

use guilderia_permissions::{calculate_user_permissions, UserPermission};
use guilderia_result::Result;
use rocket::{serde::json::Json, State};

/// # Fetch User
///
/// Retrieve a user's information.
#[openapi(tag = "User Information")]
#[get("/<target>")]
pub async fn fetch(db: &State<Database>, user: User, target: Reference) -> Result<Json<v0::User>> {
    if user.id == target.id {
        return Ok(Json(user.into_self(false).await));
    }

    let target = target.as_user(db).await?;

    let mut query = DatabasePermissionQuery::new(db, &user).user(&target);
    calculate_user_permissions(&mut query)
        .await
        .throw_if_lacking_user_permission(UserPermission::Access)?;

    Ok(Json(target.into(db, &user).await))
}
