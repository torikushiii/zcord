use super::*;
use crate::text::{markdown, timestamp::format_timestamp};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub message_id: u64,
    pub title: String,
    pub body: String,
    pub avatar: ImageSpec,
}

struct Context<'a> {
    guild: Option<&'a Guild>,
    channel: &'a Channel,
    category: Option<u64>,
    parent: Option<u64>,
    thread: bool,
    private: Option<&'a PrivateChannel>,
}

impl Store {
    /// A message's text for a toast or preview line, with mentions shown by name.
    pub(super) fn preview_text(
        &self,
        content: &str,
        guild_id: Option<u64>,
        mentions: &[model::User],
    ) -> String {
        let text = markdown::plain_text_with(&markdown::parse(content), |span| match *span {
            markdown::SpanKind::UserMention(id) => guild_id
                .and_then(|guild_id| self.member_name(guild_id, id))
                .map(str::to_owned)
                .or_else(|| self.user(id).map(|user| user.name.clone()))
                .or_else(|| {
                    let user = mentions.iter().find(|user| user.id == id)?;
                    Some(
                        user.global_name
                            .clone()
                            .unwrap_or_else(|| user.username.clone()),
                    )
                }),
            markdown::SpanKind::RoleMention(id) => Some(self.role(guild_id?, id)?.name.clone()),
            markdown::SpanKind::ChannelMention(id) => self.channel_name(id).map(str::to_owned),
            markdown::SpanKind::Timestamp { unix, style } => Some(format_timestamp(unix, style)),
            _ => None,
        });
        let text = text.trim();
        let mut end = text.len().min(200);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_owned()
    }

    pub fn dm_muted(&self, channel_id: u64, now_ms: u64) -> bool {
        self.notification_settings
            .get(&None)
            .and_then(|settings| override_for(settings, channel_id))
            .is_some_and(|setting| mute_active(setting.muted, setting.mute_config.as_ref(), now_ms))
    }

    pub fn channel_muted(&self, guild_id: u64, channel_id: u64, now_ms: u64) -> bool {
        self.notification_settings
            .get(&Some(guild_id))
            .and_then(|settings| override_for(settings, channel_id))
            .is_some_and(|setting| mute_active(setting.muted, setting.mute_config.as_ref(), now_ms))
    }

    pub fn channel_notification_level(
        &self,
        guild_id: u64,
        channel_id: u64,
    ) -> model::NotificationLevel {
        self.notification_settings
            .get(&Some(guild_id))
            .and_then(|settings| override_for(settings, channel_id))
            .map_or(model::NotificationLevel::Inherit, |setting| {
                setting.message_notifications
            })
    }

    pub fn guild_muted(&self, guild_id: u64, now_ms: u64) -> bool {
        self.notification_settings
            .get(&Some(guild_id))
            .is_some_and(|settings| {
                mute_active(settings.muted, settings.mute_config.as_ref(), now_ms)
            })
    }

    /// When an active timed mute ends, in Unix milliseconds; `None` if unmuted or muted indefinitely.
    pub fn guild_mute_end(&self, guild_id: u64, now_ms: u64) -> Option<u64> {
        let settings = self.notification_settings.get(&Some(guild_id))?;
        settings
            .mute_config
            .as_ref()?
            .end_time
            .filter(|end_time| settings.muted && *end_time > now_ms)
    }

    pub fn guild_notification_level(&self, guild_id: u64) -> model::NotificationLevel {
        if let Some(level) = self
            .notification_settings
            .get(&Some(guild_id))
            .map(|settings| settings.message_notifications)
            .filter(|level| *level != model::NotificationLevel::Inherit)
        {
            return level;
        }
        match self.guild_notification_defaults.get(&guild_id) {
            Some(model::NotificationLevel::Inherit) | None => model::NotificationLevel::All,
            Some(level) => *level,
        }
    }

    pub fn guild_notification_flags(&self, guild_id: u64) -> model::GuildNotificationFlags {
        self.notification_settings
            .get(&Some(guild_id))
            .map(|settings| model::GuildNotificationFlags {
                suppress_everyone: settings.suppress_everyone,
                suppress_roles: settings.suppress_roles,
                suppress_highlights: settings.notify_highlights == 1,
                mute_scheduled_events: settings.mute_scheduled_events,
                mobile_push: settings.mobile_push.unwrap_or(true),
            })
            .unwrap_or_default()
    }

    pub fn message_alert(&self, message: &model::Message, now_ms: u64) -> Option<Alert> {
        if message.author.id == self.current_user_id {
            return None;
        }
        if self
            .relationship(message.author.id)
            .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
        {
            return None;
        }
        if self
            .user(self.current_user_id)
            .is_some_and(|user| user.status == Status::DoNotDisturb)
        {
            return None;
        }
        let context = self.context(message.channel_id)?;
        if let Some(private) = context.private {
            if private.message_request {
                return None;
            }
            if self
                .notification_settings
                .get(&None)
                .and_then(|settings| override_for(settings, message.channel_id))
                .is_some_and(|setting| {
                    mute_active(setting.muted, setting.mute_config.as_ref(), now_ms)
                })
            {
                return None;
            }
            let title = self.author_name(message, None);
            return self.alert(&context, message, title);
        }
        let guild = context.guild?;
        if self.source.is_some()
            && !self
                .permissions(context.channel.id)
                .is_some_and(|permissions| permissions & discord::VIEW_CHANNEL != 0)
        {
            return None;
        }
        let settings = self.notification_settings.get(&Some(guild.id));
        if settings.is_some_and(|settings| {
            mute_active(settings.muted, settings.mute_config.as_ref(), now_ms)
        }) {
            return None;
        }
        for id in [Some(context.channel.id), context.parent, context.category]
            .into_iter()
            .flatten()
        {
            if settings
                .and_then(|settings| override_for(settings, id))
                .is_some_and(|setting| {
                    mute_active(setting.muted, setting.mute_config.as_ref(), now_ms)
                })
            {
                return None;
            }
        }
        let mut level = self.effective_level(guild.id, &context, settings);
        // A thread you haven't joined only reaches you by mentioning you.
        if context.thread
            && level == model::NotificationLevel::All
            && !self.thread_joins.contains_key(&context.channel.id)
        {
            level = model::NotificationLevel::Mentions;
        }
        let mentioned = self.mentions(message, settings);
        if !alerts(level, mentioned) {
            return None;
        }
        let title = self.author_name(message, Some(guild.id));
        self.alert(&context, message, title)
    }

    fn context(&self, channel_id: u64) -> Option<Context<'_>> {
        for guild in &self.guilds {
            for category in &guild.categories {
                if let Some(channel) = category
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                {
                    return Some(Context {
                        guild: Some(guild),
                        channel,
                        category: Some(category.id),
                        parent: None,
                        thread: false,
                        private: None,
                    });
                }
            }
            if let Some(thread) = guild.threads.iter().find(|thread| thread.id == channel_id) {
                let parent = thread
                    .parent_id
                    .and_then(|id| guild.channels().find(|channel| channel.id == id));
                let category = parent.and_then(|parent| {
                    guild
                        .categories
                        .iter()
                        .find(|category| {
                            category
                                .channels
                                .iter()
                                .any(|channel| channel.id == parent.id)
                        })
                        .map(|category| category.id)
                });
                return Some(Context {
                    guild: Some(guild),
                    channel: thread,
                    category,
                    parent: parent.map(|channel| channel.id),
                    thread: true,
                    private: None,
                });
            }
        }
        for private in &self.private {
            if private.channel.id == channel_id {
                return Some(Context {
                    guild: None,
                    channel: &private.channel,
                    category: None,
                    parent: None,
                    thread: false,
                    private: Some(private),
                });
            }
        }
        None
    }

    fn effective_level(
        &self,
        guild_id: u64,
        context: &Context<'_>,
        settings: Option<&model::UserGuildSettings>,
    ) -> model::NotificationLevel {
        if let Some(settings) = settings {
            for id in [Some(context.channel.id), context.parent, context.category]
                .into_iter()
                .flatten()
            {
                if let Some(level) = override_for(settings, id)
                    .map(|setting| setting.message_notifications)
                    .filter(|level| *level != model::NotificationLevel::Inherit)
                {
                    return level;
                }
            }
        }
        self.guild_notification_level(guild_id)
    }

    fn mentions(
        &self,
        message: &model::Message,
        settings: Option<&model::UserGuildSettings>,
    ) -> bool {
        let suppress_everyone = settings.is_some_and(|settings| settings.suppress_everyone);
        let suppress_roles = settings.is_some_and(|settings| settings.suppress_roles);
        if !suppress_everyone && !suppress_roles {
            return self.message_mentions_me(message);
        }
        let mut filtered = message.clone();
        if suppress_everyone {
            filtered.mention_everyone = false;
        }
        if suppress_roles {
            filtered.mention_roles.clear();
        }
        self.message_mentions_me(&filtered)
    }

    fn author_name(&self, message: &model::Message, guild_id: Option<u64>) -> String {
        let name = guild_id
            .and_then(|guild_id| self.member_name(guild_id, message.author.id))
            .map(str::to_owned)
            .or_else(|| self.user(message.author.id).map(|user| user.name.clone()))
            .or_else(|| message.author.global_name.clone())
            .unwrap_or_else(|| message.author.username.clone());
        if name.is_empty() {
            "Unknown User".to_owned()
        } else {
            name
        }
    }

    fn embed_text(&self, message: &model::Message, guild_id: Option<u64>) -> String {
        message
            .embeds
            .iter()
            .map(|embed| {
                [&embed.title, &embed.description]
                    .into_iter()
                    .flatten()
                    .map(|text| self.preview_text(text, guild_id, &message.mentions))
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .find(|text| !text.is_empty())
            .unwrap_or_default()
    }

    fn alert(
        &self,
        context: &Context<'_>,
        message: &model::Message,
        author: String,
    ) -> Option<Alert> {
        let body = self.preview_text(
            &message.content,
            context.guild.map(|guild| guild.id),
            &message.mentions,
        );
        let body = if body.is_empty() {
            let embed = self.embed_text(message, context.guild.map(|guild| guild.id));
            if !embed.is_empty() {
                embed
            // Discord's own notifications name the first file.
            } else if let Some(attachment) = message.attachments.first() {
                if attachment.filename.is_empty() {
                    "Uploaded a file".to_owned()
                } else {
                    format!("Uploaded {}", attachment.filename)
                }
            } else if !matches!(message.kind, 0 | 19) {
                return None;
            } else {
                String::new()
            }
        } else {
            body
        };
        let guild = context.guild;
        let title = match guild {
            Some(guild) => match context
                .category
                .and_then(|id| guild.categories.iter().find(|category| category.id == id))
                .map(|category| category.name.as_str())
                .filter(|name| !name.is_empty())
            {
                Some(category) => format!("{author} (#{}, {category})", context.channel.name),
                None => format!("{author} (#{})", context.channel.name),
            },
            None => author,
        };
        let avatar = guild
            .and_then(|guild| self.member_avatar_spec(guild.id, message.author.id))
            .or_else(|| self.user(message.author.id).map(User::avatar_spec))
            .unwrap_or_else(|| {
                avatar_spec(
                    message.author.id,
                    message.author.avatar.as_deref(),
                    message.author.discriminator.as_deref(),
                )
            });
        Some(Alert {
            guild_id: guild.map(|guild| guild.id),
            channel_id: message.channel_id,
            message_id: message.id,
            title,
            body,
            avatar,
        })
    }
}

fn override_for<'a>(
    settings: &'a model::UserGuildSettings,
    channel_id: u64,
) -> Option<&'a model::ChannelOverride> {
    settings
        .channel_overrides
        .iter()
        .find(|setting| setting.channel_id == Some(channel_id))
}

fn mute_active(muted: bool, config: Option<&model::MuteConfig>, now_ms: u64) -> bool {
    muted
        && config
            .and_then(|config| config.end_time)
            .is_none_or(|end_time| end_time > now_ms)
}

fn alerts(level: model::NotificationLevel, mentioned: bool) -> bool {
    match level {
        model::NotificationLevel::All => true,
        model::NotificationLevel::Mentions => mentioned,
        model::NotificationLevel::Nothing => false,
        model::NotificationLevel::Inherit => true,
    }
}
