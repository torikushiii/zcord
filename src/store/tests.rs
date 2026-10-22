use super::*;
use crate::discord::{Event, settings_fixtures};
use crate::images::ImageSpec;
use chrono::{Datelike, Timelike};

use serde_json::json;

fn ready() -> model::Ready {
    serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap()
}
fn messages() -> Vec<model::Message> {
    model::parse_entries(
        serde_json::from_str(include_str!("../discord/fixtures/messages.json")).unwrap(),
    )
}

#[test]
fn rail_folders_ready_join_delete_and_settings_updates() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    payload["user_settings_proto"] = settings_fixtures::fixture(true).into();
    let mut ready: model::Ready = serde_json::from_value(payload).unwrap();
    ready
        .guilds
        .push(serde_json::from_value(json!({"id":"300","name":"New","owner_id":"1"})).unwrap());
    let mut store = Store::from_ready(ready);
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(300),
            RailEntry::Guild(100),
            RailEntry::Folder(Folder {
                id: 42,
                name: Some("Gaming".into()),
                color: Some(0x5865F2),
                guild_ids: vec![200],
            })
        ]
    );
    assert_eq!(store.guild_by_id(100).unwrap().name, "First");
    let guild = serde_json::from_value(json!({"id":"400","name":"Joined","owner_id":"1"})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(guild)));
    assert_eq!(store.rail[0], RailEntry::Guild(400));
    let before = format!("{:?}", store.rail);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([8, 1])
            },
            partial: true,
        }))),
        Change::None
    );
    assert_eq!(format!("{:?}", store.rail), before);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([114, 0])
            },
            partial: true,
        }))),
        Change::Structure
    );
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(200),
            RailEntry::Guild(100),
            RailEntry::Guild(300)
        ]
    );
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: settings_fixtures::fixture(false),
        },
        partial: false,
    })));
    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.guild_by_id(200).is_none());
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(300),
            RailEntry::Guild(100)
        ]
    );
    let seed = Store::seed();
    assert!(
        matches!(&seed.rail[1], RailEntry::Folder(folder) if folder.name.as_deref() == Some("Gaming") && folder.guild_ids == [2, 3])
    );
}

#[test]
fn avatar_defaults_and_image_attachments() {
    let mut user = map_user(&ready().user);
    user.id = 5 << 22;
    for discriminator in [None, Some("0".to_owned())] {
        user.discriminator = discriminator;
        assert!(matches!(
            user.avatar_spec(),
            ImageSpec::DefaultAvatar { index: 5 }
        ));
    }
    user.discriminator = Some("1234".into());
    assert!(matches!(
        user.avatar_spec(),
        ImageSpec::DefaultAvatar { index: 4 }
    ));
    user.avatar = Some("a_hash".into());
    assert!(matches!(user.avatar_spec(), ImageSpec::Avatar { .. }));
    let mut store = Store::from_ready(ready());
    assert!(matches!(
        store.guilds[0].icon_spec(),
        Some(ImageSpec::GuildIcon { guild_id: 200, .. })
    ));
    store.set_messages(2002, messages());
    let update = serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[{
        "id":"77","filename":"image.GIF","url":"https://cdn.discordapp.com/attachments/1/2/image.gif",
        "proxy_url":"https://media.discordapp.net/attachments/1/2/image.gif", "width":400,"height":200,"size":42
    }, {"id":"bad"}]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[1];
    assert_eq!(message.attachments.len(), 1);
    assert!(matches!(
        message.attachments[0].image_spec(),
        Some(ImageSpec::Attachment {
            id: 77,
            width: 400,
            height: 200,
            ..
        })
    ));
    let mut attachment = message.attachments[0].clone();
    attachment.width = None;
    assert!(attachment.image_spec().is_none());
    attachment.width = Some(400);
    attachment.filename = "not-image.pdf".into();
    assert!(attachment.image_spec().is_none());
    attachment.content_type = Some("image/png".into());
    assert!(attachment.image_spec().is_some());
    let update =
        serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert!(
        store.guilds[0].channel(2002).unwrap().messages[1]
            .attachments
            .is_empty()
    );
}

#[test]
fn ready_mapping_order_permissions_and_badges() {
    let ready = ready();
    assert_eq!(
        ready.guilds[1].members.as_ref().unwrap()[0].roles,
        Vec::<u64>::new()
    );
    let mut store = Store::seed();
    assert_eq!(
        store.apply(Event::Ready(Box::new(ready))),
        Change::Structure
    );
    assert_eq!(
        store.guilds.iter().map(|g| g.id).collect::<Vec<_>>(),
        [200, 100]
    );
    let guild = &store.guilds[0];
    assert_eq!(
        guild
            .categories
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["", "Earlier", "Later"]
    );
    assert_eq!(
        guild.categories[1]
            .channels
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [2003, 2002]
    );
    assert!(guild.channel(2001).is_none());
    assert!(guild.channel(2002).unwrap().unread);
    assert!(!guild.channel(2003).unwrap().unread);
    assert!(!guild.channel(2004).unwrap().unread);
    assert!(guild.channel(2007).is_none());
    assert!(guild.channel(2008).is_none());
    assert_eq!(guild.mentions, 3);
    assert!(guild.unread);
    assert_eq!(store.dm_mentions, 4);
    assert_eq!(guild.initials, "SGN");
    assert_eq!(guild.icon.as_deref(), Some("guild-hash"));
    assert!(store.guilds[1].channel(1001).is_some());
    assert_eq!(store.user(2).unwrap().name, "A Writer");
    assert_eq!(
        store.user(2).unwrap().avatar.as_deref(),
        Some("avatar-hash")
    );
}

#[test]
fn message_history_order_local_time_and_first_embed() {
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
    let embed = channel.messages[1].embed.as_ref().unwrap();
    assert_eq!(embed.title, "Status");
    assert_eq!(embed.lines, ["one", "two", "Count: 3"]);
    assert_eq!(embed.footer, "footer");
    assert_eq!(embed.color, 0x202225);
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
    assert!(updated.embed.is_none());
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
fn partial_structure_updates_preserve_history_and_permissions() {
    let ready = ready();
    let source_users = ready.users.as_ptr();
    let mut store = Store::from_ready(ready);
    assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
    let other_guild_name = store.guilds[1].name.as_ptr();
    store.set_messages(2002, messages());
    let update =
        serde_json::from_value(json!({"id":"2002", "guild_id":"200", "name":"renamed"})).unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
    assert_eq!(store.guilds[1].name.as_ptr(), other_guild_name);
    assert_eq!(store.guilds[0].channel(2002).unwrap().name, "renamed");
    assert_eq!(store.guilds[0].channel(2002).unwrap().messages.len(), 2);
    let guild = serde_json::from_value(json!({"id":"200", "name":"Changed", "icon":null})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(guild)));
    assert_eq!(store.guilds[0].name, "Changed");
    assert!(store.guilds[0].icon.is_none());
    assert!(store.guilds[0].channel(2001).is_none());
    let hide = serde_json::from_value(json!({"id":"2002", "permission_overwrites":[{"id":"1", "type":1, "allow":"0", "deny":"1024"}]})).unwrap();
    store.apply(Event::ChannelUpsert(Box::new(hide)));
    assert!(store.guilds[0].channel(2002).is_none());
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

#[test]
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
        })
        .collect();
    ready.users = (0..20_000)
        .map(|index| model::User {
            id: 10 + index,
            username: format!("user-{index}"),
            global_name: None,
            avatar: None,
            discriminator: None,
            bot: false,
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
