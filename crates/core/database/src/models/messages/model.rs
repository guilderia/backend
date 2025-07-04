use std::{collections::HashSet, hash::RandomState};

use indexmap::{IndexMap, IndexSet};
use iso8601_timestamp::Timestamp;
use guilderia_config::{config, FeaturesLimits};
use guilderia_models::v0::{
    self, BulkMessageResponse, DataMessageSend, Embed, MessageAuthor, MessageFlags, MessageSort,
    MessageWebhook, PushNotification, ReplyIntent, SendableEmbed, Text,
};
use guilderia_permissions::{calculate_channel_permissions, ChannelPermission, PermissionValue};
use guilderia_result::{ErrorType, Result};
use ulid::Ulid;
use validator::Validate;

use crate::{
    events::client::EventV1,
    tasks::{self, ack::AckEvent},
    util::{
        bulk_permissions::BulkDatabasePermissionQuery, idempotency::IdempotencyKey,
        permissions::DatabasePermissionQuery,
    },
    Channel, Database, Emoji, File, User, AMQP,
};

auto_derived_partial!(
    /// Message
    pub struct Message {
        /// Unique Id
        #[serde(rename = "_id")]
        pub id: String,
        /// Unique value generated by client sending this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub nonce: Option<String>,
        /// Id of the channel this message was sent in
        pub channel: String,
        /// Id of the user or webhook that sent this message
        pub author: String,
        /// The webhook that sent this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub webhook: Option<MessageWebhook>,
        /// Message content
        #[serde(skip_serializing_if = "Option::is_none")]
        pub content: Option<String>,
        /// System message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub system: Option<SystemMessage>,
        /// Array of attachments
        #[serde(skip_serializing_if = "Option::is_none")]
        pub attachments: Option<Vec<File>>,
        /// Time at which this message was last edited
        #[serde(skip_serializing_if = "Option::is_none")]
        pub edited: Option<Timestamp>,
        /// Attached embeds to this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub embeds: Option<Vec<Embed>>,
        /// Array of user ids mentioned in this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub mentions: Option<Vec<String>>,
        /// Array of role ids mentioned in this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub role_mentions: Option<Vec<String>>,
        /// Array of message ids this message is replying to
        #[serde(skip_serializing_if = "Option::is_none")]
        pub replies: Option<Vec<String>>,
        /// Hashmap of emoji IDs to array of user IDs
        #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
        pub reactions: IndexMap<String, IndexSet<String>>,
        /// Information about how this message should be interacted with
        #[serde(skip_serializing_if = "Interactions::is_default", default)]
        pub interactions: Interactions,
        /// Name and / or avatar overrides for this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub masquerade: Option<Masquerade>,
        /// Whether or not the message in pinned
        #[serde(skip_serializing_if = "crate::if_option_false")]
        pub pinned: Option<bool>,

        /// Bitfield of message flags
        #[serde(skip_serializing_if = "Option::is_none")]
        pub flags: Option<u32>,
    },
    "PartialMessage"
);

auto_derived!(
    /// System Event
    #[serde(tag = "type")]
    pub enum SystemMessage {
        #[serde(rename = "text")]
        Text { content: String },
        #[serde(rename = "user_added")]
        UserAdded { id: String, by: String },
        #[serde(rename = "user_remove")]
        UserRemove { id: String, by: String },
        #[serde(rename = "user_joined")]
        UserJoined { id: String },
        #[serde(rename = "user_left")]
        UserLeft { id: String },
        #[serde(rename = "user_kicked")]
        UserKicked { id: String },
        #[serde(rename = "user_banned")]
        UserBanned { id: String },
        #[serde(rename = "channel_renamed")]
        ChannelRenamed { name: String, by: String },
        #[serde(rename = "channel_description_changed")]
        ChannelDescriptionChanged { by: String },
        #[serde(rename = "channel_icon_changed")]
        ChannelIconChanged { by: String },
        #[serde(rename = "channel_ownership_changed")]
        ChannelOwnershipChanged { from: String, to: String },
        #[serde(rename = "message_pinned")]
        MessagePinned { id: String, by: String },
        #[serde(rename = "message_unpinned")]
        MessageUnpinned { id: String, by: String },
    }

    /// Name and / or avatar override information
    pub struct Masquerade {
        /// Replace the display name shown on this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
        /// Replace the avatar shown on this message (URL to image file)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub avatar: Option<String>,
        /// Replace the display role colour shown on this message
        ///
        /// Must have `ManageRole` permission to use
        #[serde(skip_serializing_if = "Option::is_none")]
        pub colour: Option<String>,
    }

    /// Information to guide interactions on this message
    #[derive(Default)]
    pub struct Interactions {
        /// Reactions which should always appear and be distinct
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub reactions: Option<IndexSet<String>>,
        /// Whether reactions should be restricted to the given list
        ///
        /// Can only be set to true if reactions list is of at least length 1
        #[serde(skip_serializing_if = "crate::if_false", default)]
        pub restrict_reactions: bool,
    }

    /// Appended Information
    pub struct AppendMessage {
        /// Additional embeds to include in this message
        #[serde(skip_serializing_if = "Option::is_none")]
        pub embeds: Option<Vec<Embed>>,
    }

    /// Message Time Period
    ///
    /// Filter and sort messages by time
    #[serde(untagged)]
    pub enum MessageTimePeriod {
        Relative {
            /// Message id to search around
            ///
            /// Specifying 'nearby' ignores 'before', 'after' and 'sort'.
            /// It will also take half of limit rounded as the limits to each side.
            /// It also fetches the message ID specified.
            nearby: String,
        },
        Absolute {
            /// Message id before which messages should be fetched
            before: Option<String>,
            /// Message id after which messages should be fetched
            after: Option<String>,
            /// Message sort direction
            sort: Option<MessageSort>,
        },
    }

    /// Message Filter
    #[derive(Default)]
    pub struct MessageFilter {
        /// Parent channel ID
        pub channel: Option<String>,
        /// Message author ID
        pub author: Option<String>,
        /// Search query
        pub query: Option<String>,
        /// Search for pinned
        pub pinned: Option<bool>,
    }

    /// Message Query
    pub struct MessageQuery {
        /// Maximum number of messages to fetch
        ///
        /// For fetching nearby messages, this is \`(limit + 1)\`.
        pub limit: Option<i64>,
        /// Filter to apply
        #[serde(flatten)]
        pub filter: MessageFilter,
        /// Time period to fetch
        #[serde(flatten)]
        pub time_period: MessageTimePeriod,
    }

    /// Optional fields on message
    pub enum FieldsMessage {
        Pinned,
    }
);

pub struct MessageFlagsValue(pub u32);

impl MessageFlagsValue {
    pub fn has(&self, flag: MessageFlags) -> bool {
        self.has_value(flag as u32)
    }
    pub fn has_value(&self, bit: u32) -> bool {
        let mask = 1 << bit;
        self.0 & mask == mask
    }

    pub fn set(&mut self, flag: MessageFlags, toggle: bool) -> &mut Self {
        self.set_value(flag as u32, toggle)
    }
    pub fn set_value(&mut self, bit: u32, toggle: bool) -> &mut Self {
        if toggle {
            self.0 |= 1 << bit;
        } else {
            self.0 &= !(1 << bit);
        }
        self
    }
}

#[allow(clippy::derivable_impls)]
impl Default for Message {
    fn default() -> Self {
        Self {
            id: Default::default(),
            nonce: None,
            channel: Default::default(),
            author: Default::default(),
            webhook: None,
            content: None,
            system: None,
            attachments: None,
            edited: None,
            embeds: None,
            mentions: None,
            role_mentions: None,
            replies: None,
            reactions: Default::default(),
            interactions: Default::default(),
            masquerade: None,
            flags: None,
            pinned: None,
        }
    }
}

#[allow(clippy::disallowed_methods)]
impl Message {
    /// Create message from API data
    #[allow(clippy::too_many_arguments)]
    pub async fn create_from_api(
        db: &Database,
        amqp: Option<&AMQP>,
        channel: Channel,
        data: DataMessageSend,
        author: MessageAuthor<'_>,
        user: Option<v0::User>,
        member: Option<v0::Member>,
        limits: FeaturesLimits,
        mut idempotency: IdempotencyKey,
        generate_embeds: bool,
        allow_mentions: bool,
    ) -> Result<Message> {
        let config = config().await;

        Message::validate_sum(
            &data.content,
            data.embeds.as_deref().unwrap_or_default(),
            limits.message_length,
        )?;

        idempotency
            .consume_nonce(data.nonce)
            .await
            .map_err(|_| create_error!(InvalidOperation))?;

        // Check the message is not empty
        if (data.content.as_ref().is_none_or(|v| v.is_empty()))
            && (data.attachments.as_ref().is_none_or(|v| v.is_empty()))
            && (data.embeds.as_ref().is_none_or(|v| v.is_empty()))
        {
            return Err(create_error!(EmptyMessage));
        }

        let allow_mass_mentions = allow_mentions && config.features.mass_mentions_enabled;

        let mut mentions_everyone = false;
        let mut mentions_online = false;
        let mut suppress_notifications = false;

        if let Some(raw_flags) = &data.flags {
            if raw_flags > &7 {
                // quick path to failure: bigger than all the bits combined
                return Err(create_error!(InvalidProperty));
            }

            // First step of mass mention resolution
            let flags = MessageFlagsValue(*raw_flags);
            suppress_notifications = flags.has(MessageFlags::SuppressNotifications);
            mentions_everyone = allow_mentions && flags.has(MessageFlags::MentionsEveryone);
            mentions_online = allow_mentions && flags.has(MessageFlags::MentionsOnline);

            // Not a bot, and attempting to set mention flags
            if user.as_ref().is_some_and(|u| u.bot.as_ref().is_none())
                && (mentions_everyone || mentions_online)
            {
                return Err(create_error!(IsNotBot));
            }

            if mentions_everyone && mentions_online {
                return Err(create_error!(InvalidFlagValue));
            }
        }

        let server_id = match channel {
            Channel::TextChannel { ref server, .. } | Channel::VoiceChannel { ref server, .. } => {
                Some(server.clone())
            }
            _ => None,
        };

        // Ensure restrict_reactions is not specified without reactions list
        if let Some(interactions) = &data.interactions {
            if interactions.restrict_reactions {
                let disallowed = if let Some(list) = &interactions.reactions {
                    list.is_empty()
                } else {
                    true
                };

                if disallowed {
                    return Err(create_error!(InvalidProperty));
                }
            }
        }

        let (author_id, webhook) = match &author {
            MessageAuthor::User(user) => (user.id.clone(), None),
            MessageAuthor::Webhook(webhook) => (webhook.id.clone(), Some((*webhook).clone())),
            MessageAuthor::System { .. } => ("00000000000000000000000000".to_string(), None),
        };

        // Start constructing the message
        let message_id = Ulid::new().to_string();
        let mut message = Message {
            id: message_id.clone(),
            channel: channel.id().to_string(),
            masquerade: data.masquerade.map(|masquerade| masquerade.into()),
            interactions: data
                .interactions
                .map(|interactions| interactions.into())
                .unwrap_or_default(),
            author: author_id,
            webhook: webhook.map(|w| w.into()),
            flags: data.flags,
            ..Default::default()
        };

        // Parse mentions in message.

        let mut message_mentions = if let Some(raw_content) = &data.content {
            guilderia_parser::parse_message(raw_content)
        } else {
            guilderia_parser::MessageResults::default()
        };

        message_mentions.mentions_everyone |= mentions_everyone;
        message_mentions.mentions_online |= mentions_online;

        let guilderia_parser::MessageResults {
            mut user_mentions,
            mut role_mentions,
            mut mentions_everyone,
            mut mentions_online,
        } = message_mentions;

        if allow_mass_mentions && server_id.is_some() && !role_mentions.is_empty() {
            let server_data = db
                .fetch_server(server_id.unwrap().as_str())
                .await
                .expect("Failed to fetch server");

            role_mentions.retain(|role_id| server_data.roles.contains_key(role_id));
        }

        // Validate the user can perform a mass mention
        if !config.features.mass_mentions_enabled
            && (mentions_everyone || mentions_online || !role_mentions.is_empty())
        {
            mentions_everyone = false;
            mentions_online = false;
            role_mentions.clear();
        } else if mentions_everyone || mentions_online || !role_mentions.is_empty() {
            debug!(
                "Mentioned everyone: {}, mentioned online: {}, mentioned roles: {:?}",
                mentions_everyone, mentions_online, &role_mentions
            );
            if let Some(user) = match author {
                MessageAuthor::User(user) => Some(Ok(user)),
                MessageAuthor::System { .. } => Some(Err(())), // DISALLOWED
                MessageAuthor::Webhook(..) => None,            // Bypass check
            } {
                if user.is_err() {
                    return Err(create_error!(InvalidProperty));
                }
                let owned_user: User = user.unwrap().to_owned().into();

                let mut query = DatabasePermissionQuery::new(db, &owned_user).channel(&channel);
                let perms = calculate_channel_permissions(&mut query).await;

                if (mentions_everyone || mentions_online)
                    && !perms.has_channel_permission(ChannelPermission::MentionEveryone)
                {
                    return Err(create_error!(MissingPermission {
                        permission: ChannelPermission::MentionEveryone.to_string()
                    }));
                }

                if !role_mentions.is_empty()
                    && !perms.has_channel_permission(ChannelPermission::MentionRoles)
                {
                    return Err(create_error!(MissingPermission {
                        permission: ChannelPermission::MentionRoles.to_string()
                    }));
                }
            }
        }

        // Verify replies are valid.
        let mut replies = HashSet::new();
        if let Some(entries) = data.replies {
            if entries.len() > config.features.limits.global.message_replies {
                return Err(create_error!(TooManyReplies {
                    max: config.features.limits.global.message_replies,
                }));
            }

            for ReplyIntent {
                id,
                mention,
                fail_if_not_exists,
            } in entries
            {
                match db.fetch_message(&id).await {
                    // Referenced message exists
                    Ok(message) => {
                        if mention && allow_mentions {
                            user_mentions.insert(message.author.to_owned());
                        }

                        replies.insert(message.id);
                    }
                    // If the referenced message doesn't exist and fail_if_not_exists
                    // is set to false, send the message without the reply.
                    Err(e) => {
                        if !matches!(e.error_type, ErrorType::NotFound)
                            || fail_if_not_exists.unwrap_or(true)
                        {
                            return Err(e);
                        }
                    }
                }
            }
        }

        // Validate the mentions go to users in the channel/server
        if !user_mentions.is_empty() {
            match channel {
                Channel::DirectMessage { ref recipients, .. }
                | Channel::Group { ref recipients, .. } => {
                    let recipients_hash = HashSet::<&String, RandomState>::from_iter(recipients);
                    user_mentions.retain(|m| recipients_hash.contains(m));
                    role_mentions.clear();
                }
                Channel::TextChannel { ref server, .. }
                | Channel::VoiceChannel { ref server, .. } => {
                    let mentions_vec = Vec::from_iter(user_mentions.iter().cloned());

                    let valid_members = db.fetch_members(server.as_str(), &mentions_vec[..]).await;
                    if let Ok(valid_members) = valid_members {
                        let valid_mentions = HashSet::<&String, RandomState>::from_iter(
                            valid_members.iter().map(|m| &m.id.user),
                        );

                        user_mentions.retain(|m| valid_mentions.contains(m)); // quick pass, validate mentions are in the server

                        if !user_mentions.is_empty() {
                            // if there are still mentions, drill down to a channel-level
                            let member_channel_view_perms =
                                BulkDatabasePermissionQuery::from_server_id(db, server)
                                    .await
                                    .channel(&channel)
                                    .members(&valid_members)
                                    .members_can_see_channel()
                                    .await;

                            user_mentions
                                .retain(|m| *member_channel_view_perms.get(m).unwrap_or(&false));
                        }
                    } else {
                        guilderia_config::capture_error(&valid_members.unwrap_err());
                        return Err(create_error!(InternalError));
                    }
                }
                Channel::SavedMessages { .. } => {
                    user_mentions.clear();
                }
            }
        }

        if !user_mentions.is_empty() {
            message
                .mentions
                .replace(user_mentions.into_iter().collect());
        }

        if !role_mentions.is_empty() {
            message
                .role_mentions
                .replace(role_mentions.into_iter().collect());
        }

        if !replies.is_empty() {
            message
                .replies
                .replace(replies.into_iter().collect::<Vec<String>>());
        }

        // Calculate final message flags
        let mut flag_value = MessageFlagsValue(0);
        flag_value
            .set(MessageFlags::SuppressNotifications, suppress_notifications)
            .set(MessageFlags::MentionsEveryone, mentions_everyone)
            .set(MessageFlags::MentionsOnline, mentions_online);

        message.flags = Some(flag_value.0);

        // Add attachments to message.
        let mut attachments = vec![];
        if data
            .attachments
            .as_ref()
            .is_some_and(|v| v.len() > limits.message_attachments)
        {
            return Err(create_error!(TooManyAttachments {
                max: limits.message_attachments,
            }));
        }

        if data
            .embeds
            .as_ref()
            .is_some_and(|v| v.len() > config.features.limits.global.message_embeds)
        {
            return Err(create_error!(TooManyEmbeds {
                max: config.features.limits.global.message_embeds,
            }));
        }

        for attachment_id in data.attachments.as_deref().unwrap_or_default() {
            attachments
                .push(File::use_attachment(db, attachment_id, &message_id, author.id()).await?);
        }

        if !attachments.is_empty() {
            message.attachments.replace(attachments);
        }

        // Process included embeds.
        for sendable_embed in data.embeds.unwrap_or_default() {
            message.attach_sendable_embed(db, sendable_embed).await?;
        }

        // Set content
        message.content = data.content;

        // Pass-through nonce value for clients
        message.nonce = Some(idempotency.into_key());

        // Send the message
        message
            .send(db, amqp, author, user, member, &channel, generate_embeds)
            .await?;

        Ok(message)
    }

    /// Send a message without any notifications
    pub async fn send_without_notifications(
        &mut self,
        db: &Database,
        user: Option<v0::User>,
        member: Option<v0::Member>,
        is_dm: bool,
        generate_embeds: bool,
        // This determines if this function should queue the mentions task or if somewhere else will.
        // If this is true, you MUST call tasks::ack::queue yourself.
        mentions_elsewhere: bool,
    ) -> Result<()> {
        db.insert_message(self).await?;

        // Fan out events
        EventV1::Message(self.clone().into_model(user, member))
            .p(self.channel.to_string())
            .await;

        // Update last_message_id
        tasks::last_message_id::queue(self.channel.to_string(), self.id.to_string(), is_dm).await;

        // Add mentions for affected users
        if !mentions_elsewhere {
            if let Some(mentions) = &self.mentions {
                tasks::ack::queue_message(
                    self.channel.to_string(),
                    AckEvent::ProcessMessage {
                        messages: vec![(
                            None,
                            self.clone(),
                            mentions.clone(),
                            self.has_suppressed_notifications(),
                        )],
                    },
                )
                .await;
            }
        }

        // Generate embeds
        if generate_embeds {
            if let Some(content) = &self.content {
                tasks::process_embeds::queue(
                    self.channel.to_string(),
                    self.id.to_string(),
                    content.clone(),
                )
                .await;
            }
        }

        Ok(())
    }

    /// Send a message
    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &mut self,
        db: &Database,
        _amqp: Option<&AMQP>, // this is optional mostly for tests.
        author: MessageAuthor<'_>,
        user: Option<v0::User>,
        member: Option<v0::Member>,
        channel: &Channel,
        generate_embeds: bool,
    ) -> Result<()> {
        self.send_without_notifications(
            db,
            user.clone(),
            member.clone(),
            matches!(channel, Channel::DirectMessage { .. }),
            generate_embeds,
            true,
        )
        .await?;

        if !self.has_suppressed_notifications()
            && (self.mentions.is_some() || self.contains_mass_push_mention())
        {
            // send Push notifications
            tasks::ack::queue_message(
                self.channel.to_string(),
                AckEvent::ProcessMessage {
                    messages: vec![(
                        Some(
                            PushNotification::from(
                                self.clone().into_model(user, member),
                                Some(author),
                                channel.to_owned().into(),
                            )
                            .await,
                        ),
                        self.clone(),
                        match channel {
                            Channel::DirectMessage { recipients, .. }
                            | Channel::Group { recipients, .. } => recipients.clone(),
                            Channel::TextChannel { .. } => {
                                self.mentions.clone().unwrap_or_default()
                            }
                            _ => vec![],
                        },
                        false, // branch already dictates this
                    )],
                },
            )
            .await;
        }

        Ok(())
    }

    /// Create text embed from sendable embed
    pub async fn create_embed(&self, db: &Database, embed: SendableEmbed) -> Result<Embed> {
        embed.validate().map_err(|error| {
            create_error!(FailedValidation {
                error: error.to_string()
            })
        })?;

        let media = if let Some(id) = embed.media {
            Some(File::use_attachment(db, &id, &self.id, &self.author).await?)
        } else {
            None
        };

        Ok(Embed::Text(Text {
            icon_url: embed.icon_url,
            url: embed.url,
            title: embed.title,
            description: embed.description,
            media: media.map(|m| m.into()),
            colour: embed.colour,
        }))
    }

    /// Whether this message has suppressed notifications
    pub fn has_suppressed_notifications(&self) -> bool {
        if let Some(flags) = self.flags {
            flags & MessageFlags::SuppressNotifications as u32
                == MessageFlags::SuppressNotifications as u32
        } else {
            false
        }
    }

    pub fn contains_mass_push_mention(&self) -> bool {
        let ping = if let Some(flags) = self.flags {
            let flags = MessageFlagsValue(flags);
            flags.has(MessageFlags::MentionsEveryone)
        } else {
            false
        };

        ping || self.role_mentions.is_some()
    }

    /// Update message data
    pub async fn update(
        &mut self,
        db: &Database,
        partial: PartialMessage,
        remove: Vec<FieldsMessage>,
    ) -> Result<()> {
        self.apply_options(partial.clone());

        for field in &remove {
            self.remove_field(field);
        }

        db.update_message(&self.id, &partial, remove.clone())
            .await?;

        EventV1::MessageUpdate {
            id: self.id.clone(),
            channel: self.channel.clone(),
            data: partial.into(),
            clear: remove.into_iter().map(|field| field.into()).collect(),
        }
        .p(self.channel.clone())
        .await;

        Ok(())
    }

    /// Helper function to fetch many messages with users
    pub async fn fetch_with_users(
        db: &Database,
        query: MessageQuery,
        perspective: &User,
        include_users: Option<bool>,
        server_id: Option<String>,
    ) -> Result<BulkMessageResponse> {
        let messages: Vec<v0::Message> = db
            .fetch_messages(query)
            .await?
            .into_iter()
            .map(|msg| msg.into_model(None, None))
            .collect();

        if let Some(true) = include_users {
            let user_ids = messages
                .iter()
                .flat_map(|m| {
                    let mut users = vec![m.author.clone()];
                    if let Some(system) = &m.system {
                        match system {
                            v0::SystemMessage::ChannelDescriptionChanged { by } => {
                                users.push(by.clone())
                            }
                            v0::SystemMessage::ChannelIconChanged { by } => users.push(by.clone()),
                            v0::SystemMessage::ChannelOwnershipChanged { from, to, .. } => {
                                users.push(from.clone());
                                users.push(to.clone())
                            }
                            v0::SystemMessage::ChannelRenamed { by, .. } => users.push(by.clone()),
                            v0::SystemMessage::UserAdded { by, id, .. }
                            | v0::SystemMessage::UserRemove { by, id, .. } => {
                                users.push(by.clone());
                                users.push(id.clone());
                            }
                            v0::SystemMessage::UserBanned { id, .. }
                            | v0::SystemMessage::UserKicked { id, .. }
                            | v0::SystemMessage::UserJoined { id, .. }
                            | v0::SystemMessage::UserLeft { id, .. } => {
                                users.push(id.clone());
                            }
                            v0::SystemMessage::Text { .. } => {}
                            v0::SystemMessage::MessagePinned { by, .. } => {
                                users.push(by.clone());
                            }
                            v0::SystemMessage::MessageUnpinned { by, .. } => {
                                users.push(by.clone());
                            }
                        }
                    }
                    users
                })
                .collect::<HashSet<String>>()
                .into_iter()
                .collect::<Vec<String>>();
            let users = User::fetch_many_ids_as_mutuals(db, perspective, &user_ids).await?;

            Ok(BulkMessageResponse::MessagesAndUsers {
                messages,
                users,
                members: if let Some(server_id) = server_id {
                    Some(
                        db.fetch_members(&server_id, &user_ids)
                            .await?
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    )
                } else {
                    None
                },
            })
        } else {
            Ok(BulkMessageResponse::JustMessages(messages))
        }
    }

    /// Append content to message
    pub async fn append(
        db: &Database,
        id: String,
        channel: String,
        append: AppendMessage,
    ) -> Result<()> {
        db.append_message(&id, &append).await?;

        EventV1::MessageAppend {
            id,
            channel: channel.to_string(),
            append: append.into(),
        }
        .p(channel)
        .await;

        Ok(())
    }

    /// Convert sendable embed to text embed and attach to message
    pub async fn attach_sendable_embed(
        &mut self,
        db: &Database,
        embed: v0::SendableEmbed,
    ) -> Result<()> {
        let media: Option<v0::File> = if let Some(id) = embed.media {
            Some(
                File::use_attachment(db, &id, &self.id, &self.author)
                    .await?
                    .into(),
            )
        } else {
            None
        };

        let embed = v0::Embed::Text(v0::Text {
            icon_url: embed.icon_url,
            url: embed.url,
            title: embed.title,
            description: embed.description,
            media,
            colour: embed.colour,
        });

        if let Some(embeds) = &mut self.embeds {
            embeds.push(embed);
        } else {
            self.embeds = Some(vec![embed]);
        }

        Ok(())
    }

    /// Add a reaction to a message
    pub async fn add_reaction(&self, db: &Database, user: &User, emoji: &str) -> Result<()> {
        // Check how many reactions are already on the message
        let config = config().await;
        if self.reactions.len() >= config.features.limits.global.message_reactions
            && !self.reactions.contains_key(emoji)
        {
            return Err(create_error!(InvalidOperation));
        }

        // Check if the emoji is whitelisted
        if !self.interactions.can_use(emoji) {
            return Err(create_error!(InvalidOperation));
        }

        // Check if the emoji is usable by us
        if !Emoji::can_use(db, emoji).await? {
            return Err(create_error!(InvalidOperation));
        }

        // Send reaction event
        EventV1::MessageReact {
            id: self.id.to_string(),
            channel_id: self.channel.to_string(),
            user_id: user.id.to_string(),
            emoji_id: emoji.to_string(),
        }
        .p(self.channel.to_string())
        .await;

        // Add emoji
        db.add_reaction(&self.id, emoji, &user.id).await
    }

    /// Validate the sum of content of a message is under threshold
    pub fn validate_sum(
        content: &Option<String>,
        embeds: &[SendableEmbed],
        max_length: usize,
    ) -> Result<()> {
        let mut running_total = 0;
        if let Some(content) = content {
            running_total += content.len();
        }

        for embed in embeds {
            if let Some(desc) = &embed.description {
                running_total += desc.len();
            }
        }

        if running_total <= max_length {
            Ok(())
        } else {
            Err(create_error!(PayloadTooLarge))
        }
    }

    /// Delete a message
    pub async fn delete(self, db: &Database) -> Result<()> {
        let file_ids: Vec<String> = self
            .attachments
            .map(|files| files.iter().map(|file| file.id.to_string()).collect())
            .unwrap_or_default();

        if !file_ids.is_empty() {
            db.mark_attachments_as_deleted(&file_ids).await?;
        }

        db.delete_message(&self.id).await?;

        EventV1::MessageDelete {
            id: self.id,
            channel: self.channel.clone(),
        }
        .p(self.channel)
        .await;
        Ok(())
    }

    /// Bulk delete messages
    pub async fn bulk_delete(db: &Database, channel: &str, ids: Vec<String>) -> Result<()> {
        let valid_ids = db
            .fetch_messages_by_id(&ids)
            .await?
            .into_iter()
            .filter(|msg| msg.channel == channel)
            .map(|msg| msg.id)
            .collect::<Vec<String>>();

        db.delete_messages(channel, &valid_ids).await?;
        EventV1::BulkMessageDelete {
            channel: channel.to_string(),
            ids: valid_ids,
        }
        .p(channel.to_string())
        .await;
        Ok(())
    }

    /// Remove a reaction from a message
    pub async fn remove_reaction(&self, db: &Database, user: &str, emoji: &str) -> Result<()> {
        // Check if it actually exists
        let empty = if let Some(users) = self.reactions.get(emoji) {
            if !users.contains(user) {
                return Err(create_error!(NotFound));
            }

            users.len() == 1
        } else {
            return Err(create_error!(NotFound));
        };

        // Send reaction event
        EventV1::MessageUnreact {
            id: self.id.to_string(),
            channel_id: self.channel.to_string(),
            user_id: user.to_string(),
            emoji_id: emoji.to_string(),
        }
        .p(self.channel.to_string())
        .await;

        if empty {
            // If empty, remove the reaction entirely
            db.clear_reaction(&self.id, emoji).await
        } else {
            // Otherwise only remove that one reaction
            db.remove_reaction(&self.id, emoji, user).await
        }
    }

    /// Remove a reaction from a message
    pub async fn clear_reaction(&self, db: &Database, emoji: &str) -> Result<()> {
        // Send reaction event
        EventV1::MessageRemoveReaction {
            id: self.id.to_string(),
            channel_id: self.channel.to_string(),
            emoji_id: emoji.to_string(),
        }
        .p(self.channel.to_string())
        .await;

        // Write to database
        db.clear_reaction(&self.id, emoji).await
    }

    pub fn remove_field(&mut self, field: &FieldsMessage) {
        match field {
            FieldsMessage::Pinned => self.pinned = None,
        }
    }
}

impl SystemMessage {
    pub fn into_message(self, channel: String) -> Message {
        Message {
            id: Ulid::new().to_string(),
            channel,
            author: "00000000000000000000000000".to_string(),
            system: Some(self),

            ..Default::default()
        }
    }
}

impl Interactions {
    /// Validate interactions info is correct
    pub async fn validate(&self, db: &Database, permissions: &PermissionValue) -> Result<()> {
        let config = config().await;

        if let Some(reactions) = &self.reactions {
            permissions.throw_if_lacking_channel_permission(ChannelPermission::React)?;

            if reactions.len() > config.features.limits.global.message_reactions {
                return Err(create_error!(InvalidOperation));
            }

            for reaction in reactions {
                if !Emoji::can_use(db, reaction).await? {
                    return Err(create_error!(InvalidOperation));
                }
            }
        }

        Ok(())
    }

    /// Check if we can use a given emoji to react
    pub fn can_use(&self, emoji: &str) -> bool {
        if self.restrict_reactions {
            if let Some(reactions) = &self.reactions {
                reactions.contains(emoji)
            } else {
                false
            }
        } else {
            true
        }
    }

    /// Check if default initialisation of fields
    pub fn is_default(&self) -> bool {
        !self.restrict_reactions && self.reactions.is_none()
    }
}
