use guilderia_database::util::reference::Reference;
use guilderia_database::{Database, User};
use guilderia_models::v0;
use guilderia_result::Result;
use rocket::serde::json::Json;
use rocket::State;

/// # Unblock User
///
/// Unblock another user by their id.
#[openapi(tag = "Relationships")]
#[delete("/<target>/block")]
pub async fn unblock(
    db: &State<Database>,
    mut user: User,
    target: Reference,
) -> Result<Json<v0::User>> {
    let mut target = target.as_user(db).await?;

    user.unblock_user(db, &mut target).await?;
    Ok(Json(target.into(db, &user).await))
}
