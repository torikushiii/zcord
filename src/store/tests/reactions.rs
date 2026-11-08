use super::*;

#[test]
fn receive_only_reaction_events_update_loaded_messages_in_order() {
    fn reaction(user: u64, emoji: serde_json::Value) -> model::ReactionEvent {
        serde_json::from_value(
            json!({"channel_id":"2002","message_id":"500","user_id":user,"emoji":emoji}),
        )
        .unwrap()
    }
    let mut store = Store::from_ready(ready());
    let me = store.current_user_id;
    let mut source = messages().remove(0);
    source.id = 500;
    source.channel_id = 2002;
    source.reactions.clear();
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            me,
            json!({"name":"one"})
        ))),
        Change::None
    );
    store.set_messages(2002, vec![source]);
    let changed = Change::MessageChanged {
        channel_id: 2002,
        index: 0,
    };
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            me,
            json!({"name":"one"})
        ))),
        changed
    );
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            8,
            json!({"name":"one"})
        ))),
        changed
    );
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            8,
            json!({"id":"99","name":"custom","animated":true})
        ))),
        changed
    );
    store.apply(Event::MessageReactionAdd(reaction(
        me,
        json!({"id":"99","name":"renamed"}),
    )));
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0].emoji.name, "one");
    assert_eq!(reactions[0].count, 2);
    assert!(reactions.iter().all(|r| r.me));
    assert!(reactions[1].emoji.animated);
    assert_eq!(reactions[1].count, 2);
    store.apply(Event::MessageReactionRemove(reaction(
        me,
        json!({"name":"one"}),
    )));
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"id":"99","name":"ignored"}),
    )));
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert!(!reactions[0].me);
    assert!(reactions[1].me);
    assert_eq!(reactions[0].count, 1);
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"name":"one"}),
    )));
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"name":"absent"}),
    )));
    let many = serde_json::from_value(json!({"channel_id":"2002","message_id":"500","reactions":[null,{"users":[me,8],"emoji":{"id":"99","name":"renamed"}},{"users":[7],"emoji":{"name":"new"}},{"users":[],"emoji":{"name":"empty"}}]})).unwrap();
    assert_eq!(store.apply(Event::MessageReactionAddMany(many)), changed);
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0].emoji.id, Some(99));
    assert_eq!(reactions[0].count, 2);
    assert_eq!(reactions[1].emoji.name, "new");
    assert_eq!(
        store.apply(Event::MessageReactionRemoveEmoji(reaction(
            8,
            json!({"id":"99","name":"different"})
        ))),
        changed
    );
    assert_eq!(
        store.guilds[0].channel(2002).unwrap().messages[0]
            .reactions
            .len(),
        1
    );
    assert_eq!(
        store.apply(Event::MessageReactionRemoveAll(reaction(8, json!({})))),
        changed
    );
    assert!(
        store.guilds[0].channel(2002).unwrap().messages[0]
            .reactions
            .is_empty()
    );
    let mut absent = reaction(me, json!({"name":"one"}));
    absent.message_id = 501;
    assert_eq!(store.apply(Event::MessageReactionAdd(absent)), Change::None);
}

#[test]
fn emoji_api_reaction_echo_does_not_double_count() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    let emoji = ReactionEmoji {
        id: None,
        name: "\u{1F44D}".into(),
        animated: false,
    };
    assert_ne!(store.set_reaction(2002, 61, &emoji, true), Change::None);
    let event: model::ReactionEvent = serde_json::from_value(json!({
        "channel_id": "2002",
        "message_id": "61",
        "user_id": "1",
        "emoji": {"name": "\u{1F44D}"}
    }))
    .unwrap();
    store.apply(Event::MessageReactionAdd(event));
    let reaction = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .find(|message| message.id == 61)
        .unwrap()
        .reactions
        .iter()
        .find(|reaction| reaction.emoji.name == "\u{1F44D}")
        .unwrap();
    assert_eq!(reaction.count, 1);
    assert!(reaction.me);
}
