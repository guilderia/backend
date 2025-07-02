use guilderia_database::util::reference::Reference;
use guilderia_database::{Database, User};
use guilderia_models::v0;
use guilderia_result::{create_error, Result};
use rocket::serde::json::Json;
use rocket::State;

/// # Deny Friend Request / Remove Friend
///
/// Denies another user's friend request or removes an existing friend.
#[openapi(tag = "Relationships")]
#[delete("/<target>/friend")]
pub async fn remove(
    db: &State<Database>,
    mut user: User,
    target: Reference,
) -> Result<Json<v0::User>> {
    let mut target = target.as_user(db).await?;

    if user.bot.is_some() || target.bot.is_some() {
        return Err(create_error!(IsBot));
    }

    user.remove_friend(db, &mut target).await?;
    Ok(Json(target.into(db, &user).await))
}
