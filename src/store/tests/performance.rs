use super::*;

#[test]
#[cfg_attr(debug_assertions, ignore)]
fn thousand_message_creates_with_large_ready_are_fast() {
    let mut ready = ready();
    ready.guilds = (0..100u64)
        .map(|index| {
            let id = 10_000 + index;
            let channels: Vec<_> = (0..20u64)
                .map(|channel| {
                    json!({
                        "id": (100_000 + index * 20 + channel).to_string(), "type": 0,
                        "name": "busy", "last_message_id": "5000"
                    })
                })
                .collect();
            serde_json::from_value(json!({
                "id": id.to_string(), "properties": {"name": "Busy Guild", "owner_id": "99"},
                "roles": [{"id": id.to_string(), "permissions": "1024"}],
                "members": [{"user_id": "1", "roles": []}], "channels": channels
            }))
            .unwrap()
        })
        .collect();
    ready.read_state.entries = (0..2000)
        .map(|index| model::ReadState {
            id: 100_000 + index,
            last_message_id: Some(5000),
            mention_count: 0,
            kind: 0,
            last_pin_timestamp: None,
        })
        .collect();
    ready.users = (0..20_000)
        .map(|index| model::User {
            id: 10 + index,
            username: format!("user-{index}"),
            ..Default::default()
        })
        .collect();
    ready.private_channels.clear();
    let mut store = Store::from_ready(ready);
    let template = messages().remove(0);
    let events: Vec<_> = (0..1000)
        .map(|index| {
            let mut message = template.clone();
            message.id = 6000 + index;
            message.channel_id = 100_000 + index * 2;
            message.author.id = 20_009 - index;
            store.set_messages(message.channel_id, Vec::new());
            Event::MessageCreate(Box::new(message))
        })
        .collect();
    let start = std::time::Instant::now();
    for event in events {
        assert!(matches!(
            store.apply(event),
            Change::MessageInserted { index: 0, .. }
        ));
    }
    let elapsed = start.elapsed();
    println!(
        "1000 MESSAGE_CREATE events / 2000 channels / 2000 read states / 20000 users: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "message apply took {elapsed:?}"
    );
    assert_eq!(
        store
            .guilds
            .iter()
            .flat_map(|g| g.channels())
            .filter(|c| c.unread)
            .count(),
        1000
    );
    assert!(store.guilds.iter().all(|g| g.unread));
}
