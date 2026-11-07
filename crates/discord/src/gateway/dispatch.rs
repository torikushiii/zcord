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
mod tests {
    use super::*;

    #[test]
    fn private_call_events_parse_ids_and_partial_updates() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "CALL_CREATE",
            json!({
                "channel_id": "902", "message_id": "50", "region": "singapore",
                "ringing": ["1", 10], "voice_states": [{"user_id": "10", "self_mute": true}]
            }),
            &mut ignored,
        );
        let Some(Event::CallCreate(call)) = event else {
            panic!("expected call create")
        };
        assert_eq!(call.channel_id, 902);
        assert_eq!(call.message_id, Some(50));
        assert_eq!(call.region, Some(Some("singapore".to_owned())));
        assert_eq!(call.ringing, Some(vec![1, 10]));
        assert_eq!(call.voice_states.unwrap()[0].user_id, 10);
        let event = dispatch(
            "CALL_UPDATE",
            json!({"channel_id": 902, "ringing": [], "region": null}),
            &mut ignored,
        );
        let Some(Event::CallUpdate(call)) = event else {
            panic!("expected call update")
        };
        assert_eq!(call.ringing, Some(vec![]));
        assert_eq!(call.region, Some(None));
        assert!(call.voice_states.is_none());
        assert!(matches!(
            dispatch("CALL_DELETE", json!({"channel_id": "902"}), &mut ignored),
            Some(Event::CallDelete { channel_id: 902 })
        ));
        for kind in ["CALL_CREATE", "CALL_UPDATE", "CALL_DELETE"] {
            assert!(dispatch(kind, json!({"channel_id": "bad"}), &mut ignored).is_none());
        }
        assert!(ignored.is_empty());
    }

    #[test]
    fn private_recipient_events_parse_user_and_channel() {
        let mut ignored = HashSet::new();
        let payload = json!({"channel_id": "902", "user": {"id": "20", "username": "new"}});
        for kind in ["CHANNEL_RECIPIENT_ADD", "CHANNEL_RECIPIENT_REMOVE"] {
            let event = dispatch(kind, payload.clone(), &mut ignored).unwrap();
            let (Event::ChannelRecipientAdd(update) | Event::ChannelRecipientRemove(update)) =
                event
            else {
                panic!("expected recipient event");
            };
            assert_eq!(update.channel_id, 902);
            assert_eq!(update.user.id, 20);
            assert!(dispatch(kind, json!({"channel_id": "902"}), &mut ignored).is_none());
        }
        assert!(ignored.is_empty());
    }

    #[test]
    fn channel_updates_share_the_existing_guild_and_private_dispatch_path() {
        let mut ignored = HashSet::new();
        for payload in [
            json!({"id": "902", "type": 3, "name": "Renamed", "icon": null, "owner_id": "1"}),
            json!({"id": "2002", "guild_id": "200", "type": 2, "name": "Voice"}),
        ] {
            assert!(matches!(
                dispatch("CHANNEL_UPDATE", payload, &mut ignored),
                Some(Event::ChannelUpsert(_))
            ));
        }
        assert!(matches!(
            dispatch("CHANNEL_DELETE", json!({"id": "902"}), &mut ignored),
            Some(Event::ChannelDelete { id: 902 })
        ));
    }

    #[test]
    fn guild_stickers_update_dispatches_typed_catalog_and_skips_malformed_entries() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "GUILD_STICKERS_UPDATE",
            json!({"guild_id": "200", "stickers": [
                {"id": "9001", "name": "wave", "type": 2, "format_type": 4,
                 "guild_id": "200", "available": false, "description": null},
                {"name": "missing-id"}, {"id": "bad"}, null
            ]}),
            &mut ignored,
        );
        let Some(Event::GuildStickersUpdate(update)) = event else {
            panic!("expected guild stickers update");
        };
        assert_eq!(update.guild_id, 200);
        let stickers = update.stickers.unwrap();
        assert_eq!(stickers.len(), 1);
        assert_eq!(stickers[0].id, 9001);
        assert_eq!(stickers[0].name, "wave");
        assert_eq!(stickers[0].kind, 2);
        assert_eq!(stickers[0].format_type, 4);
        assert_eq!(stickers[0].guild_id, Some(200));
        assert_eq!(stickers[0].available, Some(false));
        assert!(stickers[0].description.is_none());
        assert!(ignored.is_empty());
    }

    #[test]
    fn guild_stickers_update_distinguishes_missing_or_null_from_empty_catalog() {
        let mut ignored = HashSet::new();
        for (payload, expected_len) in [
            (json!({"guild_id": 200}), None),
            (json!({"guild_id": "200", "stickers": null}), None),
            (json!({"guild_id": "200", "stickers": []}), Some(0)),
        ] {
            let event = dispatch("GUILD_STICKERS_UPDATE", payload, &mut ignored);
            let Some(Event::GuildStickersUpdate(update)) = event else {
                panic!("expected guild stickers update");
            };
            assert_eq!(update.guild_id, 200);
            assert_eq!(update.stickers.as_ref().map(Vec::len), expected_len);
        }
    }

    #[test]
    fn guild_stickers_update_rejects_missing_or_invalid_guild_identity() {
        let mut ignored = HashSet::new();
        for payload in [
            json!({"stickers": []}),
            json!({"guild_id": null, "stickers": []}),
            json!({"guild_id": "bad", "stickers": []}),
        ] {
            assert!(dispatch("GUILD_STICKERS_UPDATE", payload, &mut ignored).is_none());
        }
    }

    #[test]
    fn voice_session_timestamps_dispatch_with_missing_and_null_fields() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "CHANNEL_INFO",
            json!({"guild_id":"7","channels":[
                {"id":"8","voice_start_time":1770000000},
                {"id":"9","voice_start_time":null},
                {"id":"10","status":"Chatting"}
            ]}),
            &mut ignored,
        );
        let Some(Event::ChannelInfo(info)) = event else {
            panic!("expected channel info");
        };
        assert_eq!(info.guild_id, 7);
        assert_eq!(info.channels.len(), 3);
        assert_eq!(info.channels[0].id, 8);
        assert_eq!(info.channels[0].voice_start_time, Some(Some(1770000000)));
        assert_eq!(info.channels[1].voice_start_time, Some(None));
        assert_eq!(info.channels[2].voice_start_time, None);

        for (value, expected) in [
            (
                json!({"id":"8","guild_id":"7","voice_start_time":1770000000}),
                Some(Some(1770000000)),
            ),
            (
                json!({"id":"8","guild_id":"7","voice_start_time":null}),
                Some(None),
            ),
            (json!({"id":"8","guild_id":"7"}), None),
        ] {
            let event = dispatch("VOICE_CHANNEL_START_TIME_UPDATE", value, &mut ignored);
            assert!(matches!(
                event,
                Some(Event::VoiceChannelStartTimeUpdate(update))
                    if update.id == 8 && update.guild_id == 7 && update.voice_start_time == expected
            ));
        }
        assert!(
            dispatch(
                "CHANNEL_INFO",
                json!({"guild_id":"bad","channels":[]}),
                &mut ignored,
            )
            .is_none()
        );
        assert!(ignored.is_empty());
    }

    #[test]
    fn typing_start_dispatches_with_member() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "TYPING_START",
            json!({"channel_id":"2","guild_id":"3","user_id":"4","timestamp":1770000000,
                "member":{"user":{"id":"4","username":"typer"},"roles":["5"]}}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::TypingStart(typing))
                if typing.channel_id == 2
                    && typing.guild_id == Some(3)
                    && typing.user_id == 4
                    && typing.member.as_ref().and_then(|m| m.user.as_ref()).is_some_and(|u| u.id == 4)
        ));
        assert!(ignored.is_empty());
    }

    #[test]
    fn relationship_remove_dispatches_id() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "RELATIONSHIP_REMOVE",
            json!({"id":"10","type":1}),
            &mut ignored,
        );
        assert!(matches!(event, Some(Event::RelationshipRemove { id: 10 })));
        assert!(ignored.is_empty());
    }

    #[test]
    fn user_update_dispatches_nullable_user_fields() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "USER_UPDATE",
            json!({"id":"1","username":"new_handle","global_name":"New Name","avatar":null}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::UserUpdate(user))
                if user.id == 1
                    && user.username == "new_handle"
                    && matches!(&user.global_name, Some(Some(name)) if name == "New Name")
                    && matches!(user.avatar, Some(None))
        ));
        assert!(ignored.is_empty());
    }

    #[test]
    fn reaction_dispatches_are_receive_only() {
        let mut ignored = HashSet::new();
        let value = json!({"channel_id":"1","message_id":"2","user_id":"3","emoji":{"id":"4","name":"custom","animated":true}});
        assert!(
            matches!(dispatch("MESSAGE_REACTION_ADD", value.clone(), &mut ignored), Some(Event::MessageReactionAdd(event)) if event.user_id == Some(3) && event.emoji.animated)
        );
        assert!(matches!(
            dispatch("MESSAGE_REACTION_REMOVE", value.clone(), &mut ignored),
            Some(Event::MessageReactionRemove(_))
        ));
        assert!(matches!(
            dispatch("MESSAGE_REACTION_REMOVE_EMOJI", value, &mut ignored),
            Some(Event::MessageReactionRemoveEmoji(_))
        ));
        assert!(
            matches!(dispatch("MESSAGE_REACTION_REMOVE_ALL", json!({"channel_id":"1","message_id":"2"}), &mut ignored), Some(Event::MessageReactionRemoveAll(event)) if event.channel_id == 1 && event.message_id == 2)
        );
        let many = json!({"channel_id":"1","message_id":"2","reactions":[false,{"users":["3","bad",4],"emoji":{"name":"ok"}}]});
        assert!(
            matches!(dispatch("MESSAGE_REACTION_ADD_MANY", many, &mut ignored), Some(Event::MessageReactionAddMany(event)) if event.reactions.len() == 1 && event.reactions[0].users == [3,4])
        );
        for kind in [
            "MESSAGE_REACTION_ADD",
            "MESSAGE_REACTION_REMOVE",
            "MESSAGE_REACTION_REMOVE_ALL",
            "MESSAGE_REACTION_REMOVE_EMOJI",
            "MESSAGE_REACTION_ADD_MANY",
        ] {
            assert!(dispatch(kind, json!({"message_id":"bad"}), &mut ignored).is_none());
        }
        assert!(ignored.is_empty());
    }

    #[test]
    fn social_dispatches_and_disconnect_generations() {
        let fixture: Value = serde_json::from_str(include_str!("../fixtures/social.json")).unwrap();
        let mut ignored = HashSet::new();
        assert!(matches!(
            dispatch(
                "READY_SUPPLEMENTAL",
                fixture["supplemental"].clone(),
                &mut ignored
            ),
            Some(Event::ReadySupplemental(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_MEMBERS_CHUNK",
                fixture["chunk"].clone(),
                &mut ignored
            ),
            Some(Event::MembersChunk(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_MEMBER_LIST_UPDATE",
                fixture["list"].clone(),
                &mut ignored
            ),
            Some(Event::MemberListUpdate(_))
        ));
        for event in ["GUILD_MEMBER_ADD", "GUILD_MEMBER_UPDATE"] {
            assert!(matches!(
                dispatch(
                    event,
                    json!({"guild_id":"1","user":{"id":"2"}}),
                    &mut ignored
                ),
                Some(Event::MemberUpsert(_))
            ));
        }
        assert!(matches!(
            dispatch(
                "GUILD_MEMBER_REMOVE",
                json!({"guild_id":"1","user":{"id":"2"}}),
                &mut ignored
            ),
            Some(Event::MemberRemove(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_ROLE_CREATE",
                json!({"guild_id":"1","role":{"id":"2"}}),
                &mut ignored
            ),
            Some(Event::RoleUpsert(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_ROLE_DELETE",
                json!({"guild_id":"1","role_id":"2"}),
                &mut ignored
            ),
            Some(Event::RoleDelete {
                guild_id: 1,
                role_id: 2
            })
        ));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
        let mut commands = CommandReceiver {
            receiver,
            generation: generation.clone(),
            presence: Arc::default(),
            presence_after: Instant::now(),
        };
        commands.ready();
        assert_eq!(generation.load(Ordering::Acquire), 1);
        sender
            .send((1, crate::commands::subscription(1, 2, &[])))
            .unwrap_or_else(|_| panic!("queue closed"));
        let (completion, mut result) = tokio::sync::oneshot::channel();
        let mut pending = crate::commands::presence(json!({"op":3}), 1);
        pending.completion = Some(completion);
        sender
            .send((1, pending))
            .unwrap_or_else(|_| panic!("queue closed"));
        commands.disconnected();
        assert_eq!(generation.load(Ordering::Acquire), 2);
        assert!(commands.receiver.try_recv().is_err());
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        commands.ready();
        assert_eq!(generation.load(Ordering::Acquire), 3);
    }

    #[test]
    fn guild_create_fills_voice_state_guild_ids() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "GUILD_CREATE",
            json!({"id":"7","voice_states":[{"channel_id":"8","user_id":"9"}]}),
            &mut ignored,
        );
        let Some(Event::GuildUpsert(guild)) = event else {
            panic!("expected guild upsert");
        };
        assert_eq!(guild.voice_states.len(), 1);
        assert_eq!(guild.voice_states[0].guild_id, Some(7));
        assert_eq!(guild.voice_states[0].channel_id, Some(8));
        assert_eq!(guild.voice_states[0].user_id, 9);
    }

    #[test]
    fn thread_and_pin_events_dispatch() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "CHANNEL_PINS_UPDATE",
            json!({"guild_id": "2", "channel_id": "20", "last_pin_timestamp": "2026-09-10T12:00:00Z"}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::ChannelPinsUpdate(update))
                if update.channel_id == 20
                    && update.guild_id == Some(2)
                    && update.last_pin_timestamp.as_deref() == Some("2026-09-10T12:00:00Z")
        ));

        let event = dispatch(
            "SAVED_MESSAGE_CREATE",
            json!({"save_data": {"channel_id": "20", "message_id": "30", "guild_id": "2",
                "saved_at": "2026-11-06T20:11:36.127788+00:00",
                "due_at": "2026-11-06T20:41:35.871000+00:00"},
                "message": {"id": "30", "channel_id": "20", "content": "hi", "type": 0,
                    "timestamp": "2026-11-02T18:22:45.791000+00:00",
                    "author": {"id": "5", "username": "user", "discriminator": "0"}}}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::SavedMessageCreate(saved))
                if saved.save_data.message_id == 30
                    && saved.save_data.guild_id == Some(2)
                    && saved.save_data.due_at.as_deref() == Some("2026-11-06T20:41:35.871000+00:00")
                    && saved.message.as_ref().is_some_and(|message| message.id == 30)
        ));
        for payload in [
            json!({"channel_id": "20", "message_id": "30"}),
            json!({"save_data": {"channel_id": "20", "message_id": "30"}}),
        ] {
            assert_eq!(
                dispatch("SAVED_MESSAGE_DELETE", payload, &mut ignored).and_then(
                    |event| match event {
                        Event::SavedMessageDelete(removed) => Some(removed),
                        _ => None,
                    }
                ),
                Some(model::SavedMessageRemoved {
                    channel_id: 20,
                    message_id: 30
                })
            );
        }

        let event = dispatch(
            "THREAD_MEMBERS_UPDATE",
            json!({"id": "30", "guild_id": "2", "member_count": 3,
                "added_members": [{"id": "30", "user_id": "1"}], "removed_member_ids": []}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::ThreadMembersUpdate(update))
                if update.id == 30 && update.member_count == 3
        ));

        let event = dispatch(
            "THREAD_MEMBER_UPDATE",
            json!({"id": "30", "guild_id": "2", "user_id": "1", "member_count": 4}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::ThreadMemberUpdate(update))
                if update.id == 30 && update.user_id == Some(1)
        ));
        assert!(ignored.is_empty());
    }

    #[test]
    fn user_guild_settings_update_dispatches_server_and_dm_entries() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "USER_GUILD_SETTINGS_UPDATE",
            json!({
                "guild_id":"7",
                "message_notifications":2,
                "suppress_everyone":true,
                "suppress_roles":true,
                "notify_highlights":1,
                "mute_scheduled_events":true,
                "mobile_push":false,
                "channel_overrides":[{"channel_id":"8","muted":true}]
            }),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::UserGuildSettingsUpdate(settings))
                if settings.guild_id == Some(7)
                    && settings.message_notifications == model::NotificationLevel::Nothing
                    && settings.suppress_everyone
                    && settings.suppress_roles
                    && settings.notify_highlights == 1
                    && settings.mute_scheduled_events
                    && settings.mobile_push == Some(false)
                    && settings.channel_overrides[0].channel_id == Some(8)
                    && settings.channel_overrides[0].muted
        ));

        let event = dispatch(
            "USER_GUILD_SETTINGS_UPDATE",
            json!({"guild_id":null,"channel_overrides":[{"channel_id":"9","muted":true}]}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::UserGuildSettingsUpdate(settings))
                if settings.guild_id.is_none()
                    && settings.channel_overrides[0].channel_id == Some(9)
        ));
        assert!(ignored.is_empty());
    }
}
