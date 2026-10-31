use super::*;

fn webhook_message(id: u64, name: &str, avatar: Option<&str>) -> model::Message {
    serde_json::from_value(json!({
        "id": id.to_string(),
        "channel_id": "2002",
        "webhook_id": "800",
        "timestamp": "2026-10-22T00:00:00Z",
        "author": {"id": "800", "username": name, "avatar": avatar, "bot": true},
        "content": "Webhook message"
    }))
    .unwrap()
}

#[test]
fn webhook_author_overrides_are_per_message_and_define_grouping() {
    let mut store = Store::from_ready(ready());
    store.absorb_user(&serde_json::from_value(json!({
        "id": "800", "username": "canonical-user", "global_name": "Canonical User", "avatar": "canonical"
    })).unwrap());
    member_event(
        &mut store,
        json!({
            "guild_id": "200", "user": {"id": "800", "username": "canonical-user", "global_name": "Canonical User", "avatar": "canonical"},
            "nick": "Server Nick", "roles": []
        }),
    );
    let mut normal = webhook_message(504, "Canonical User", None);
    normal.webhook_id = None;
    store.set_messages(
        2002,
        vec![
            webhook_message(500, "First Sender", Some("first")),
            webhook_message(501, "Second Sender", Some("second")),
            webhook_message(502, "Second Sender", Some("second")),
            webhook_message(503, "Second Sender", None),
        ],
    );
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    let messages = &channel.messages;
    assert_eq!(messages[0].webhook.as_ref().unwrap().name, "First Sender");
    assert_eq!(
        messages[0].webhook.as_ref().unwrap().avatar.as_deref(),
        Some("first")
    );
    assert_eq!(messages[1].webhook.as_ref().unwrap().name, "Second Sender");
    assert!(!messages[0].same_author(&messages[1]));
    assert!(messages[1].same_author(&messages[2]));
    assert!(!messages[2].same_author(&messages[3]));
    assert!(!messages[3].same_author(&map_message(&normal).unwrap()));
    assert_eq!(store.user(800).unwrap().name, "Canonical User");
    assert_eq!(
        store.user(800).unwrap().avatar.as_deref(),
        Some("canonical")
    );
    assert_eq!(store.member_name(200, 800), Some("Server Nick"));
}

#[test]
fn webhook_updates_and_other_messages_cannot_rename_previous_authors() {
    let mut store = Store::from_ready(ready());
    store.set_messages(
        2002,
        vec![webhook_message(500, "First Sender", Some("first"))],
    );
    store.apply(Event::MessageCreate(Box::new(webhook_message(
        501,
        "Second Sender",
        Some("second"),
    ))));
    store.apply(Event::MessageUpdate(Box::new(
        serde_json::from_value(json!({
            "id": "500", "channel_id": "2002", "content": "Edited"
        }))
        .unwrap(),
    )));
    {
        let messages = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages;
        assert_eq!(messages[0].webhook.as_ref().unwrap().name, "First Sender");
        assert_eq!(messages[0].content, "Edited");
    }
    store.apply(Event::MessageUpdate(Box::new(
        serde_json::from_value(json!({
            "id": "500", "channel_id": "2002", "author": {"id": "800"}
        }))
        .unwrap(),
    )));
    {
        let messages = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages;
        assert_eq!(messages[0].webhook.as_ref().unwrap().name, "First Sender");
        assert_eq!(
            messages[0].webhook.as_ref().unwrap().avatar.as_deref(),
            Some("first")
        );
    }
    // Partial updates normally omit webhook_id; the remembered identity still applies.
    store.apply(Event::MessageUpdate(Box::new(
        serde_json::from_value(json!({
            "id": "500", "channel_id": "2002",
            "author": {"id": "800", "username": "Edited Sender", "avatar": null, "bot": true}
        }))
        .unwrap(),
    )));
    let messages = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages;
    assert_eq!(messages[0].webhook.as_ref().unwrap().name, "Edited Sender");
    assert_eq!(messages[0].webhook.as_ref().unwrap().avatar, None);
    assert_eq!(messages[1].webhook.as_ref().unwrap().name, "Second Sender");
    assert_eq!(
        messages[1].webhook.as_ref().unwrap().avatar.as_deref(),
        Some("second")
    );
    assert!(
        store.user(800).is_none(),
        "webhook authors are not real user-cache entries"
    );
}

#[test]
fn loaded_and_optimistic_replies_preserve_the_webhook_author_snapshot() {
    let mut store = Store::from_ready(ready());
    let mut message = server_message(501, 2002, 2, None);
    message.message_reference = Some(serde_json::from_value(json!({"message_id": "500"})).unwrap());
    message.referenced_message = Some(Some(Box::new(
        serde_json::from_value(json!({
            "id": "500", "webhook_id": 800,
            "author": {"id": "800", "username": "Original Sender", "avatar": "original"},
            "content": "Original"
        }))
        .unwrap(),
    )));
    store.set_messages(
        2002,
        vec![
            webhook_message(500, "Original Sender", Some("original")),
            message,
        ],
    );
    store.apply(Event::MessageCreate(Box::new(webhook_message(
        502,
        "New Sender",
        Some("new"),
    ))));
    store.queue_message(2002, 1000, "Reply".into(), Some(500), Vec::new());
    let messages = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages;
    for index in [1, 3] {
        let Some(ReplyState::Loaded(reply)) = &messages[index].reply else {
            panic!("loaded reply expected");
        };
        let author = reply.webhook.as_ref().unwrap();
        assert_eq!(author.id, 800);
        assert_eq!(author.name, "Original Sender");
        assert_eq!(author.avatar.as_deref(), Some("original"));
    }
    assert!(store.user(800).is_none());
}
