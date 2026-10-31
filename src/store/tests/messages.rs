use super::*;

#[test]
fn message_lookup_matches_display_indices_with_unsorted_placeholders() {
    let mut store = Store::seed();
    let channel = store.guilds[0].categories[0].channels.first_mut().unwrap();
    channel.messages.truncate(5);
    assert_eq!(channel.messages.len(), 5);
    for pattern in 0..(1 << 5) {
        for (index, message) in channel.messages.iter_mut().enumerate() {
            let placeholder = pattern & (1 << index) != 0;
            message.delivery = if !placeholder {
                Delivery::Sent
            } else if index % 2 == 0 {
                Delivery::Pending
            } else {
                Delivery::Failed
            };
            message.id = if placeholder {
                1000 - index as u64
            } else {
                (index as u64 + 1) * 10
            };
        }
        for (index, message) in channel.messages.iter().enumerate() {
            assert_eq!(channel.message_index(message.id), Some(index));
        }
        for missing in [0, 15, 35, 1001] {
            assert_eq!(channel.message_index(missing), None);
        }
    }
    channel.messages.clear();
    assert_eq!(channel.message_index(10), None);
}

#[test]
fn component_and_formatting_fixtures_map_to_view() {
    let source: model::Message =
        serde_json::from_str(include_str!("../fixtures/components_v2.json")).unwrap();
    let message = map_message(&source).unwrap();
    assert!(message.embeds.is_empty());
    let [
        Component::Container {
            accent,
            spoiler,
            components,
        },
    ] = message.components.as_slice()
    else {
        panic!("container")
    };
    assert_eq!(*accent, None);
    assert!(!spoiler);
    let [
        Component::Section {
            components,
            accessory,
        },
        Component::Row(buttons),
    ] = components.as_slice()
    else {
        panic!("section and row")
    };
    assert!(
        matches!(&components[..], [Component::Text(text)] if text.starts_with("## SAMPLE NOTICE"))
    );
    let Component::Thumbnail {
        media,
        description,
        spoiler,
    } = accessory.as_deref().unwrap()
    else {
        panic!("thumbnail")
    };
    assert_eq!(
        media.url,
        source.components[0].components[0]
            .accessory
            .as_ref()
            .unwrap()
            .media
            .as_ref()
            .unwrap()
            .proxy_url
            .as_ref()
            .unwrap()
            .as_str()
    );
    assert!(
        media
            .url
            .starts_with("https://images-ext-1.discordapp.net/")
    );
    assert_eq!((media.width, media.height), (256, 256));
    assert!(description.is_none() && !spoiler);
    let [
        Component::Button {
            style,
            label,
            emoji,
            disabled,
            url,
        },
    ] = buttons.as_slice()
    else {
        panic!("button")
    };
    assert_eq!(*style, 2);
    assert_eq!(label.as_deref(), Some("Actions: 12"));
    assert_eq!(emoji.as_ref().unwrap().name, "\u{1f36f}");
    assert!(!disabled && url.is_none());
    assert_eq!(message.reactions.len(), 5);
    for (wire, view) in source.reactions.iter().zip(&message.reactions) {
        assert_eq!(wire.count, view.count);
        assert_eq!(wire.emoji.id, view.emoji.id);
        assert_eq!(wire.emoji.name.as_deref(), Some(view.emoji.name.as_str()));
    }
    let animated = &message.reactions[4];
    assert_eq!(animated.emoji.name, "sample_animated");
    assert_eq!(animated.emoji.id, Some(1191168972947456014));
    assert!(animated.emoji.animated);
    assert_eq!(animated.count, 4);
    let mut source = source;
    source.reactions[0].me_burst = true;
    source.reactions[1].burst_me = true;
    source.reactions[2].me = true;
    assert!(
        map_message(&source).unwrap().reactions[..3]
            .iter()
            .all(|r| r.me)
    );

    let source: model::Message =
        serde_json::from_str(include_str!("../fixtures/formatting.json")).unwrap();
    let message = map_message(&source).unwrap();
    assert_eq!(message.content, source.content);
    assert!(message.embeds.is_empty() && message.components.is_empty());
    assert_eq!(message.reactions.len(), 3);
    assert_eq!(message.reactions[1].emoji.name, "sample_icon");
}

#[test]
fn malformed_rich_entries_never_discard_a_message() {
    let mut value = json!({
        "id":"1", "channel_id":"2", "author":{"id":"3"},
        "timestamp":"2026-04-09T10:24:21Z", "content":"intact",
        "embeds":[false, {"title":77}, {"title":"survives", "fields":[false, {"name":"n","value":"v"}]}],
        "components":[{"type":99}, null, {"type":17,"components":[{"type":99},{"type":9,"components":[false,{"type":10,"content":"intact"}],"accessory":{"type":99}}]},
            {"type":12,"items":[false,{"media":7},{"media":{"url":"https://cdn.discordapp.com/a"}}]}],
        "reactions":[false,{"count":"bad"},{"emoji":{"name":"ok"},"count":2}]
    });
    let source: model::Message = serde_json::from_value(value.clone()).unwrap();
    let mapped = map_message(&source).unwrap();
    assert_eq!(mapped.content, "intact");
    assert_eq!(mapped.embeds.len(), 1);
    assert_eq!(mapped.embeds[0].fields.len(), 1);
    assert_eq!(mapped.reactions.len(), 1);
    assert_eq!(mapped.components.len(), 2);
    let Component::Container { components, .. } = &mapped.components[0] else {
        panic!("container")
    };
    assert!(
        matches!(&components[..], [Component::Section { accessory: None, components }] if matches!(&components[..], [Component::Text(text)] if text == "intact"))
    );
    assert!(matches!(&mapped.components[1], Component::Gallery(items) if items.len() == 1));
    for field in ["embeds", "components", "reactions"] {
        value[field] = json!({"not":"an array"});
    }
    let source: model::Message = serde_json::from_value(value).unwrap();
    assert!(
        source.embeds.is_empty() && source.components.is_empty() && source.reactions.is_empty()
    );
}

#[test]
fn message_updates_preserve_absent_fields_and_restore_suppressed_embeds() {
    let mut store = Store::from_ready(ready());
    let mut source: model::Message =
        serde_json::from_str(include_str!("../fixtures/components_v2.json")).unwrap();
    source.channel_id = 2002;
    source.id = 500;
    source.embeds = model::parse_entries(json!([{"title":"original"}]));
    source.message_reference = Some(serde_json::from_value(json!({"message_id":"400"})).unwrap());
    source.referenced_message = Some(Some(Box::new(
        serde_json::from_value(json!({
            "id":"400","author":{"id":"7"},"components":[{"type":10,"content":"reply"}]
        }))
        .unwrap(),
    )));
    store.set_messages(2002, vec![source]);
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert!(matches!(
        &message.reply,
        Some(ReplyState::Loaded(Reply { embeds: true, .. }))
    ));
    for update in [
        json!({"flags":4}),
        json!({"content":"edited"}),
        json!({"embeds":[{"title":"replacement"}]}),
    ] {
        let mut value = update;
        value["id"] = json!("500");
        value["channel_id"] = json!("2002");
        assert_eq!(
            store.apply(Event::MessageUpdate(Box::new(
                serde_json::from_value(value).unwrap()
            ))),
            Change::MessageChanged {
                channel_id: 2002,
                index: 0
            }
        );
        let message = &store.guilds[0].channel(2002).unwrap().messages[0];
        assert!(message.embeds.is_empty());
        assert_eq!(message.reactions.len(), 5);
        assert_eq!(message.components.len(), 1);
    }
    let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","flags":0,"components":[{"type":10,"content":"new"},{"type":99}]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert_eq!(
        message.embeds[0].title.as_ref().unwrap().text,
        "replacement"
    );
    assert!(matches!(&message.components[..], [Component::Text(text)] if text == "new"));
    let update = serde_json::from_value(
        json!({"id":"500","channel_id":"2002","embeds":[],"components":[],"reactions":[]}),
    )
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert!(
        message.embeds.is_empty() && message.components.is_empty() && message.reactions.is_empty()
    );
}

#[test]
fn message_history_order_local_time_and_embeds() {
    let mut store = Store::from_ready(ready());
    assert!(!store.guilds[0].channel(2002).unwrap().messages_loaded);
    store.set_messages(2002, messages());
    let channel = store.guilds[0].channel(2002).unwrap();
    assert!(channel.messages_loaded);
    assert_eq!(
        channel.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        [61, 62]
    );
    let timestamp = channel.messages[1].timestamp;
    let expected = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:34:56.123+00:00")
        .unwrap()
        .with_timezone(&chrono::Local);
    assert_eq!(
        (
            timestamp.year,
            timestamp.month,
            timestamp.day,
            timestamp.hour,
            timestamp.minute
        ),
        (
            expected.year() as u16,
            expected.month() as u8,
            expected.day() as u8,
            expected.hour() as u8,
            expected.minute() as u8
        )
    );
    let embed = &channel.messages[1].embeds[0];
    assert!(embed.title.is_none());
    assert_eq!(embed.author.as_ref().unwrap().text, "Status");
    assert_eq!(channel.messages[1].embeds.len(), 2);
    assert_eq!(embed.description.as_deref(), Some("one\ntwo"));
    assert_eq!(embed.fields[0].name, "Count");
    assert_eq!(embed.fields[0].value, "3");
    assert_eq!(embed.footer.as_ref().unwrap().text, "footer");
    assert_eq!(embed.color, None);
    assert_eq!(channel.messages[1].content, "<@1> **raw**");
    assert!(store.user(3).unwrap().bot);
}

#[test]
fn apply_messages_loaded_unloaded_update_delete_and_ack() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.channel_id = 2004;
    message.id = 100;
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message.clone()))),
        Change::Structure
    );
    assert!(store.guilds[0].channel(2004).unwrap().messages.is_empty());
    assert!(store.guilds[0].channel(2004).unwrap().unread);
    store.set_messages(2004, vec![]);
    message.id = 101;
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message.clone()))),
        Change::MessageInserted {
            channel_id: 2004,
            index: 0
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message))),
        Change::None
    );
    let update =
        serde_json::from_value(json!({"id":"101", "channel_id":"2004", "content":"", "embeds":[]}))
            .unwrap();
    assert_eq!(
        store.apply(Event::MessageUpdate(Box::new(update))),
        Change::MessageChanged {
            channel_id: 2004,
            index: 0
        }
    );
    let updated = &store.guilds[0].channel(2004).unwrap().messages[0];
    assert!(updated.content.is_empty());
    assert!(updated.embeds.is_empty());
    assert_eq!(updated.author_id, 3);
    let update = serde_json::from_value(json!({"id":"101", "channel_id":"2004"})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert!(
        store.guilds[0].channel(2004).unwrap().messages[0]
            .content
            .is_empty()
    );
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 100,
    });
    assert!(store.guilds[0].channel(2004).unwrap().unread);
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 101,
    });
    assert!(!store.guilds[0].channel(2004).unwrap().unread);
    assert_eq!(
        store.apply(Event::MessageDelete {
            channel_id: 2004,
            id: 101
        }),
        Change::MessageRemoved {
            channel_id: 2004,
            index: 0
        }
    );
    assert_eq!(
        store.apply(Event::MessageDelete {
            channel_id: 2004,
            id: 101
        }),
        Change::None
    );
}

#[test]
fn reply_previews_hide_only_standalone_media_urls_and_keep_the_original_time() {
    let mut store = Store::from_ready(ready());
    let url = "https://klipy.com/gifs/pardofelis-honkai-impact";
    let timestamp = "2026-10-22T12:57:00Z";
    for (kind, content, embed_url, flags, expected) in [
        ("gifv", url.to_owned(), url, 0, String::new()),
        ("image", format!("  {url}\n"), url, 0, String::new()),
        ("article", url.to_owned(), url, 0, url.to_owned()),
        (
            "gifv",
            format!("Look at this {url}"),
            url,
            0,
            format!("Look at this {url}"),
        ),
        ("gifv", url.to_owned(), url, 4, url.to_owned()),
        (
            "gifv",
            url.to_owned(),
            "https://example.com/different",
            0,
            url.to_owned(),
        ),
    ] {
        let value = json!({
            "id":"500", "channel_id":"2002", "guild_id":"200",
            "author":{"id":"2","username":"writer"},
            "timestamp":"2026-10-22T13:00:00Z", "content":"reply",
            "message_reference":{"message_id":"400","channel_id":"2002"},
            "referenced_message":{
                "id":"400", "author":{"id":"8","username":"original"},
                "timestamp":timestamp, "content":content, "flags":flags,
                "embeds":[{"type":kind,"url":embed_url}]
            }
        });
        store.set_messages(2002, vec![serde_json::from_value(value).unwrap()]);
        let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
        let Some(ReplyState::Loaded(reply)) = &channel.messages[0].reply else {
            panic!("loaded reply expected");
        };
        assert_eq!(reply.content, expected, "kind={kind} flags={flags}");
        assert_eq!(Some(reply.timestamp), local_timestamp(timestamp));
    }
}

#[test]
fn queued_and_forwarded_gif_replies_use_the_same_media_preview_rule() {
    let mut store = Store::from_ready(ready());
    let url = "https://klipy.com/gifs/example";
    let original = json!({
        "id":"400", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"8","username":"original"},
        "timestamp":"2026-10-22T12:57:00Z", "content":url,
        "embeds":[{"type":"gifv","url":url}]
    });
    store.set_messages(2002, vec![serde_json::from_value(original).unwrap()]);
    store.queue_message(2002, 600, "reply".into(), Some(400));
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.messages[0].content, url);
    let Some(ReplyState::Loaded(reply)) = &channel.messages[1].reply else {
        panic!("queued reply expected");
    };
    assert!(reply.content.is_empty() && reply.embeds);
    assert_eq!(reply.timestamp, channel.messages[0].timestamp);

    let forwarded = json!({
        "id":"700", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"2","username":"writer"},
        "timestamp":"2026-10-22T13:00:00Z", "content":"reply",
        "message_reference":{"message_id":"500","channel_id":"2002"},
        "referenced_message":{
            "id":"500", "author":{"id":"8","username":"original"},
            "content":"", "message_snapshots":[{"message":{
                "content":url, "embeds":[{"type":"gifv","url":url}]
            }}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(forwarded).unwrap()]);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    let Some(ReplyState::Loaded(reply)) = &channel.messages[0].reply else {
        panic!("forwarded reply expected");
    };
    assert!(reply.content.is_empty() && reply.embeds);
}

#[test]
fn replies_mentions_and_partial_message_updates() {
    let mut store = Store::from_ready(ready());
    let base = json!({"id":"500","channel_id":"2002","guild_id":"200","author":{"id":"2","username":"writer"},"timestamp":"2026-10-22T00:00:00Z","content":"reply","member":{"nick":"Message Nick","roles":["201"]},"message_reference":{"message_id":"400","channel_id":"2002","guild_id":"200"}});
    for (referenced, expected) in [
        (None, ReplyState::Unavailable),
        (Some(serde_json::Value::Null), ReplyState::Deleted),
        (
            Some(
                json!({"id":"400","author":{"id":"8","username":"reply-author"},"content":"original","attachments":[{"id":"1"}],"embeds":[{}],"referenced_message":{"author":"malformed nested object is ignored"}}),
            ),
            ReplyState::Loaded(Reply {
                message_id: 400,
                author_id: 8,
                webhook: None,
                timestamp: timestamp_from_nonce(400),
                content: "original".into(),
                attachments: true,
                embeds: true,
                sticker: None,
            }),
        ),
    ] {
        let mut value = base.clone();
        if let Some(referenced) = referenced {
            value["referenced_message"] = referenced;
        }
        let message = serde_json::from_value(value).unwrap();
        store.set_messages(2002, vec![message]);
        assert_eq!(
            store
                .guild_by_id(200)
                .unwrap()
                .channel(2002)
                .unwrap()
                .messages[0]
                .reply,
            Some(expected)
        );
    }
    assert!(store.user(8).is_some());
    assert_eq!(store.member_name(200, 2), Some("Message Nick"));
    let mut forwarded = base.clone();
    forwarded["message_reference"]["type"] = 1.into();
    forwarded["message_snapshots"] = json!([{
        "message": {
            "content": "forwarded text",
            "embeds": [],
            "attachments": [],
            "components": [],
            "flags": 0,
            "type": 0,
            "timestamp": "2026-10-22T02:00:00Z",
            "edited_timestamp": null
        }
    }]);
    store.set_messages(2002, vec![serde_json::from_value(forwarded).unwrap()]);
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    assert!(message.reply.is_none());
    let forward = message.forward.as_ref().unwrap();
    assert_eq!(forward.content, "forwarded text");
    assert_eq!(forward.channel_id, Some(2002));
    assert_eq!(forward.guild_id, Some(200));
    assert!(forward.timestamp.is_some() && !forward.edited);
    for fields in [
        json!({"mentions":[{"id":"1"}]}),
        json!({"mention_roles":["201"]}),
        json!({"mention_everyone":true}),
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let message: model::Message = serde_json::from_value(value).unwrap();
        store.set_messages(2002, vec![]);
        store.apply(Event::MessageCreate(Box::new(message)));
        assert!(
            store
                .guild_by_id(200)
                .unwrap()
                .channel(2002)
                .unwrap()
                .messages[0]
                .mentions_me
        );
        let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","edited_timestamp":"2026-10-22T01:00:00Z","type":7})).unwrap();
        store.apply(Event::MessageUpdate(Box::new(update)));
        let message = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0];
        assert!(message.mentions_me && message.edited);
        assert_eq!(message.kind, 7);
        let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","mentions":[],"mention_roles":[],"mention_everyone":false,"referenced_message":null,"member":{"nick":"Updated Nick"}})).unwrap();
        store.apply(Event::MessageUpdate(Box::new(update)));
        let message = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0];
        assert!(!message.mentions_me);
        assert_eq!(message.reply, Some(ReplyState::Deleted));
        assert_eq!(store.member_name(200, 2), Some("Updated Nick"));
    }
}

#[test]
fn sticker_items_map_into_messages_forwards_and_updates() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "content": "",
        "sticker_items": [
            {"id": "11", "name": "wave", "format_type": 1},
            {"id": "12", "name": "dance", "format_type": 2}
        ],
        "message_reference": {"type": 1, "channel_id": "2002", "guild_id": "200"},
        "message_snapshots": [{"message": {
            "content": "",
            "sticker_items": [{"id": "13", "name": "forwarded", "format_type": 4}]
        }}]
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    let stickers: Vec<_> = message
        .stickers
        .iter()
        .map(|sticker| (sticker.id, sticker.name.as_str(), sticker.format))
        .collect();
    assert_eq!(stickers, [(11, "wave", 1), (12, "dance", 2)]);
    let forward = message.forward.as_ref().unwrap();
    assert_eq!(forward.stickers[0].id, 13);
    assert_eq!(forward.stickers[0].name, "forwarded");
    assert_eq!(forward.stickers[0].format, 4);

    let update: model::MessageUpdate = serde_json::from_value(json!({
        "id": "500",
        "channel_id": "2002",
        "sticker_items": [{"id": "14", "name": "edited", "format_type": 3}]
    }))
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    assert_eq!(message.stickers.len(), 1);
    assert_eq!(
        (
            message.stickers[0].id,
            message.stickers[0].name.as_str(),
            message.stickers[0].format
        ),
        (14, "edited", 3)
    );
}

#[test]
fn sticker_only_reply_preview_uses_sticker_name() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "message_reference": {"message_id": "400", "channel_id": "2002", "guild_id": "200"},
        "referenced_message": {
            "id": "400",
            "author": {"id": "8", "username": "reply-author"},
            "content": "",
            "sticker_items": [{"id": "9", "name": "wave", "format_type": 1}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let Some(ReplyState::Loaded(reply)) = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0]
        .reply
    else {
        panic!("sticker reply expected");
    };
    assert_eq!(reply.sticker.as_deref(), Some("wave"));
    assert!(!reply.attachments && !reply.embeds);
}

#[test]
fn message_update_replaces_forward_snapshot() {
    let mut store = Store::from_ready(ready());
    let message = json!({"id":"500","channel_id":"2002","guild_id":"200","author":{"id":"2","username":"writer"},"timestamp":"2026-10-22T00:00:00Z","message_reference":{"type":1,"channel_id":"2002","guild_id":"200"},"message_snapshots":[{"message":{"content":"first"}}]});
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let update: model::MessageUpdate = serde_json::from_value(json!({
        "id": "500",
        "channel_id": "2002",
        "message_snapshots": [{"message": {"content": "second"}}]
    }))
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let forward = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0]
        .forward
        .as_ref()
        .unwrap();
    assert_eq!(forward.content, "second");
    assert_eq!(forward.channel_id, Some(2002));
}

#[test]
fn reply_to_forward_previews_snapshot() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "message_reference": {"message_id": "400", "channel_id": "2002", "guild_id": "200"},
        "referenced_message": {
            "id": "400",
            "author": {"id": "8", "username": "reply-author"},
            "content": "",
            "attachments": [],
            "embeds": [],
            "message_reference": {"type": 1, "channel_id": "2002", "guild_id": "200"},
            "message_snapshots": [{"message": {
                "content": "forwarded preview",
                "attachments": [{"id": "1"}],
                "embeds": [{}],
                "sticker_items": [{"id": "9", "name": "forwarded sticker", "format_type": 2}]
            }}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0]
            .reply,
        Some(ReplyState::Loaded(Reply {
            message_id: 400,
            author_id: 8,
            webhook: None,
            timestamp: timestamp_from_nonce(400),
            content: "forwarded preview".into(),
            attachments: true,
            embeds: true,
            sticker: Some("forwarded sticker".into()),
        }))
    );
}

#[test]
fn prepend_messages_inserts_only_older_messages_in_order() {
    let mut store = Store::from_ready(ready());
    let mut page = messages();
    page.retain(|message| message.channel_id == 2002);
    page.sort_by_key(|message| message.id);
    let (older, newer) = page.split_at(page.len() / 2);
    assert!(!older.is_empty() && !newer.is_empty());
    store.set_messages(2002, newer.to_vec());
    let mut fetched = page.clone();
    fetched.reverse();
    assert_eq!(store.prepend_messages(2002, fetched), older.len());
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(
        ids,
        page.iter().map(|message| message.id).collect::<Vec<_>>()
    );
    assert_eq!(store.prepend_messages(2002, older.to_vec()), 0);
}

#[test]
fn open_at_detached_channel_ignores_live_message() {
    let mut store = Store::from_ready(ready());
    store.set_around_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert!(!store.at_present(2002));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            70, 2002, 3, None
        )))),
        Change::Structure
    );
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(
        channel
            .messages
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        [55]
    );
    assert!(channel.unread);
    assert_eq!(store.channel_ack(2002), Some(70));
    assert_eq!(
        store.queue_message(2002, 90, "mine".into(), None),
        Change::None
    );

    store.set_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert!(store.at_present(2002));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            70, 2002, 3, None
        )))),
        Change::MessageInserted {
            channel_id: 2002,
            index: 1
        }
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [55, 70]);
    assert_eq!(
        store.queue_message(2002, 90, "mine".into(), None),
        Change::MessageInserted {
            channel_id: 2002,
            index: 2
        }
    );
}

#[test]
fn open_at_append_messages_dedups_and_short_page_marks_present() {
    let mut store = Store::from_ready(ready());
    store.set_around_messages(
        2002,
        vec![
            server_message(55, 2002, 2, None),
            server_message(56, 2002, 2, None),
        ],
    );
    assert!(!store.at_present(2002));
    assert_eq!(
        store.append_messages(
            2002,
            vec![
                server_message(57, 2002, 2, None),
                server_message(56, 2002, 2, None),
                server_message(58, 2002, 2, None),
            ],
            50,
        ),
        2
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [55, 56, 57, 58]);
    assert!(store.at_present(2002));

    let mut store = Store::from_ready(ready());
    store.set_around_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert_eq!(
        store.append_messages(
            2002,
            vec![
                server_message(57, 2002, 2, None),
                server_message(58, 2002, 2, None),
            ],
            2,
        ),
        2
    );
    assert!(!store.at_present(2002));
    assert_eq!(store.append_messages(2002, Vec::new(), 50), 0);
    assert!(store.at_present(2002));
}

#[test]
fn queued_message_is_replaced_by_rest_then_gateway_duplicate() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    assert_eq!(
        store.queue_message(2002, 1000, "hello".into(), Some(61)),
        Change::MessageInserted {
            channel_id: 2002,
            index: 2
        }
    );
    let queued = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[2];
    assert_eq!(queued.delivery, Delivery::Pending);
    assert_eq!(queued.author_id, 1);
    assert!(matches!(&queued.reply, Some(ReplyState::Loaded(reply)) if reply.message_id == 61));
    assert_eq!(
        store.queue_message(2002, 1002, "missing".into(), Some(9999)),
        Change::MessageInserted {
            channel_id: 2002,
            index: 3
        }
    );
    assert!(matches!(
        &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[3]
            .reply,
        Some(ReplyState::Unavailable)
    ));
    store.discard_message(2002, 1002);

    let created = server_message(1001, 2002, 1, Some(1000));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created.clone()))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 2,
            inserted: 2
        }
    );
    let messages = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages;
    assert_eq!(messages[2].delivery, Delivery::Sent);
    assert_eq!(messages[2].id, 1001);
    assert_eq!(store.last_read(2002), Some(1001));
    assert!(
        !store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .unread
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created))),
        Change::None
    );
}

#[test]
fn queued_message_is_replaced_by_gateway_then_rest_duplicate() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 2000, "first".into(), None);
    let created = server_message(2001, 2002, 1, Some(2000));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created.clone()))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 2,
            inserted: 2
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created))),
        Change::None
    );
}

#[test]
fn replacement_indices_account_for_messages_arriving_between() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "mine".into(), None);
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            1500, 2002, 2, None
        )))),
        Change::MessageInserted {
            channel_id: 2002,
            index: 2
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            2000,
            2002,
            1,
            Some(1000)
        )))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 3,
            inserted: 3
        }
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [61, 62, 1500, 2000]);
}

#[test]
fn failed_and_discarded_messages_and_sent_rows_stay() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 5000, "oops".into(), None);
    assert_eq!(
        store.fail_message(2002, 5000),
        Change::MessageChanged {
            channel_id: 2002,
            index: 2
        }
    );
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[2]
            .delivery,
        Delivery::Failed
    );
    assert_eq!(store.fail_message(2002, 5000), Change::None);
    assert_eq!(store.discard_message(2002, 61), Change::None);
    assert_eq!(
        store.discard_message(2002, 5000),
        Change::MessageRemoved {
            channel_id: 2002,
            index: 2
        }
    );
    assert_eq!(store.discard_message(2002, 5000), Change::None);
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages
            .len(),
        2
    );
}

fn outbound_confirmation(id: u64, nonce: u64, content: &str) -> model::Message {
    let mut message = server_message(id, 2002, 1, Some(nonce));
    message.content = content.into();
    message
}

fn assert_display_and_lookup_invariants(store: &Store, expected_local: &[&str]) {
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    let local: Vec<_> = channel
        .messages
        .iter()
        .filter(|message| matches!(message.content.as_str(), "A" | "B" | "C" | "Retry"))
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(local, expected_local);
    let sent: Vec<_> = channel
        .messages
        .iter()
        .filter(|message| message.delivery == Delivery::Sent)
        .map(|message| message.id)
        .collect();
    assert!(
        sent.windows(2).all(|pair| pair[0] < pair[1]),
        "server chronology stays canonical"
    );
    for (index, message) in channel.messages.iter().enumerate() {
        assert_eq!(channel.message_index(message.id), Some(index));
    }
    assert_eq!(
        channel.oldest_message().map(|message| message.id),
        sent.first().copied()
    );
    assert_eq!(
        channel.newest_message().map(|message| message.id),
        sent.last().copied()
    );
}

#[test]
fn multiple_outbound_messages_keep_composer_order_for_every_ack_arrival_order() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut store = Store::from_ready(ready());
        store.set_messages(2002, messages());
        for (nonce, content) in [(1000, "A"), (1001, "B"), (1002, "C")] {
            store.queue_message(2002, nonce, content.into(), None);
        }
        store.apply(Event::MessageCreate(Box::new(server_message(
            2500, 2002, 2, None,
        ))));
        assert_display_and_lookup_invariants(&store, &["A", "B", "C"]);
        let confirmations = [
            outbound_confirmation(2000, 1000, "A"),
            outbound_confirmation(3000, 1001, "B"),
            outbound_confirmation(4000, 1002, "C"),
        ];
        for index in order {
            let message = confirmations[index].clone();
            assert!(matches!(
                store.apply(Event::MessageCreate(Box::new(message.clone()))),
                Change::MessageReplaced { .. }
            ));
            assert_display_and_lookup_invariants(&store, &["A", "B", "C"]);
            assert_eq!(
                store.apply(Event::MessageCreate(Box::new(message))),
                Change::None
            );
        }
        let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
        assert_eq!(
            channel
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [61, 62, 2000, 2500, 3000, 4000]
        );
        assert_eq!(store.last_read(2002), Some(4000));
    }
}

#[test]
fn optimistic_display_indices_support_edits_reactions_deletes_and_pagination() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    store.queue_message(2002, 1001, "B".into(), None);
    store.apply(Event::MessageCreate(Box::new(outbound_confirmation(
        3000, 1001, "B",
    ))));
    store.apply(Event::MessageCreate(Box::new(server_message(
        2500, 2002, 2, None,
    ))));
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(
        channel
            .messages
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        [61, 62, 2500, 1000, 3000]
    );
    assert_eq!(channel.first_unread_index(2500, 9999), Some(4));
    assert_eq!(channel.first_unread_index(3000, 9999), None);
    assert_eq!(channel.unread_message_count(2500, 9999), 1);
    assert_eq!(channel.unread_message_count(3000, 9999), 0);
    assert!(matches!(
        store.edit_message_locally(2002, 1000, "edited pending".into()),
        Change::MessageChanged { index: 3, .. }
    ));
    let emoji = ReactionEmoji {
        id: None,
        name: "test".into(),
        animated: false,
    };
    store.set_reaction(2002, 3000, &emoji, true);
    assert_eq!(store.reaction_state(2002, 3000, &emoji), Some(true));
    store.apply(Event::MessageReactionAdd(
        serde_json::from_value(json!({
            "channel_id": "2002", "message_id": "3000", "user_id": "2",
            "emoji": {"name": "test"}
        }))
        .unwrap(),
    ));
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(
        channel.messages[channel.message_index(3000).unwrap()].reactions[0].count,
        2
    );
    store.apply(Event::MessageUpdate(Box::new(
        serde_json::from_value(json!({
            "id": "3000", "channel_id": "2002", "content": "updated sent"
        }))
        .unwrap(),
    )));
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(
        channel.messages[channel.message_index(3000).unwrap()].content,
        "updated sent"
    );
    store.apply(Event::MessageDelete {
        channel_id: 2002,
        id: 2500,
    });
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .message_index(2500),
        None
    );
    assert_eq!(
        store.prepend_messages(2002, vec![server_message(60, 2002, 2, None)]),
        1
    );
    assert_eq!(
        store.append_messages(2002, vec![server_message(3500, 2002, 2, None)], 50),
        1
    );
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.oldest_message().unwrap().id, 60);
    assert_eq!(channel.newest_message().unwrap().id, 3500);
    assert!(channel.message_index(1000).is_some());
    store.apply(Event::MessageDelete {
        channel_id: 2002,
        id: 3000,
    });
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert!(channel.message_index(3000).is_none());
    assert_eq!(
        channel.messages.last().unwrap().id,
        1000,
        "removing a successor reanchors its placeholder"
    );
}

#[test]
fn history_refresh_preserves_and_reconciles_outstanding_local_messages() {
    for around in [false, true] {
        let mut store = Store::from_ready(ready());
        store.set_messages(2002, messages());
        store.queue_message(2002, 1000, "A".into(), None);
        store.queue_message(2002, 1001, "B".into(), None);
        store.queue_message(2002, 1002, "C".into(), None);
        store.fail_message(2002, 1002);
        store.apply(Event::MessageCreate(Box::new(outbound_confirmation(
            3000, 1001, "B",
        ))));
        if around {
            store.set_around_messages(2002, messages());
        } else {
            store.set_messages(2002, messages());
        }
        assert_display_and_lookup_invariants(&store, &["A", "B", "C"]);
        let mut page = messages();
        page.push(outbound_confirmation(2000, 1000, "A"));
        page.push(outbound_confirmation(3000, 1001, "B"));
        store.set_messages(2002, page);
        assert_display_and_lookup_invariants(&store, &["A", "B", "C"]);
        let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
        assert!(channel.message_index(1000).is_none());
        assert!(channel.message_index(1001).is_none());
        assert_eq!(
            channel.messages[channel.message_index(1002).unwrap()].delivery,
            Delivery::Failed
        );
        assert_eq!(
            store.apply(Event::MessageCreate(Box::new(outbound_confirmation(
                2000, 1000, "A"
            )))),
            Change::None
        );
        store.discard_message(2002, 1002);
        store.queue_message(2002, 5000, "Retry".into(), None);
        assert_display_and_lookup_invariants(&store, &["A", "B", "Retry"]);
        assert_eq!(
            store.append_messages(2002, vec![outbound_confirmation(6000, 5000, "Retry")], 50),
            0
        );
        assert_display_and_lookup_invariants(&store, &["A", "B", "Retry"]);
        let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
        assert!(channel.message_index(5000).is_none());
        assert_eq!(channel.newest_message().unwrap().id, 6000);
    }
}

#[test]
fn failed_first_send_stays_before_its_successor_but_a_retry_joins_the_tail() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    store.queue_message(2002, 1001, "B".into(), None);
    store.fail_message(2002, 1000);
    store.apply(Event::MessageCreate(Box::new(outbound_confirmation(
        3000, 1001, "B",
    ))));
    store.apply(Event::MessageCreate(Box::new(server_message(
        4000, 2002, 2, None,
    ))));
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
    store.discard_message(2002, 1000);
    store.queue_message(2002, 5000, "Retry".into(), None);
    assert_display_and_lookup_invariants(&store, &["B", "Retry"]);
    store.apply(Event::MessageCreate(Box::new(outbound_confirmation(
        6000, 5000, "Retry",
    ))));
    assert_display_and_lookup_invariants(&store, &["B", "Retry"]);
}

#[test]
fn pending_and_failed_nonces_are_never_history_or_unread_boundaries() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 10, "A".into(), None);
    store.queue_message(2002, 10000, "B".into(), None);
    store.fail_message(2002, 10000);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.oldest_message().unwrap().id, 61);
    assert_eq!(channel.newest_message().unwrap().id, 62);
    assert_eq!(channel.first_unread_index(62, 9999), None);
    assert_eq!(channel.unread_message_count(62, 9999), 0);
    assert_eq!(
        store.prepend_messages(2002, vec![server_message(60, 2002, 2, None)]),
        1
    );
    assert_eq!(
        store.append_messages(2002, vec![server_message(63, 2002, 2, None)], 50),
        1
    );
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.oldest_message().unwrap().id, 60);
    assert_eq!(channel.newest_message().unwrap().id, 63);
    assert!(channel.message_index(10).is_some());
    assert!(channel.message_index(10000).is_some());
}

#[test]
fn an_outbound_ack_reconciles_a_retained_row_even_while_viewing_older_history() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    store.queue_message(2002, 1001, "B".into(), None);
    store.set_around_messages(2002, vec![server_message(50, 2002, 2, None)]);
    assert!(!store.at_present(2002));
    let confirmation = outbound_confirmation(2000, 1000, "A");
    assert!(matches!(
        store.apply(Event::MessageCreate(Box::new(confirmation.clone()))),
        Change::MessageReplaced { .. }
    ));
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(confirmation))),
        Change::None
    );
    assert!(!store.at_present(2002));
    assert!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .message_index(1000)
            .is_none()
    );
}

#[test]
fn a_stale_refresh_cannot_erase_a_send_confirmed_after_the_request_started() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    let confirmation = outbound_confirmation(2000, 1000, "A");
    store.apply(Event::MessageCreate(Box::new(confirmation.clone())));
    assert_eq!(
        store.append_messages(2002, vec![confirmation.clone()], 50),
        0
    );
    // A concurrent newer-page response may see the acknowledgement before an older
    // latest-page response arrives. It must not release the outbox protection early.
    store.set_messages(2002, messages());
    assert_display_and_lookup_invariants(&store, &["A"]);
    let mut fresh = messages();
    fresh.push(confirmation);
    store.set_messages(2002, fresh);
    assert_display_and_lookup_invariants(&store, &["A"]);
    // Once a history page has observed the confirmed send, ordinary history replacement
    // may evict it again. It is no longer an outstanding outbox entry.
    store.set_around_messages(2002, messages());
    assert_display_and_lookup_invariants(&store, &[]);
}

#[test]
fn retained_outbox_confirmations_do_not_skip_unloaded_history() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    store.queue_message(2002, 1001, "B".into(), None);
    store.fail_message(2002, 1000);
    let confirmation = outbound_confirmation(3000, 1001, "B");
    store.apply(Event::MessageCreate(Box::new(confirmation.clone())));
    let mut observed = messages();
    observed.push(confirmation);
    store.set_messages(2002, observed);
    store.set_around_messages(2002, vec![server_message(50, 2002, 2, None)]);
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.newest_message().unwrap().id, 3000);
    assert_eq!(channel.newest_history_message().unwrap().id, 50);
    assert_eq!(channel.oldest_history_message().unwrap().id, 50);
    assert_eq!(
        store.append_messages(2002, vec![server_message(51, 2002, 2, None)], 1),
        1
    );
    assert!(
        !store.at_present(2002),
        "the retained confirmation cannot mark the gap complete"
    );
    assert_eq!(
        store.prepend_messages(2002, vec![server_message(49, 2002, 2, None)]),
        1
    );
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.oldest_history_message().unwrap().id, 49);
    assert_eq!(channel.newest_history_message().unwrap().id, 51);
    assert_display_and_lookup_invariants(&store, &["A", "B"]);
}

#[test]
fn stale_history_retains_confirmation_metadata_for_partial_updates() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "A".into(), None);
    let mut confirmation = outbound_confirmation(2000, 1000, "A");
    confirmation.message_reference = Some(
        serde_json::from_value(json!({
            "message_id": "61", "channel_id": "2002", "type": 0
        }))
        .unwrap(),
    );
    confirmation.referenced_message = Some(None);
    store.apply(Event::MessageCreate(Box::new(confirmation)));
    store.set_messages(2002, messages());
    store.apply(Event::MessageUpdate(Box::new(
        serde_json::from_value(json!({
            "id": "2000", "channel_id": "2002", "content": "updated sent"
        }))
        .unwrap(),
    )));
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    let message = &channel.messages[channel.message_index(2000).unwrap()];
    assert!(matches!(&message.reply, Some(ReplyState::Deleted)));
    assert_eq!(message.content, "updated sent");
}
