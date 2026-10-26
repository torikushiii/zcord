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
