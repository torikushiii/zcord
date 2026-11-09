use super::*;

pub(super) fn dispatch(kind: &str, value: Value, ignored: &mut HashSet<String>) -> Option<Event> {
    fn id(value: &Value, field: &str) -> Option<u64> {
        value.get(field).and_then(model::id_value)
    }
    let event =
        match kind {
            "READY_SUPPLEMENTAL" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::ReadySupplemental(Box::new(v))),
            "PRESENCE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::PresenceUpdate(Box::new(v))),
            "SESSIONS_REPLACE" => Some(Event::SessionsReplace(model::parse_entries(value))),
            "GUILD_MEMBERS_CHUNK" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MembersChunk(Box::new(v))),
            "GUILD_MEMBER_LIST_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberListUpdate(Box::new(v))),
            "GUILD_MEMBER_ADD" | "GUILD_MEMBER_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberUpsert(Box::new(v))),
            "GUILD_MEMBER_REMOVE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberRemove(Box::new(v))),
            "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::RoleUpsert(Box::new(v))),
            "GUILD_ROLE_DELETE" => id(&value, "guild_id")
                .zip(id(&value, "role_id"))
                .map(|(guild_id, role_id)| Event::RoleDelete { guild_id, role_id }),
            "USER_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|user| Event::UserUpdate(Box::new(user))),
            "USER_SETTINGS_PROTO_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|settings| Event::SettingsUpdate(Box::new(settings))),
            "USER_GUILD_SETTINGS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|settings| Event::UserGuildSettingsUpdate(Box::new(settings))),
            "MESSAGE_CREATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageCreate(Box::new(m))),
            "MESSAGE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageUpdate(Box::new(m))),
            "TYPING_START" => serde_json::from_value(value)
                .ok()
                .map(|t| Event::TypingStart(Box::new(t))),
            "MESSAGE_REACTION_ADD" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionAdd),
            "MESSAGE_REACTION_REMOVE" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemove),
            "MESSAGE_REACTION_REMOVE_ALL" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemoveAll),
            "MESSAGE_REACTION_REMOVE_EMOJI" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemoveEmoji),
            "MESSAGE_REACTION_ADD_MANY" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionAddMany),
            "MESSAGE_DELETE" => id(&value, "channel_id")
                .zip(id(&value, "id"))
                .map(|(channel_id, id)| Event::MessageDelete { channel_id, id }),
            "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "THREAD_CREATE" | "THREAD_UPDATE" => {
                serde_json::from_value(value)
                    .ok()
                    .map(|c| Event::ChannelUpsert(Box::new(c)))
            }
            "CHANNEL_DELETE" | "THREAD_DELETE" => {
                id(&value, "id").map(|id| Event::ChannelDelete { id })
            }
            "CHANNEL_INFO" => serde_json::from_value(value).ok().map(Event::ChannelInfo),
            "CHANNEL_RECIPIENT_ADD" => serde_json::from_value(value)
                .ok()
                .map(Event::ChannelRecipientAdd),
            "CHANNEL_RECIPIENT_REMOVE" => serde_json::from_value(value)
                .ok()
                .map(Event::ChannelRecipientRemove),
            "CALL_CREATE" => serde_json::from_value(value).ok().map(Event::CallCreate),
            "CALL_UPDATE" => serde_json::from_value(value).ok().map(Event::CallUpdate),
            "CALL_DELETE" => {
                id(&value, "channel_id").map(|channel_id| Event::CallDelete { channel_id })
            }
            "VOICE_CHANNEL_START_TIME_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(Event::VoiceChannelStartTimeUpdate),
            "RELATIONSHIP_ADD" => serde_json::from_value(value)
                .ok()
                .map(|r| Event::RelationshipAdd(Box::new(r))),
            "RELATIONSHIP_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|r| Event::RelationshipUpdate(Box::new(r))),
            "RELATIONSHIP_REMOVE" => id(&value, "id").map(|id| Event::RelationshipRemove { id }),
            "FRIEND_SUGGESTION_CREATE" => serde_json::from_value(value)
                .ok()
                .map(|s| Event::FriendSuggestionCreate(Box::new(s))),
            "FRIEND_SUGGESTION_DELETE" => id(&value, "suggested_user_id")
                .map(|user_id| Event::FriendSuggestionDelete { user_id }),
            "THREAD_LIST_SYNC" => serde_json::from_value(value)
                .ok()
                .map(|sync| Event::ThreadListSync(Box::new(sync))),
            "THREAD_MEMBERS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|update| Event::ThreadMembersUpdate(Box::new(update))),
            "THREAD_MEMBER_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|update| Event::ThreadMemberUpdate(Box::new(update))),
            "CHANNEL_PINS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(Event::ChannelPinsUpdate),
            "SAVED_MESSAGE_CREATE" => serde_json::from_value(value)
                .ok()
                .map(|saved| Event::SavedMessageCreate(Box::new(saved))),
            "SAVED_MESSAGE_DELETE" => {
                model::SavedMessageRemoved::from_value(&value).map(Event::SavedMessageDelete)
            }
            "GUILD_CREATE" | "GUILD_UPDATE" => serde_json::from_value::<model::Guild>(value)
                .ok()
                .map(|mut guild| {
                    if kind == "GUILD_CREATE" && guild.unavailable.is_none() {
                        guild.unavailable = Some(false);
                    }
                    guild.fill_voice_state_guilds();
                    Event::GuildUpsert(Box::new(guild))
                }),
            "GUILD_EMOJIS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(Event::GuildEmojisUpdate),
            "GUILD_STICKERS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(Event::GuildStickersUpdate),
            "GUILD_SCHEDULED_EVENT_CREATE" | "GUILD_SCHEDULED_EVENT_UPDATE" => {
                serde_json::from_value(value)
                    .ok()
                    .map(Event::GuildScheduledEventUpsert)
            }
            "GUILD_SCHEDULED_EVENT_DELETE" => serde_json::from_value(value)
                .ok()
                .map(Event::GuildScheduledEventDelete),
            "GUILD_DELETE" => id(&value, "id").map(|id| Event::GuildDelete { id }),
            "MESSAGE_ACK" => {
                if value.get("ack_type").and_then(Value::as_u64).unwrap_or(0) != 0 {
                    return None;
                }
                id(&value, "channel_id").zip(id(&value, "message_id")).map(
                    |(channel_id, message_id)| Event::MessageAck {
                        channel_id,
                        message_id,
                    },
                )
            }
            _ => {
                if kind.len() <= 80
                    && kind.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
                    && ignored.insert(kind.to_owned())
                {
                    log::debug!("Ignoring gateway event {kind}");
                }
                return None;
            }
        };
    if event.is_none() {
        log::warn!("Skipping malformed supported gateway event");
    }
    event
}

#[cfg(test)]
mod tests;
