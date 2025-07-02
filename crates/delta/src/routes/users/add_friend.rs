use guilderia_database::util::reference::Reference;
use guilderia_database::{Database, User, AMQP};
use guilderia_models::v0;
use guilderia_result::{create_error, Result};
use rocket::serde::json::Json;
use rocket::State;

/// # Accept Friend Request
///
/// Accept another user's friend request.
#[openapi(tag = "Relationships")]
#[put("/<target>/friend")]
pub async fn add(
    db: &State<Database>,
    amqp: &State<AMQP>,
    mut user: User,
    target: Reference,
) -> Result<Json<v0::User>> {
    let mut target = target.as_user(db).await?;

    if user.bot.is_some() || target.bot.is_some() {
        return Err(create_error!(IsBot));
    }

    user.add_friend(db, amqp, &mut target).await?;
    Ok(Json(target.into(db, &user).await))
}
