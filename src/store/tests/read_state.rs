use super::*;

#[test]
fn mentioning_message_increments_channel_and_guild_mentions() {
    let mut store = Store::from_ready(ready());
    let message: model::Message = serde_json::from_value(json!({
        "id":"100", "channel_id":"2004", "guild_id":"200",
        "author":{"id":"2","username":"writer"},
        "timestamp":"2026-10-22T12:00:00Z", "content":"ping",
        "mentions":[{"id":"1"}]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2004).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 4);
    assert_eq!(
        guild
            .channels()
            .map(|channel| channel.mentions)
            .sum::<u32>(),
        guild.mentions
    );
}

#[test]
fn own_message_does_not_increment_channel_mentions() {
    let mut store = Store::from_ready(ready());
    // Older than the channel's read state so the own-message ack cannot mask the count.
    let message: model::Message = serde_json::from_value(json!({
        "id":"45", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"1","username":"owner"},
        "timestamp":"2026-10-22T12:00:00Z", "content":"self ping",
        "mentions":[{"id":"1"}]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    assert_eq!(store.guilds[0].channel(2002).unwrap().mentions, 2);
    assert_eq!(store.guilds[0].mentions, 3);
}

#[test]
fn ack_resets_channel_and_guild_mentions() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::MessageAck {
        channel_id: 2002,
        message_id: 60,
    });
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2002).unwrap().mentions, 0);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 1);
}

#[test]
fn mark_unread_returns_the_cursor_and_counts_guild_mentions() {
    let mut store = Store::from_ready(ready());
    store.set_messages(
        2002,
        vec![
            server_message(40, 2002, 2, None),
            server_message(45, 2002, 2, None),
            server_message(50, 2002, 1, None),
        ],
    );
    let channel = store.guilds[0].channel_mut(2002).unwrap();
    channel.messages[0].mentions_me = true;
    channel.messages[1].mentions_me = true;
    channel.messages[2].mentions_me = true;

    assert_eq!(store.mark_unread(2002, 45), Some((44, 3)));
    assert_eq!(store.last_read(2002), Some(44));
    assert_eq!(store.guilds[0].channel(2002).unwrap().mentions, 3);
    assert!(store.channel_ack(2002).is_some());
}

#[test]
fn mark_unread_counts_every_dm_message_from_someone_else() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::MessageAck {
        channel_id: 900,
        message_id: 500,
    });
    store.set_messages(
        900,
        vec![
            server_message(400, 900, 2, None),
            server_message(450, 900, 2, None),
            server_message(500, 900, 2, None),
            server_message(600, 900, 1, None),
        ],
    );

    assert_eq!(store.mark_unread(900, 450), Some((449, 2)));
    assert_eq!(store.last_read(900), Some(449));
    assert_eq!(store.private_channel(900).unwrap().channel.mentions, 2);
}

#[test]
fn mark_unread_rejects_an_unread_message() {
    let mut store = Store::from_ready(ready());
    assert_eq!(store.last_read(2002), Some(50));
    assert_eq!(store.mark_unread(2002, 51), None);
    assert_eq!(store.mark_unread(2002, 0), None);
}

#[test]
fn structure_rebuild_preserves_channel_mentions() {
    let mut store = Store::from_ready(ready());
    let update = serde_json::from_value(json!({
        "id":"2002", "guild_id":"200", "name":"renamed"
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2002).unwrap().name, "renamed");
    assert_eq!(guild.channel(2002).unwrap().mentions, 2);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 3);
}

#[test]
fn badge_deltas_and_indexes_survive_structure_changes() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::MessageAck {
        channel_id: 2002,
        message_id: 60,
    });
    assert_eq!(store.guilds[0].mentions, 1);
    assert!(!store.guilds[0].unread);
    store.apply(Event::MessageAck {
        channel_id: 900,
        message_id: 1,
    });
    assert_eq!(store.dm_mentions, 0);
    store.apply(Event::ChannelDelete { id: 2003 });
    assert_eq!(store.guilds[0].mentions, 0);
    let mut message = messages().remove(0);
    message.id = 100;
    message.channel_id = 2004;
    store.apply(Event::MessageCreate(Box::new(message.clone())));
    assert!(store.guilds[0].unread);
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 100,
    });
    assert!(!store.guilds[0].unread);
    store.apply(Event::GuildDelete { id: 200 });
    assert_eq!(store.guilds[0].id, 100);
    message.id = 101;
    message.channel_id = 1001;
    store.apply(Event::MessageCreate(Box::new(message)));
    assert!(store.guilds[0].channel(1001).unwrap().unread);
    assert!(store.guilds[0].unread);
}
