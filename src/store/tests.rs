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
fn component_and_formatting_fixtures_map_to_view() {
    let source: model::Message =
        serde_json::from_str(include_str!("fixtures/components_v2.json")).unwrap();
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
        serde_json::from_str(include_str!("fixtures/formatting.json")).unwrap();
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
fn embeds_map_kinds_text_fields_timestamps_and_proxy_only_media() {
    let wire: model::Embed = serde_json::from_value(json!({
        "type":"rich", "url":"https://example.com/post", "title":"Title", "description":"Description", "color":123,
        "timestamp":"2026-04-09T10:24:21Z", "provider":{"name":"Provider","url":"https://example.com"},
        "author":{"name":"Author","url":"https://example.com/author","icon_url":"https://evil.test/icon", "proxy_icon_url":"https://images-ext-2.discordapp.net/icon"},
        "footer":{"text":"Footer","icon_url":"https://cdn.discordapp.com/footer"},
        "fields":[{"name":"n","value":"v","inline":true}],
        "thumbnail":{"url":"https://evil.test/thumb","proxy_url":"https://images-ext-1.discordapp.net/thumb","width":20,"height":30},
        "image":{"url":"https://evil.test/image"}, "video":{"url":"https://media.discordapp.net/video"}
    })).unwrap();
    let embed = mapping::map_embed(&wire);
    assert!(embed.kind == EmbedKind::Rich);
    assert_eq!(
        embed.title.as_ref().unwrap().url.as_deref(),
        Some("https://example.com/post")
    );
    assert_eq!(embed.description.as_deref(), Some("Description"));
    assert_eq!(embed.color, Some(123));
    assert_eq!(embed.provider.as_ref().unwrap().text, "Provider");
    assert_eq!(
        embed.author.as_ref().unwrap().icon.as_ref().unwrap().width,
        0
    );
    assert_eq!(
        embed.footer.as_ref().unwrap().icon.as_ref().unwrap().height,
        0
    );
    assert!(embed.fields[0].inline);
    assert_eq!(
        embed.thumbnail.as_ref().unwrap().url,
        "https://images-ext-1.discordapp.net/thumb"
    );
    assert!(embed.images.is_empty());
    assert_eq!(embed.video.as_ref().unwrap().width, 0);
    assert!(embed.timestamp == local_timestamp("2026-04-09T10:24:21Z"));
    for (kind, expected) in [
        (None, EmbedKind::Rich),
        (Some(""), EmbedKind::Rich),
        (Some("image"), EmbedKind::Image),
        (Some("gifv"), EmbedKind::Gifv),
        (Some("video"), EmbedKind::Video),
        (Some("article"), EmbedKind::Link),
        (Some("link"), EmbedKind::Link),
        (Some("future"), EmbedKind::Link),
    ] {
        let mut wire = model::Embed {
            kind: kind.map(str::to_owned),
            ..Default::default()
        };
        assert!(mapping::map_embed(&wire).kind == expected);
        wire.color = Some(0);
        if matches!(kind, Some("article" | "link" | "future")) {
            assert!(mapping::map_embed(&wire).kind == EmbedKind::Rich);
            wire.color = None;
            wire.fields = vec![serde_json::from_value(json!({})).unwrap()];
            assert!(mapping::map_embed(&wire).kind == EmbedKind::Rich);
        }
    }
    let empty: model::Embed = serde_json::from_value(json!({"title":"","url":"","description":"","author":{"name":""},"provider":{"name":""},"footer":{"text":""},"timestamp":"bad"})).unwrap();
    let empty = mapping::map_embed(&empty);
    assert!(
        empty.title.is_none()
            && empty.url.is_none()
            && empty.description.is_none()
            && empty.author.is_none()
            && empty.provider.is_none()
            && empty.footer.is_none()
            && empty.timestamp.is_none()
    );
    for (url, proxy, allowed) in [
        ("https://evil.test/image", None, false),
        ("https://images-ext-1.discordapp.net/a", None, false),
        ("https://cdn.discordapp.com/a", None, true),
        (
            "https://media.discordapp.net/a",
            Some("https://evil.test/proxy"),
            false,
        ),
        (
            "https://evil.test/a",
            Some("https://images-ext-1.discordapp.net/a"),
            true,
        ),
        ("https://user@cdn.discordapp.com/a", None, false),
        ("http://cdn.discordapp.com/a", None, false),
    ] {
        let wire: model::Embed =
            serde_json::from_value(json!({"image":{"url":url,"proxy_url":proxy}})).unwrap();
        assert_eq!(!mapping::map_embed(&wire).images.is_empty(), allowed);
    }
}

#[test]
fn embed_galleries_are_consecutive_capped_and_suppressible() {
    let mut embeds = vec![
        json!({"url":"https://example.com/post","title":"gallery","image":{"url":"https://cdn.discordapp.com/first"}}),
    ];
    for index in 0..6 {
        embeds.push(json!({"url":"https://example.com/post","title":"","description":"","image":{"url":format!("https://cdn.discordapp.com/{index}")}}));
    }
    embeds.push(json!({"url":"https://example.com/other","image":{"url":"https://cdn.discordapp.com/other"}}));
    embeds.push(
        json!({"url":"https://example.com/post","image":{"url":"https://cdn.discordapp.com/last"}}),
    );
    let embeds: Vec<model::Embed> = model::parse_entries(json!(embeds));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 3);
    assert_eq!(mapped[0].images.len(), 4);
    assert_eq!(mapped[0].images[3].url, "https://cdn.discordapp.com/2");
    assert_eq!(mapped[2].images.len(), 1);
    assert!(mapping::map_embeds(&embeds, 1 << 2).is_empty());
}

#[test]
fn remaining_components_map_with_defaults_and_unsafe_media_is_dropped() {
    let wire: Vec<model::Component> = model::parse_entries(json!([
        {"type":3}, {"type":5,"placeholder":"pick","disabled":true}, {"type":6}, {"type":7}, {"type":8},
        {"type":13,"file":{"url":"attachment://a.txt"},"name":"a.txt","size":12,"spoiler":true},
        {"type":14}, {"type":14,"divider":false,"spacing":2},
        {"type":11,"media":{"url":"https://evil.test/a"}},
        {"type":12,"items":[{"media":{"url":"https://evil.test/a"}}, {"media":{"url":"https://media.discordapp.net/a"},"description":"alt","spoiler":true}]}
    ]));
    let mapped = mapping::map_components(&wire);
    assert_eq!(mapped.len(), 9);
    assert!(matches!(
        &mapped[0],
        Component::Select {
            placeholder: None,
            disabled: false
        }
    ));
    assert!(
        matches!(&mapped[1], Component::Select { placeholder: Some(text), disabled: true } if text == "pick")
    );
    assert!(
        matches!(&mapped[5], Component::File { name, size: 12, spoiler: true } if name == "a.txt")
    );
    assert!(matches!(
        &mapped[6],
        Component::Separator {
            divider: true,
            large: false
        }
    ));
    assert!(matches!(
        &mapped[7],
        Component::Separator {
            divider: false,
            large: true
        }
    ));
    assert!(
        matches!(&mapped[8], Component::Gallery(items) if items.len() == 1 && items[0].spoiler && items[0].description.as_deref() == Some("alt"))
    );
}

#[test]
fn message_updates_preserve_absent_fields_and_restore_suppressed_embeds() {
    let mut store = Store::from_ready(ready());
    let mut source: model::Message =
        serde_json::from_str(include_str!("fixtures/components_v2.json")).unwrap();
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
    assert_eq!(reactions[0].count, 3);
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

fn social_fixture(key: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(include_str!("../discord/fixtures/social.json"))
        .unwrap()[key]
        .clone()
}

fn member_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::MemberUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

fn role_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::RoleUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

#[test]
fn supplemental_chunk_names_colors_avatars_and_missing_members() {
    let mut store = Store::from_ready(ready());
    assert!(store.user(1).unwrap().status == Status::Online);
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    assert_eq!(store.member_name(200, 77), None);
    assert_eq!(store.missing_members(200, [1, 2, 2, 3, 4]), [2, 3, 4]);
    assert!(store.missing_members(200, [2, 3, 4]).is_empty());
    let supplemental = serde_json::from_value(social_fixture("supplemental")).unwrap();
    assert_eq!(
        store.apply(Event::ReadySupplemental(Box::new(supplemental))),
        Change::Members { guild_id: None }
    );
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    assert_eq!(store.member_name(100, 2), Some("Other Nick"));
    assert!(store.user(2).unwrap().status == Status::Idle);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("Example Game"));
    assert!(store.user(2).unwrap().playing);
    let chunk = serde_json::from_value(social_fixture("chunk")).unwrap();
    assert_eq!(
        store.apply(Event::MembersChunk(Box::new(chunk))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_name(200, 3), Some("Three"));
    assert!(
        matches!(store.member_avatar_spec(200,3), Some(ImageSpec::GuildAvatar { guild_id:200,user_id:3,hash }) if hash == "guild_hash")
    );
    assert!(store.user(3).unwrap().avatar.is_none());
    assert!(store.user(3).unwrap().status == Status::DoNotDisturb);
    assert_eq!(
        store.user(3).unwrap().activity.as_deref(),
        Some("Streaming Making things")
    );
    assert!(store.missing_members(200, [3, 4]).is_empty());
    for (id, position, color, primary) in [
        (201, 1, 0x112233, 0),
        (202, 3, 0x445566, 0x778899),
        (203, 9, 0, 0),
    ] {
        assert_eq!(
            role_event(
                &mut store,
                json!({"guild_id":"200","role":{"id":id.to_string(),"name":format!("Role {id}"),"permissions":"0","position":position,"color":color,"colors":{"primary_color":primary},"hoist":true}})
            ),
            Change::Structure
        );
    }
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"2"},"roles":["201","202","203"]})
        ),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_color(200, 2), Some(0x778899));
    assert_eq!(store.role(200, 202).unwrap().position, 3);
    assert!(store.role(200, 202).unwrap().hoist);
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"2"},"nick":null}),
    );
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"8","username":"username"},"roles":[]}),
    );
    assert_eq!(store.member_name(200, 8), Some("username"));
    assert_eq!(
        store.apply(Event::RoleDelete {
            guild_id: 200,
            role_id: 202
        }),
        Change::Structure
    );
    assert_eq!(store.member_color(200, 2), Some(0x112233));
    assert!(store.role(200, 202).is_none());
    store.apply(Event::Ready(Box::new(ready())));
    assert_eq!(store.missing_members(200, [4]), [4]);
}

#[test]
fn own_roles_recompute_permissions_and_member_removal() {
    let mut store = Store::from_ready(ready());
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":[]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
    assert!(
        store.source.as_ref().unwrap().guilds[0]
            .members
            .as_ref()
            .unwrap()[0]
            .roles
            .is_empty()
    );
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":["201"]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        store.apply(Event::MemberRemove(Box::new(
            serde_json::from_value(json!({"guild_id":"200","user":{"id":"1"}})).unwrap()
        ))),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
}

#[test]
fn member_list_all_ops_switching_and_item_presence() {
    let mut store = Store::from_ready(ready());
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Mods","color":42}}),
    );
    let list: model::MemberListUpdate = serde_json::from_value(social_fixture("list")).unwrap();
    assert_eq!(
        store.apply(Event::MemberListUpdate(Box::new(list))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    let rows = &store.member_list(200).unwrap().rows;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0],
        Some(MemberRow::Group {
            name: "Mods".into(),
            count: 1
        })
    );
    assert_eq!(rows[1], Some(MemberRow::Member { user_id: 2 }));
    assert_eq!(store.member_name(200, 2), Some("List Nick"));
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Renamed Mods","color":43}}),
    );
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Renamed Mods".into(),
            count: 1
        })
    );
    assert_eq!(
        store.user(2).unwrap().activity.as_deref(),
        Some("\u{1f3ae} grinding")
    );
    let mut update = social_fixture("list");
    update["ops"] = json!([
        {"op":"INSERT","index":1,"item":{"member":{"user":{"id":"8","username":"inserted"},"roles":[]}}},
        {"op":"UPDATE","index":2,"item":{"member":{"user":{"id":"9","username":"updated"},"roles":[]}}},
        {"op":"DELETE","index":3},
        {"op":"INVALIDATE","range":[0,0]}
    ]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(
        store.member_list(200).unwrap().rows,
        vec![
            None,
            Some(MemberRow::Member { user_id: 8 }),
            Some(MemberRow::Member { user_id: 9 }),
            Some(MemberRow::Member { user_id: 3 })
        ]
    );
    update["id"] = "different".into();
    update["ops"] = json!([{"op":"UPDATE","index":1,"item":{"member":{"user":{"id":"77","username":"ignored"}}}}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().id, "everyone");
    assert!(store.user(77).is_none());
    update["ops"] = json!([{"op":"SYNC","range":[2,3],"items":[{"group":{"id":"online","count":1}},{"member":{"user":{"id":"77","username":"synced"}}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    let list = store.member_list(200).unwrap();
    assert_eq!(list.id, "different");
    assert_eq!(list.rows[0], None);
    assert_eq!(
        list.rows[2],
        Some(MemberRow::Group {
            name: "Online".into(),
            count: 0
        })
    );
    assert_eq!(list.rows.len(), 4);
    update["groups"] = json!([{"id":"offline","count":1}]);
    update["ops"] =
        json!([{"op":"SYNC","range":[0,0],"items":[{"group":{"id":"unknown-role","count":1}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().rows.len(), 2);
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Role".into(),
            count: 1
        })
    );
}

#[test]
fn presence_activity_mapping_and_partial_users_preserve_names() {
    let mut store = Store::from_ready(ready());
    for (value, expected) in [
        ("online", Status::Online),
        ("idle", Status::Idle),
        ("dnd", Status::DoNotDisturb),
        ("offline", Status::Offline),
        ("invisible", Status::Offline),
    ] {
        let presence =
            serde_json::from_value(json!({"user":{"id":"2"},"status":value,"activities":[]}))
                .unwrap();
        assert_eq!(
            store.apply(Event::PresenceUpdate(Box::new(presence))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(2).unwrap().status == expected);
        assert_eq!(store.user(2).unwrap().name, "A Writer");
    }
    for (kind, expected) in [
        (0, "Example Game"),
        (1, "Streaming Example Game"),
        (2, "Listening to Example Game"),
        (3, "Watching Example Game"),
        (5, "Competing in Example Game"),
    ] {
        let presence = serde_json::from_value(
            json!({"user_id":"2","guild_id":"200","activities":[{"type":kind,"name":"Example Game"}]}),
        )
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        assert_eq!(store.user(2).unwrap().activity.as_deref(), Some(expected));
    }
    let presence = serde_json::from_value(json!({"user_id":"2","activities":[{"type":0,"name":"Game"},{"type":4,"state":"custom","emoji":{"id":"77","name":"custom_emoji"}}]})).unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
    let mut message = messages().remove(0);
    message.author.id = 2;
    message.author.username = "writer".into();
    store.set_messages(2002, vec![message]);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
}

#[test]
fn primary_guild_tag_maps_and_survives_users_without_the_field() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":true,"identity_guild_id":"9","tag":"BYND","badge":"flame"}})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let tag = store.user(2).unwrap().tag.clone().unwrap();
    assert_eq!((tag.text.as_str(), tag.guild_id), ("BYND", 9));
    assert!(tag.badge_spec().is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    assert!(store.user(2).unwrap().tag.is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":false,"tag":"BYND"}})).unwrap();
    store.set_messages(2002, vec![message]);
    assert!(store.user(2).unwrap().tag.is_none());
}

#[test]
fn decorations_survive_partial_authors_and_guild_nameplates_override() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "avatar_decoration_data": {"asset": "a_cat", "sku_id": "7"},
        "collectibles": {"nameplate": {"asset": "nameplates/mouse/1/", "palette": "berry"}},
        "display_name_styles": {"font_id": 14, "effect_id": 8, "colors": [1, 2]}
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert_eq!(user.decoration.as_ref().unwrap().asset, "a_cat");
    assert!(user.decoration_spec(true).is_some());
    assert_eq!(user.name_style.as_ref().unwrap().colors, [1, 2]);
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({"id": "2", "username": "writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({"id": "2", "global_name": "W"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_some() && user.nameplate.is_some() && user.name_style.is_some());

    member_event(
        &mut store,
        json!({
            "guild_id": "200",
            "user": {"id": "2"},
            "collectibles": {"nameplate": {"asset": "nameplates/rain/2/"}}
        }),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    assert_eq!(
        store.nameplate(Some(201), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "nick": "N"}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "collectibles": null}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({
        "id": "2",
        "global_name": "W",
        "avatar_decoration_data": null,
        "collectibles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "display_name_styles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_none() && user.nameplate.is_none() && user.name_style.is_none());
    assert!(store.nameplate(Some(200), 2).is_none());
}

#[test]
fn settings_own_status_partial_updates_do_not_reorder_folders() {
    let mut store = Store::from_ready(ready());
    let rail = format!("{:?}", store.rail);
    for (value, expected) in [
        ("idle", Status::Idle),
        ("invisible", Status::Offline),
        ("dnd", Status::DoNotDisturb),
        ("unknown", Status::Online),
    ] {
        let proto =
            settings_fixtures::status_fixture(Some(value), Some(("working", 0, "\u{1f3ae}", 0)));
        assert_eq!(
            store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
                settings: model::SettingsProto { kind: 1, proto },
                partial: true
            }))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(1).unwrap().status == expected);
        assert_eq!(
            store.user(1).unwrap().activity.as_deref(),
            Some("\u{1f3ae} working")
        );
        assert_eq!(format!("{:?}", store.rail), rail);
    }
    let proto = settings_fixtures::status_fixture(Some("idle"), None);
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert_eq!(
        store.user(1).unwrap().activity.as_deref(),
        Some("\u{1f3ae} working")
    );
    let proto = settings_fixtures::status_fixture(None, Some(("expired", 0, "", 1)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert!(store.user(1).unwrap().activity.is_none());
    assert!(store.user(1).unwrap().status == Status::Idle);
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
                content: "original".into(),
                attachments: true,
                embeds: true,
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
    store.set_messages(2002, vec![serde_json::from_value(forwarded).unwrap()]);
    assert!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0]
            .reply
            .is_none()
    );
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
fn all_channel_names_and_seed_social_data() {
    let mut store = Store::from_ready(ready());
    assert_eq!(store.channel_name(210), Some("Later"));
    assert_eq!(store.channel_name(2005), Some("stage"));
    assert_eq!(store.channel_name(2001), Some("hidden"));
    assert_eq!(store.channel_name(900), Some("A Writer"));
    let thread =
        serde_json::from_value(json!({"id":"777","guild_id":"200","type":11,"name":"thread"}))
            .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(thread)));
    assert_eq!(store.channel_name(777), Some("thread"));
    store.apply(Event::ChannelDelete { id: 777 });
    assert!(store.channel_name(777).is_none());
    let store = Store::seed();
    assert_eq!(store.member_name(1, 2), Some("Demo Two"));
    assert!(store.member_color(1, 1).is_some());
    assert!(
        store
            .guilds
            .iter()
            .all(|g| store.member_list(g.id).is_some())
    );
    let messages: Vec<_> = store
        .guilds
        .iter()
        .flat_map(|g| g.channels())
        .flat_map(|c| &c.messages)
        .collect();
    assert!(messages.iter().any(|m| m.kind == 7));
    assert!(messages.iter().any(|m| m.edited));
    assert!(
        messages
            .iter()
            .any(|m| matches!(m.reply, Some(ReplyState::Loaded(_))))
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

fn server_message(id: u64, channel_id: u64, author_id: u64, nonce: Option<u64>) -> model::Message {
    let mut value = json!({
        "id": id.to_string(), "channel_id": channel_id.to_string(),
        "author": {"id": author_id.to_string(), "username": "me"},
        "timestamp": "2026-10-22T12:00:00Z", "content": "hi"
    });
    if let Some(nonce) = nonce {
        value["nonce"] = json!(nonce.to_string());
    }
    serde_json::from_value(value).unwrap()
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
            index: 3
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
            removed: 2,
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

#[test]
fn channel_permissions_gate_sending_and_managing() {
    let store = Store::from_ready(ready());
    assert!(!store.can_send(2002));
    assert!(!store.can_manage_messages(2002));
    assert!(store.can_send(1001));
    assert!(store.can_manage_messages(1001));
    let demo = Store::seed();
    assert!(demo.can_send(0));
    assert!(demo.can_manage_messages(0));
}

#[test]
fn typing_start_registers_member_without_state_change() {
    let mut store = Store::from_ready(ready());
    let typing: model::TypingStart = serde_json::from_value(json!({
        "channel_id": "2002", "guild_id": "200", "user_id": "77",
        "timestamp": 1770000000,
        "member": {"user": {"id": "77", "username": "typer"}, "roles": []}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::TypingStart(Box::new(typing))),
        Change::None
    );
    assert_eq!(store.member_name(200, 77), Some("typer"));
}
