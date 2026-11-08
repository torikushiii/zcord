use super::*;

impl Store {
    fn reply(info: &MessageInfo) -> Option<ReplyState> {
        if info.reference.as_ref()?.kind != 0 {
            return None;
        }
        Some(match &info.referenced {
            Some(Some(message)) => {
                let snapshot = message
                    .message_snapshots
                    .first()
                    .map(|snapshot| &snapshot.message);
                let (content, embeds, flags) = match snapshot.filter(|_| message.content.is_empty())
                {
                    Some(snapshot) => (&snapshot.content, &snapshot.embeds, snapshot.flags),
                    None => (&message.content, &message.embeds, message.flags),
                };
                let embeds = mapping::map_embeds(embeds, flags);
                ReplyState::Loaded(Reply {
                    message_id: message.id,
                    author_id: message.author.id,
                    webhook: mapping::map_webhook_author(message.webhook_id, &message.author),
                    timestamp: message
                        .timestamp
                        .as_deref()
                        .and_then(local_timestamp)
                        .unwrap_or_else(|| timestamp_from_nonce(message.id)),
                    content: if hides_embed_content(content, &embeds) {
                        String::new()
                    } else {
                        content.clone()
                    },
                    attachments: !message.attachments.is_empty()
                        || snapshot.is_some_and(|snapshot| !snapshot.attachments.is_empty()),
                    embeds: !message.embeds.is_empty()
                        || !message.components.is_empty()
                        || snapshot.is_some_and(|snapshot| {
                            !snapshot.embeds.is_empty() || !snapshot.components.is_empty()
                        }),
                    sticker: message
                        .sticker_items
                        .first()
                        .or_else(|| {
                            if message.content.is_empty() {
                                snapshot.and_then(|snapshot| snapshot.sticker_items.first())
                            } else {
                                None
                            }
                        })
                        .map(|sticker| sticker.name.clone()),
                })
            }
            Some(None) => ReplyState::Deleted,
            None => ReplyState::Unavailable,
        })
    }
    fn mentioned(&self, info: &MessageInfo) -> bool {
        info.everyone
            || info.mentions.contains(&self.current_user_id)
            || info
                .guild_id
                .and_then(|id| self.social.guilds.get(&id))
                .and_then(|g| g.members.get(&self.current_user_id))
                .is_some_and(|member| info.roles.iter().any(|id| member.roles.contains(id)))
    }
    pub(crate) fn message_mentions_me(&self, message: &model::Message) -> bool {
        self.mentioned(&MessageInfo {
            guild_id: self.message_guild(message.channel_id, message.guild_id),
            mentions: message.mentions.iter().map(|user| user.id).collect(),
            roles: message.mention_roles.clone(),
            everyone: message.mention_everyone,
            ..Default::default()
        })
    }
    pub(crate) fn clear_message_info(&mut self, channel_id: u64) {
        self.social
            .messages
            .retain(|(channel, _), _| *channel != channel_id);
    }
    pub(crate) fn forget_message_info(&mut self, channel_id: u64, message_id: u64) {
        self.social.messages.remove(&(channel_id, message_id));
    }
    pub(crate) fn map_network_message(
        &mut self,
        message: &model::Message,
        remember: bool,
    ) -> Option<Message> {
        if message.webhook_id.is_none() {
            self.register_author(&message.author);
        }
        for user in &message.mentions {
            self.register_author(user);
        }
        if let Some(Some(reply)) = &message.referenced_message
            && reply.webhook_id.is_none()
        {
            self.register_author(&reply.author);
        }
        let guild_id = self.message_guild(message.channel_id, message.guild_id);
        if message.webhook_id.is_none()
            && let (Some(guild), Some(member)) = (guild_id, &message.member)
        {
            self.upsert_member(guild, message.author.id, member);
        }
        if !remember {
            return None;
        }
        let info = MessageInfo {
            flags: message.flags,
            embeds: message.embeds.clone(),
            guild_id,
            author_id: message.author.id,
            webhook_id: message.webhook_id,
            mentions: message.mentions.iter().map(|u| u.id).collect(),
            roles: message.mention_roles.clone(),
            everyone: message.mention_everyone,
            reference: message.message_reference.clone(),
            referenced: message.referenced_message.clone(),
        };
        let mut mapped = map_message(message)?;
        if let Some(thread) = &message.thread {
            mapped.thread = Some(self.message_thread(thread));
        } else {
            self.refresh_thread_notice(&mut mapped);
        }
        mapped.reply = Self::reply(&info);
        mapped.mentions_me = self.mentioned(&info);
        self.social
            .messages
            .insert((message.channel_id, message.id), info);
        Some(mapped)
    }
    pub(crate) fn update_message_info(
        &mut self,
        update: &model::MessageUpdate,
    ) -> (Option<ReplyState>, bool, Option<Vec<Embed>>) {
        let key = (update.channel_id, update.id);
        let webhook_id = update.webhook_id.or_else(|| {
            self.social
                .messages
                .get(&key)
                .and_then(|info| info.webhook_id)
        });
        if webhook_id.is_none()
            && let Some(user) = &update.author
        {
            self.register_author(user);
        }
        for user in update.mentions.iter().flatten() {
            self.register_author(user);
        }
        if let Some(Some(reply)) = &update.referenced_message
            && reply.webhook_id.is_none()
        {
            self.register_author(&reply.author);
        }
        let known = self.social.messages.contains_key(&key);
        let mut info = self.social.messages.remove(&key).unwrap_or_default();
        info.webhook_id = webhook_id;
        info.guild_id = self
            .message_guild(update.channel_id, update.guild_id)
            .or(info.guild_id);
        if let Some(author) = &update.author {
            info.author_id = author.id;
        }
        if info.webhook_id.is_none()
            && let (Some(guild), Some(member)) = (info.guild_id, &update.member)
            && info.author_id != 0
        {
            self.upsert_member(guild, info.author_id, member);
        }
        if let Some(mentions) = &update.mentions {
            info.mentions = mentions.iter().map(|u| u.id).collect();
        }
        if let Some(roles) = &update.mention_roles {
            info.roles.clone_from(roles);
        }
        if let Some(everyone) = update.mention_everyone {
            info.everyone = everyone;
        }
        if let Some(reference) = &update.message_reference {
            if &info.reference != reference && update.referenced_message.is_none() {
                info.referenced = None;
            }
            info.reference.clone_from(reference);
        }
        if update.referenced_message.is_some() {
            info.referenced.clone_from(&update.referenced_message);
        }
        if let Some(flags) = update.flags {
            info.flags = flags;
        }
        if let Some(embeds) = &update.embeds {
            info.embeds.clone_from(embeds);
        }
        let embeds = (update.flags.is_some() || update.embeds.is_some())
            .then(|| mapping::map_embeds(&info.embeds, info.flags));
        let result = (Self::reply(&info), self.mentioned(&info), embeds);
        if known {
            self.social.messages.insert(key, info);
        }
        result
    }
}
