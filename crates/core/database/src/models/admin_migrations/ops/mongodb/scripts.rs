use std::{collections::HashSet, ops::BitXor, time::Duration};

use crate::{
    mongodb::{
        bson::{doc, from_bson, from_document, to_document, Bson, DateTime, Document},
        options::FindOptions,
    },
    AbstractChannels, AbstractServers, Channel, Invite, MongoDb, User, DISCRIMINATOR_SEARCH_SPACE,
};
use bson::{oid::ObjectId, to_bson};
use futures::StreamExt;
use iso8601_timestamp::Timestamp;
use rand::seq::SliceRandom;
use guilderia_permissions::DEFAULT_WEBHOOK_PERMISSIONS;
use guilderia_result::{Error, ErrorType};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Serialize, Deserialize)]
struct MigrationInfo {
    _id: i32,
    revision: i32,
}

pub const LATEST_REVISION: i32 = 41; // MUST BE +1 to last migration

pub async fn migrate_database(db: &MongoDb) {
    let migrations = db.col::<Document>("migrations");
    let data = migrations
        .find_one(doc! {})
        .await
        .expect("Failed to fetch migration data.");

    if let Some(doc) = data {
        let info: MigrationInfo =
            from_document(doc).expect("Failed to read migration information.");

        let revision = run_migrations(db, info.revision).await;

        migrations
            .update_one(
                doc! {
                    "_id": info._id
                },
                doc! {
                    "$set": {
                        "revision": revision
                    }
                },
            )
            .await
            .expect("Failed to commit migration information.");

        info!("Migration complete. Currently at revision {}.", revision);
    } else {
        panic!("Database was configured incorrectly, possibly because initalization failed.")
    }
}

pub async fn run_migrations(db: &MongoDb, revision: i32) -> i32 {
    info!("Starting database migration.");

    if revision <= 0 {
        info!("Running migration [revision 0]: Test migration system.");
    }

    if revision <= 1 {
        info!("Running migration [revision 1 / 2021-04-24]: Migrate to Autumn v1.0.0.");

        let messages = db.col::<Document>("messages");
        let attachments = db.col::<Document>("attachments");

        messages
            .update_many(
                doc! { "attachment": { "$exists": 1_i32 } },
                doc! { "$set": { "attachment.tag": "attachments", "attachment.size": 0_i32 } },
            )
            .await
            .expect("Failed to update messages.");

        attachments
            .update_many(
                doc! {},
                doc! { "$set": { "tag": "attachments", "size": 0_i32 } },
            )
            .await
            .expect("Failed to update attachments.");
    }

    if revision <= 2 {
        info!("Running migration [revision 2 / 2021-05-08]: Add servers collection.");

        db.db()
            .create_collection("servers")
            .await
            .expect("Failed to create servers collection.");
    }

    if revision <= 3 {
        info!("Running migration [revision 3 / 2021-05-25]: Support multiple file uploads, add channel_unreads and user_settings.");

        let messages = db.col::<Document>("messages");
        let mut cursor = messages
            .find(doc! {
                "attachment": {
                    "$exists": 1_i32
                }
            })
            .with_options(
                FindOptions::builder()
                    .projection(doc! {
                        "_id": 1_i32,
                        "attachments": [ "$attachment" ]
                    })
                    .build(),
            )
            .await
            .expect("Failed to fetch messages.");

        while let Some(result) = cursor.next().await {
            let doc = result.unwrap();
            let id = doc.get_str("_id").unwrap();
            let attachments = doc.get_array("attachments").unwrap();

            messages
                .update_one(
                    doc! { "_id": id },
                    doc! { "$unset": { "attachment": 1_i32 }, "$set": { "attachments": attachments } },
                )
                .await
                .unwrap();
        }

        db.db()
            .create_collection("channel_unreads")
            .await
            .expect("Failed to create channel_unreads collection.");

        db.db()
            .create_collection("user_settings")
            .await
            .expect("Failed to create user_settings collection.");
    }

    if revision <= 4 {
        info!("Running migration [revision 4 / 2021-06-01]: Add more server collections.");

        db.db()
            .create_collection("server_members")
            .await
            .expect("Failed to create server_members collection.");

        db.db()
            .create_collection("server_bans")
            .await
            .expect("Failed to create server_bans collection.");

        db.db()
            .create_collection("channel_invites")
            .await
            .expect("Failed to create channel_invites collection.");
    }

    if revision <= 5 {
        info!("Running migration [revision 5 / 2021-06-26]: Add permissions.");

        #[derive(Serialize)]
        struct Server {
            pub default_permissions: (i32, i32),
        }

        let server = Server {
            default_permissions: (0_i32, 0_i32),
        };

        db.col::<Document>("servers")
            .update_many(
                doc! {},
                doc! {
                    "$set": to_document(&server).unwrap()
                },
            )
            .await
            .expect("Failed to migrate servers.");
    }

    if revision <= 6 {
        info!("Running migration [revision 6 / 2021-07-09]: Add message text index.");

        db.db()
            .run_command(doc! {
                "createIndexes": "messages",
                "indexes": [
                    {
                        "key": {
                            "content": "text"
                        },
                        "name": "content"
                    }
                ]
            })
            .await
            .expect("Failed to create message index.");
    }

    if revision <= 7 {
        info!("Running migration [revision 7 / 2021-08-11]: Add message text index.");

        db.db()
            .create_collection("bots")
            .await
            .expect("Failed to create bots collection.");
    }

    if revision <= 8 {
        info!("Running migration [revision 8 / 2021-09-10]: Update to Authifier version 1.");

        db.db()
            .run_command(doc! {
                "dropIndexes": "accounts",
                "index": ["email", "email_normalised"]
            })
            .await
            .expect("Failed to delete legacy account indexes.");

        let col = db.col::<Document>("sessions");
        let mut cursor = db.col::<Document>("accounts").find(doc! {}).await.unwrap();

        while let Some(doc) = cursor.next().await {
            if let Ok(account) = doc {
                let id = account.get_str("_id").unwrap();
                if let Some(sessions) = account.get("sessions") {
                    #[derive(Deserialize)]
                    struct Session {
                        id: String,
                        token: String,
                        friendly_name: String,
                        subscription: Option<Document>,
                    }

                    let sessions = from_bson::<Vec<Session>>(sessions.clone()).unwrap();
                    for session in sessions {
                        info!("Converting session {} to new format.", &session.id);

                        let mut doc = doc! {
                            "_id": session.id,
                            "token": session.token,
                            "user_id": id,
                            "name": session.friendly_name,
                        };

                        if let Some(sub) = session.subscription {
                            doc.insert("subscription", sub);
                        }

                        col.insert_one(doc).await.ok();
                    }
                } else {
                    info!("Account doesn't have any sessions!");
                }
            }
        }

        db.col::<Document>("accounts")
            .update_many(
                doc! {},
                doc! {
                    "$unset": {
                        "sessions": 1_i32,
                    },
                    "$set": {
                        "mfa": {
                            "recovery_codes": []
                        }
                    }
                },
            )
            .await
            .unwrap();
    }

    if revision <= 9 {
        info!("Running migration [revision 9 / 2021-09-14]: Switch from last_message to last_message_id.");

        let mut cursor = db.col::<Document>("channels").find(doc! {}).await.unwrap();

        while let Some(doc) = cursor.next().await {
            if let Ok(channel) = doc {
                let channel_id = channel.get_str("_id").unwrap();
                if let Some(last_message) = channel.get("last_message") {
                    #[derive(Serialize, Deserialize, Debug, Clone)]
                    pub struct Obj {
                        #[serde(rename = "_id")]
                        id: String,
                    }

                    #[derive(Serialize, Deserialize, Debug, Clone)]
                    #[serde(untagged)]
                    pub enum LastMessage {
                        Obj(Obj),
                        Id(String),
                    }

                    let lm = from_bson::<LastMessage>(last_message.clone()).unwrap();
                    let id = match lm {
                        LastMessage::Obj(Obj { id }) => id,
                        LastMessage::Id(id) => id,
                    };

                    info!("Converting session {} to new format.", &channel_id);
                    db.col::<Document>("channels")
                        .update_one(
                            doc! {
                                "_id": channel_id
                            },
                            doc! {
                                "$set": {
                                    "last_message_id": id
                                },
                                "$unset": {
                                    "last_message": 1_i32,
                                }
                            },
                        )
                        .await
                        .unwrap();
                } else {
                    info!("{} has no last_message.", &channel_id);
                }
            }
        }
    }

    if revision <= 10 {
        info!("Running migration [revision 10 / 2021-11-01]: Remove nonce values on channels and servers.");

        db.col::<Document>("servers")
            .update_many(
                doc! {},
                doc! {
                    "$unset": {
                        "nonce": 1_i32,
                    }
                },
            )
            .await
            .unwrap();

        db.col::<Document>("channels")
            .update_many(
                doc! {},
                doc! {
                    "$unset": {
                        "nonce": 1_i32,
                    }
                },
            )
            .await
            .unwrap();
    }

    if revision <= 11 {
        info!("Running migration [revision 11 / 2021-11-14]: Add indexes to database.");

        db.db()
            .run_command(doc! {
                "createIndexes": "messages",
                "indexes": [
                    {
                        "key": {
                            "channel": 1_i32
                        },
                        "name": "channel"
                    }
                ]
            })
            .await
            .expect("Failed to create message index.");

        db.db()
            .run_command(doc! {
                "createIndexes": "channel_unreads",
                "indexes": [
                    {
                        "key": {
                            "_id.channel": 1_i32,
                            "_id.user": 1_i32,
                        },
                        "name": "compound_id"
                    },
                    {
                        "key": {
                            "_id.user": 1_i32,
                        },
                        "name": "user_id"
                    }
                ]
            })
            .await
            .expect("Failed to create channel_unreads index.");

        db.db()
            .run_command(doc! {
                "createIndexes": "server_members",
                "indexes": [
                    {
                        "key": {
                            "_id.server": 1_i32,
                            "_id.user": 1_i32,
                        },
                        "name": "compound_id"
                    },
                    {
                        "key": {
                            "_id.user": 1_i32,
                        },
                        "name": "user_id"
                    }
                ]
            })
            .await
            .expect("Failed to create server_members index.");
    }

    if revision <= 12 {
        info!("Running migration [revision 12 / 2021-11-21]: Add indexes to database.");

        db.db()
            .run_command(doc! {
                "createIndexes": "messages",
                "indexes": [
                    {
                        "key": {
                            "channel": 1_i32,
                            "_id": 1_i32
                        },
                        "name": "channel_id_compound"
                    }
                ]
            })
            .await
            .expect("Failed to create message index.");
    }

    if revision <= 13 {
        info!("Running migration [revision 13 / 22-02-2022]: Wipe legacy permission values.");

        warn!("This is a destructive operation and will wipe existing permission data (excl. defaults for SendMessage).");
        warn!("Taking a backup is advised.");
        warn!("Continuing in 10 seconds...");
        async_std::task::sleep(Duration::from_secs(10)).await;

        let servers = db.col::<Document>("servers");
        let mut cursor = servers.find(doc! {}).await.unwrap();

        while let Some(Ok(mut document)) = cursor.next().await {
            let id = document.get_str("_id").unwrap().to_string();
            info!("Updating server {id}");

            let mut update = doc! {};

            // Try to pluck channel permission SendMessage (0x2)
            // Structure of default_permissions used to be [server, channel]
            let has_send = document
                .get_array("default_permissions")
                .map(|x| {
                    x.get(1)
                        .map(|x| x.as_i32().map(|x| (x as u32 & 0x2) == 0x2))
                })
                .ok()
                .flatten()
                .flatten()
                .unwrap_or_default();

            update.insert(
                "default_permissions",
                // Remove Send Message permission if it wasn't originally granted
                (4000323584).bitxor(if has_send { 0 } else { (1 << 22) as u64 }) as i64,
            );

            if let Some(Bson::Document(mut roles)) = document.remove("roles") {
                for role in roles.keys().cloned().collect::<Vec<String>>() {
                    if let Some(Bson::Document(role)) = roles.get_mut(role) {
                        role.insert(
                            "permissions",
                            doc! {
                                "a": 0_i64,
                                "d": 0_i64,
                            },
                        );
                    }
                }

                update.insert("roles", roles);
            }

            servers
                .update_one(doc! { "_id": id }, doc! { "$set": update })
                .await
                .unwrap();
        }

        let channels = db.col::<Document>("channels");
        let mut cursor = channels.find(doc! {}).await.unwrap();

        while let Some(Ok(document)) = cursor.next().await {
            let id = document.get_str("_id").unwrap().to_string();
            info!("Updating channel {id}");

            let mut unset = doc! {
                "permissions": 1_i32,
                "role_permissions": 1_i32,
            };

            // Try to pluck channel permission SendMessage (0x2)
            let has_send = document
                .get_i32("default_permissions")
                .map(|x| (x as u32 & 0x2) == 0x2)
                .unwrap_or(true);

            if has_send {
                // Let parent permissions fall through.
                unset.insert("default_permissions", 1_i32);
            }

            let mut update = doc! {
                "$unset": unset
            };

            if !has_send {
                // Block send message permission.
                update.insert(
                    "$set",
                    doc! {
                        "default_permissions": {
                            "a": 0_i64,
                            "d": (1 << 22) as i64
                        }
                    },
                );
            }

            channels
                .update_one(doc! { "_id": id }, update)
                .await
                .unwrap();
        }
    }

    if revision <= 14 {
        info!("Running migration [revision 14 / 21-04-2022]: Split content into content and system fields.");

        db.col::<Document>("messages")
            .update_many(
                doc! {
                    "content": {
                        "$type": "object"
                    }
                },
                doc! {
                    "$rename": {
                        "content": "system"
                    }
                },
            )
            .await
            .unwrap();
    }

    if revision <= 15 {
        info!("Running migration [revision 15 / 04-06-2022]: Migrate Authifier to latest version.");

        let db = authifier::Database::MongoDb(authifier::database::MongoDb(db.db()));
        db.run_migration(authifier::Migration::M2022_06_03EnsureUpToSpec)
            .await
            .unwrap();
    }

    if revision <= 16 {
        info!("Running migration [revision 16 / 07-07-2022]: Add `emojis` collection and Authifier migration.");

        let authifier_db = authifier::Database::MongoDb(authifier::database::MongoDb(db.db()));
        authifier_db
            .run_migration(authifier::Migration::M2022_06_09AddIndexForDeletion)
            .await
            .unwrap();

        db.db()
            .create_collection("emojis")
            .await
            .expect("Failed to create emojis collection.");

        db.db()
            .run_command(doc! {
                "createIndexes": "emojis",
                "indexes": [
                    {
                        "key": {
                            "parent.id": 1_i32,
                        },
                        "name": "parent_id"
                    }
                ]
            })
            .await
            .expect("Failed to create emoji parent index.");
    }

    if revision <= 17 {
        info!("Running migration [revision 17 / 15-07-2022]: Initialise `joined_at` property on server members.");

        db.col::<Document>("server_members")
            .update_many(
                doc! {},
                doc! {
                    "$set": {
                        "joined_at": DateTime::now().try_to_rfc3339_string().expect("Failed to convert the date to rfc3339")
                    }
                },
            )
            .await
            .expect("Failed to update server members.");
    }

    if revision <= 18 {
        info!("Running migration [revision 18 / 27-02-2022]: Create author index on messages. Drop plain channel index if exists.");

        if db
            .db()
            .run_command(doc! {
                "dropIndexes": "messages",
                "index": ["channel"]
            })
            .await
            .is_err()
        {
            info!("Failed to drop `messages.channel` index but this is ok since that means it's probably gone.");
        }

        db.db()
            .run_command(doc! {
                "createIndexes": "messages",
                "indexes": [
                    {
                        "key": {
                            "author": 1_i32,
                        },
                        "name": "author"
                    }
                ]
            })
            .await
            .expect("Failed to create messages author index.");
    }

    if revision <= 19 {
        info!(
            "Running migration [revision 19 / 27-02-2023]: Create report / snapshot collections."
        );

        db.db().create_collection("safety_reports").await.unwrap();

        db.db().create_collection("safety_snapshots").await.unwrap();
    }

    if revision <= 20 {
        info!("Running migration [revision 20 / 28-02-2023]: Add index `snapshot.report_id`.");

        db.db()
            .run_command(doc! {
                "createIndexes": "safety_snapshots",
                "indexes": [
                    {
                        "key": {
                            "report_id": 1_i32
                        },
                        "name": "report_id"
                    }
                ]
            })
            .await
            .expect("Failed to create safety snapshot index.");
    }

    if revision <= 21 {
        info!("Running migration [revision 21 / 31-05-2023]: Add collection `safety_strikes`.");

        db.db().create_collection("safety_strikes").await.unwrap();
    }

    if revision <= 22 {
        info!("Running migration [revision 22 / 31-05-2023]: Add moderator_id to account strikes.");

        db.col::<Document>("safety_strikes")
            .update_many(
                doc! {},
                doc! {
                    "$set": {
                        "moderator_id": "01EX2NCWQ0CHS3QJF0FEQS1GR4"
                    }
                },
            )
            .await
            .expect("Failed to update server members.");
    }

    if revision <= 23 {
        info!("Running migration [revision 23 / 10-06-2023]: Generate discriminators for users.");

        db.db()
            .run_command(doc! {
                "dropIndexes": "users",
                "index": "username"
            })
            .await
            .expect("Failed to drop existing username index.");

        #[derive(Serialize, Deserialize)]
        struct UserInformation {
            #[serde(rename = "_id")]
            id: String,
            username: String,
        }

        let re_username = regex::Regex::new(r"^(\p{L}|[\d_.-])+$").unwrap();

        let users: Vec<UserInformation> = db
            .col::<UserInformation>("users")
            .find(doc! {})
            .await
            .unwrap()
            .map(|doc| doc.expect("id and username"))
            .collect()
            .await;

        let search_space: Vec<String> = DISCRIMINATOR_SEARCH_SPACE.iter().cloned().collect();
        let mut claimed: HashSet<String> = HashSet::new();

        for i in 0..users.len() {
            let info = &users[i];
            let mut discriminator = {
                let mut rng = rand::thread_rng();
                search_space.choose(&mut rng).unwrap()
            };

            if re_username.is_match(&info.username) {
                while claimed.contains(&format!("{}#{}", info.username, discriminator)) {
                    let new_discriminator = {
                        let mut rng = rand::thread_rng();
                        search_space.choose(&mut rng).unwrap()
                    };

                    info!(
                        "Re-rolled {} to {new_discriminator} from {discriminator}",
                        info.username
                    );

                    discriminator = new_discriminator;
                }

                claimed.insert(format!("{}#{}", info.username, discriminator));

                info!(
                    "({}/{}) Migrating user \"{}\" to #{} - compliant",
                    i + 1,
                    users.len(),
                    info.username,
                    discriminator
                );

                db.col::<UserInformation>("users")
                    .update_one(
                        doc! {
                            "_id": &info.id
                        },
                        doc! {
                            "$set": {
                                "discriminator": discriminator
                            }
                        },
                    )
                    .await
                    .unwrap();
            } else {
                let mut sanitised = info
                    .username
                    .graphemes(true)
                    .filter(|s| re_username.is_match(s))
                    .collect::<String>();

                while sanitised.len() < 2 {
                    sanitised += "_";
                }

                while claimed.contains(&format!("{}#{}", sanitised, discriminator)) {
                    let new_discriminator = {
                        let mut rng = rand::thread_rng();
                        search_space.choose(&mut rng).unwrap()
                    };

                    info!("Re-rolled {sanitised} to {new_discriminator} from {discriminator}");
                    discriminator = new_discriminator;
                }

                claimed.insert(format!("{}#{}", sanitised, discriminator));

                info!(
                    "({}/{}) Migrating user \"{}\" to #{} - sanitised: \"{}\"",
                    i + 1,
                    users.len(),
                    info.username,
                    discriminator,
                    sanitised
                );

                db.col::<UserInformation>("users")
                    .update_one(
                        doc! {
                            "_id": &info.id
                        },
                        doc! {
                            "$set": {
                                "username": sanitised,
                                "discriminator": discriminator,
                                "display_name": &info.username
                            }
                        },
                    )
                    .await
                    .unwrap();
            }
        }
    }

    if revision <= 24 {
        info!("Running migration [revision 24 / 09-06-2023]: Add collection `channel_webhooks` if not exists, update users index.");

        db.db().create_collection("channel_webhooks").await.ok();

        db.db()
            .run_command(doc! {
                "createIndexes": "users",
                "indexes": [
                    {
                        "key": {
                            "username": 1_i32
                        },
                        "name": "username",
                        "unique": false,
                        "collation": {
                            "locale": "en",
                            "strength": 2_i32
                        }
                    },
                    {
                        "key": {
                            "username": 1_i32,
                            "discriminator": 1_i32
                        },
                        "name": "username_discriminator",
                        "unique": true,
                        "collation": {
                            "locale": "en",
                            "strength": 2_i32
                        }
                    }
                ]
            })
            .await
            .expect("Failed to create username index.");
    };

    if revision <= 25 {
        info!("Running migration [revision 25 / 11-06-2023]: Add permissions to webhooks.");

        db.col::<Document>("webhooks")
            .update_many(
                doc! {},
                doc! {
                    "$set": {
                        "permissions": *DEFAULT_WEBHOOK_PERMISSIONS as i64
                    }
                },
            )
            .await
            .expect("Failed to update webhooks.");
    }

    if revision <= 25 {
        info!("Running migration [revision 25 / 15-06-2023]: Add collection `ratelimit_events` with index.");

        db.db().create_collection("ratelimit_events").await.ok();

        db.db()
            .run_command(doc! {
                "createIndexes": "ratelimit_events",
                "indexes": [
                    {
                        "key": {
                            "_id": 1_i32,
                            "target_id": 1_i32,
                            "event_type": 1_i32,
                        },
                        "name": "compound_key"
                    }
                ]
            })
            .await
            .expect("Failed to create ratelimit_events index.");
    }

    if revision <= 26 {
        info!("Running migration [revision 26 / 15-05-2024]: fix invites being incorrectly serialized with wrong enum tagging.");

        auto_derived!(
            pub enum OldInvite {
                Server {
                    #[serde(rename = "_id")]
                    code: String,
                    server: String,
                    creator: String,
                    channel: String,
                },
                Group {
                    #[serde(rename = "_id")]
                    code: String,
                    creator: String,
                    channel: String,
                },
            }
        );

        #[derive(serde::Serialize, serde::Deserialize)]
        struct Outer {
            _id: ObjectId,
            #[serde(flatten)]
            invite: OldInvite,
        }

        let invites = db
            .db()
            .collection::<Outer>("channel_invites")
            .find(doc! {
                "type": { "$exists": false }
            })
            .await
            .expect("failed to find invites")
            .filter_map(|s| async { s.ok() })
            .collect::<Vec<Outer>>()
            .await
            .into_iter()
            .map(|invite| match invite.invite {
                OldInvite::Server {
                    code,
                    server,
                    creator,
                    channel,
                } => Invite::Server {
                    code,
                    server,
                    creator,
                    channel,
                },
                OldInvite::Group {
                    code,
                    creator,
                    channel,
                } => Invite::Group {
                    code,
                    creator,
                    channel,
                },
            })
            .collect::<Vec<Invite>>();

        if !invites.is_empty() {
            db.db()
                .collection("channel_invites")
                .insert_many(invites)
                .await
                .expect("failed to insert corrected invite");

            db.db()
                .collection::<Outer>("channel_invites")
                .delete_many(doc! {
                    "type": { "$exists": false }
                })
                .await
                .expect("failed to find invites");
        }
    }

    if revision <= 27 {
        info!("Running migration [revision 27 / 21-07-2024]: create message pinned index.");

        db.db()
            .run_command(doc! {
                "createIndexes": "messages",
                "indexes": [
                    {
                        "key": {
                            "channel": 1_i32,
                            "pinned": 1_i32
                        },
                        "name": "channel_pinned_compound"
                    }
                ]
            })
            .await
            .expect("Failed to create message index.");
    }

    if revision <= 28 {
        info!("Running migration [revision 28 / 10-09-2024]: Add support for new Autumn.");

        db.db().create_collection("attachment_hashes").await.ok();

        db.db()
            .run_command(doc! {
                "createIndexes": "attachments",
                "indexes": [
                    {
                        "key": {
                            "hash": 1_i32
                        },
                        "name": "hash"
                    }
                ]
            })
            .await
            .expect("Failed to create attachments index.");

        db.db()
            .run_command(doc! {
                "createIndexes": "attachment_hashes",
                "indexes": [
                    {
                        "key": {
                            "processed_hash": 1_i32
                        },
                        "name": "processed_hash"
                    }
                ]
            })
            .await
            .expect("Failed to create attachment_hashes index.");
    }

    // Revision 29 omitted due to bug.

    if revision <= 30 {
        info!("Running migration [revision 30 / 29-09-2024]: Add index for used_for.id to attachments.");

        db.db()
            .run_command(doc! {
                "createIndexes": "attachments",
                "indexes": [
                    {
                        "key": {
                            "used_for.id": 1_i32
                        },
                        "name": "used_for_id"
                    }
                ]
            })
            .await
            .expect("Failed to create attachments index.");
    }

    if revision <= 31 {
        info!("Running migration [revision 31 / 31-10-2024]: Add creator_id to webhooks and delete those whose channels don't exist.");

        #[derive(serde::Serialize, serde::Deserialize)]
        struct WebhookShell {
            _id: String,
            channel_id: String,
        }

        let webhooks = db
            .db()
            .collection::<WebhookShell>("channel_webhooks")
            .find(doc! {})
            .await
            .expect("webhooks")
            .filter_map(|s| async { s.ok() })
            .collect::<Vec<WebhookShell>>()
            .await;

        for webhook in webhooks {
            match db.fetch_channel(&webhook.channel_id).await {
                Ok(channel) => {
                    let creator_id = match channel {
                        Channel::Group { owner, .. } => owner,
                        Channel::TextChannel { server, .. }
                        | Channel::VoiceChannel { server, .. } => {
                            let server = db.fetch_server(&server).await.expect("server");
                            server.owner
                        }
                        _ => unreachable!("not server or group channel!"),
                    };

                    db.db()
                        .collection::<Document>("channel_webhooks")
                        .update_one(
                            doc! {
                                "_id": webhook._id,
                            },
                            doc! {
                                "$set" : {
                                    "creator_id": creator_id
                                }
                            },
                        )
                        .await
                        .expect("update webhook");
                }
                Err(Error {
                    error_type: ErrorType::NotFound,
                    ..
                }) => {
                    db.db()
                        .collection::<WebhookShell>("channel_webhooks")
                        .delete_one(doc! { "_id": webhook._id })
                        .await
                        .expect("failed to delete invalid webhook");
                }
                Err(err) => panic!("{err:?}"),
            }
        }
    }

    if revision <= 32 {
        info!(
            "Running migration [revision 32 / 12-05-2025]: (Authifier) Add last_seen to sessions."
        );

        let db = authifier::Database::MongoDb(authifier::database::MongoDb(db.db()));
        db.run_migration(authifier::Migration::M2025_02_20AddLastSeenToSession)
            .await
            .unwrap();
    }

    if revision <= 40 {
        info!(
            "Running migration [revision |> 40 / 30-05-2025]: Set last policy acknowlegement date to now and create policy changes collection."
        );

        db.db()
            .create_collection("policy_changes")
            .await
            .expect("Failed to create policy_changes collection.");

        db.db()
            .collection::<User>("users")
            .update_many(
                doc! {},
                doc! {
                    "$set": {
                        "last_acknowledged_policy_change": to_bson(&Timestamp::now_utc())
                            .expect("failed to serialise timestamp")
                    }
                },
            )
            .await
            .expect("failed to update users");
    }

    // Reminder to update LATEST_REVISION when adding new migrations.
    LATEST_REVISION.max(revision)
}
