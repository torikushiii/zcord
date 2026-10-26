use super::*;

#[test]
fn voice_channels_follow_text_channels_within_a_category() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    let guild = &mut payload["guilds"][0];
    let owner = guild["properties"]["owner_id"].clone();
    guild["channels"] = json!([
        {"id": "900", "type": 4, "name": "Category", "position": 0},
        {"id": "901", "type": 2, "name": "voice-first", "position": 0, "parent_id": "900"},
        {"id": "902", "type": 0, "name": "text-second", "position": 1, "parent_id": "900"},
        {"id": "903", "type": 13, "name": "stage", "position": 2, "parent_id": "900"},
        {"id": "904", "type": 5, "name": "news-last", "position": 3, "parent_id": "900"},
        {"id": "905", "type": 15, "name": "forum", "position": 0, "parent_id": "900"},
        {"id": "906", "type": 16, "name": "media-last", "position": 4, "parent_id": "900"},
    ]);
    payload["user"]["id"] = owner;
    let ready: model::Ready = serde_json::from_value(payload).unwrap();
    let guild_id = ready.guilds[0].id;
    let store = Store::from_ready(ready);
    let guild = store.guilds.iter().find(|g| g.id == guild_id).unwrap();
    let names: Vec<_> = guild
        .categories
        .iter()
        .flat_map(|category| category.channels.iter().map(|c| c.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            "forum",
            "text-second",
            "news-last",
            "media-last",
            "voice-first",
            "stage"
        ]
    );
    assert!(guild.channel(905).unwrap().kind.is_forum());
    assert!(guild.channel(906).unwrap().kind.is_forum());
}

#[test]
fn guild_banner_comes_from_ready_and_updates_and_clears() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    payload["guilds"][0]["properties"]["banner"] = json!("a_ready_banner");
    let ready: model::Ready = serde_json::from_value(payload).unwrap();
    let guild_id = ready.guilds[0].id;
    let mut store = Store::from_ready(ready);
    assert_eq!(
        store.guild_banner(guild_id),
        Some(ImageSpec::GuildBanner {
            guild_id,
            hash: "a_ready_banner".into(),
        })
    );

    let update: model::Guild =
        serde_json::from_value(json!({"id": guild_id.to_string(), "banner": "updated"})).unwrap();
    assert_eq!(
        store.apply(Event::GuildUpsert(Box::new(update))),
        Change::Structure
    );
    assert_eq!(
        store.guild_banner(guild_id),
        Some(ImageSpec::GuildBanner {
            guild_id,
            hash: "updated".into(),
        })
    );

    let clear: model::Guild =
        serde_json::from_value(json!({"id": guild_id.to_string(), "banner": null})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(clear)));
    assert_eq!(store.guild_banner(guild_id), None);
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
    assert!(guild.channel(2007).is_some());
    assert!(guild.channel(2007).unwrap().kind.is_forum());
    assert!(guild.channel(2008).is_none());
    assert_eq!(guild.channel(2002).unwrap().mentions, 2);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.channel(2004).unwrap().mentions, 0);
    assert!(guild.channel(2006).unwrap().kind == ChannelKind::Announcement);
    assert_eq!(guild.mentions, 3);
    assert_eq!(
        guild
            .channels()
            .map(|channel| channel.mentions)
            .sum::<u32>(),
        guild.mentions
    );
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
fn channel_privacy_follows_the_everyone_view_overwrite() {
    let store = Store::from_ready(ready());
    let guild = store.guild_by_id(200).unwrap();
    assert!(guild.channel(2002).unwrap().private);
    assert!(!guild.channel(2003).unwrap().private);
    assert!(
        store
            .guild_by_id(100)
            .unwrap()
            .channel(1001)
            .unwrap()
            .private
    );

    let mut ready = ready();
    let channel = ready.guilds[0]
        .channels
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|channel| channel.id == 2003)
        .unwrap();
    channel.permission_overwrites = Some(vec![model::Overwrite {
        id: 200,
        kind: 0,
        allow: discord::VIEW_CHANNEL,
        deny: 0,
    }]);
    let store = Store::from_ready(ready);
    assert!(
        !store
            .guild_by_id(200)
            .unwrap()
            .channel(2003)
            .unwrap()
            .private
    );
}

#[test]
fn forum_channels_map_tags_posts_and_new_post_counts() {
    let threads = vec![
        forum_thread(
            3001,
            2007,
            "First post",
            Some(3001),
            "2026-10-20T12:00:00Z",
            0,
            2,
        ),
        forum_thread(
            3002,
            2007,
            "Second post",
            Some(3002),
            "2026-10-21T12:00:00Z",
            0,
            4,
        ),
    ];
    let store = Store::from_ready(forum_ready(threads, Some(3000), None));
    let guild = store.guild_by_id(200).unwrap();
    let forum = guild.channel(2007).unwrap();
    assert!(forum.kind.is_forum());
    assert_eq!(forum.new_posts, 2);
    assert!(forum.unread);
    assert!(guild.channel(2009).unwrap().kind.is_forum());

    let tags = store.forum_tags(2007);
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].name, "Help");
    assert_eq!(
        tags[0].emoji,
        Some(StatusEmoji::Unicode("\u{1F9E0}".into()))
    );
    assert!(tags[1].moderated);
    assert!(matches!(
        tags[1].emoji,
        Some(StatusEmoji::Custom { id: 555, .. })
    ));

    let posts = store.forum_posts(2007);
    assert_eq!(
        posts
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["Second post", "First post"]
    );
    assert!(guild.channel(3001).is_some());
    assert!(store.can_send(3001));
    assert!(!store.can_send(2007));
}

#[test]
fn forum_new_posts_track_thread_create_and_ack() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Existing",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        1,
    )];
    let mut store = Store::from_ready(forum_ready(threads, Some(3000), None));
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2007)
            .unwrap()
            .new_posts,
        1
    );

    let thread: model::Channel = serde_json::from_value(forum_thread(
        3002,
        2007,
        "Live post",
        Some(3002),
        "2026-10-22T12:00:00Z",
        0,
        0,
    ))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(thread)));
    let forum = store.guild_by_id(200).unwrap().channel(2007).unwrap();
    assert_eq!(forum.new_posts, 2);
    assert!(forum.unread);

    store.apply(Event::MessageAck {
        channel_id: 2007,
        message_id: 3002,
    });
    let forum = store.guild_by_id(200).unwrap().channel(2007).unwrap();
    assert_eq!(forum.new_posts, 0);
    assert!(!forum.unread);
}

#[test]
fn forum_posts_order_by_pinned_activity_and_creation() {
    let threads = vec![
        forum_thread(
            3001,
            2007,
            "older",
            Some(1_200_000_000_000_000_000),
            "2026-10-20T12:00:00Z",
            0,
            3,
        ),
        forum_thread(
            3002,
            2007,
            "pinned",
            Some(1_100_000_000_000_000_000),
            "2026-10-21T12:00:00Z",
            2,
            2,
        ),
        forum_thread(
            3003,
            2007,
            "newest",
            Some(1_000_000_000_000_000_000),
            "2026-10-22T12:00:00Z",
            0,
            1,
        ),
    ];
    let store = Store::from_ready(forum_ready(threads.clone(), Some(3000), None));
    assert_eq!(
        store
            .forum_posts(2007)
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["pinned", "older", "newest"]
    );

    let store = Store::from_ready(forum_ready(threads, Some(3000), Some(1)));
    assert_eq!(
        store
            .forum_posts(2007)
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["pinned", "newest", "older"]
    );
}

#[test]
fn forum_posts_merge_ready_search_and_thread_events() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Ready post",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        1,
    )];
    let mut store = Store::from_ready(forum_ready(threads, Some(3000), None));
    let search: model::ForumSearch = serde_json::from_value(json!({
        "threads": [forum_thread(
            3002, 2007, "Search post", Some(3400), "2026-10-21T12:00:00Z", 0, 5
        )],
        "first_messages": [{
            "id": "3401",
            "channel_id": "3002",
            "author": {"id": "2", "username": "writer"},
            "timestamp": "2026-10-21T12:00:00Z",
            "content": "first **body**",
            "attachments": [{
                "id": "77",
                "filename": "image.png",
                "url": "https://cdn.discordapp.com/attachments/1/77/image.png",
                "proxy_url": "https://media.discordapp.net/attachments/1/77/image.png",
                "width": 400,
                "height": 200,
                "content_type": "image/png"
            }]
        }],
        "has_more": true
    }))
    .unwrap();
    store.apply_forum_search(2007, search);
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3002)
        .unwrap();
    assert_eq!(post.name, "Search post");
    assert_eq!(post.preview.as_deref(), Some("first **body**"));
    assert!(matches!(
        post.thumbnail,
        Some(ImageSpec::Attachment {
            id: 77,
            width: 400,
            height: 200,
            ..
        })
    ));

    let search: model::ForumSearch = serde_json::from_value(json!({
        "threads": [forum_thread(
            3002, 2007, "Renamed", Some(3400), "2026-10-21T12:00:00Z", 0, 5
        )],
        "has_more": false
    }))
    .unwrap();
    store.apply_forum_search(2007, search);
    let posts = store.forum_posts(2007);
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts
            .iter()
            .find(|post| post.id == 3002)
            .unwrap()
            .preview
            .as_deref(),
        Some("first **body**")
    );

    let update: model::Channel = serde_json::from_value(json!({
        "id": "3002", "type": 11, "guild_id": "200", "parent_id": "2007",
        "name": "Edited", "flags": 2, "message_count": 9, "applied_tags": ["4002"],
        "last_message_id": "3500",
        "thread_metadata": {
            "archived": true, "locked": true,
            "create_timestamp": "2026-10-21T12:00:00Z"
        }
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3002)
        .unwrap();
    assert_eq!(post.name, "Edited");
    assert!(post.pinned && post.locked && post.archived);
    assert_eq!(post.tag_ids, [4002]);
    assert_eq!(post.message_count, 9);

    let sync: model::ThreadListSync = serde_json::from_value(json!({
        "guild_id": "200",
        "threads": [forum_thread(
            3003, 2007, "Synced", Some(3003), "2026-10-22T12:00:00Z", 0, 0
        )]
    }))
    .unwrap();
    store.apply(Event::ThreadListSync(Box::new(sync)));
    assert!(store.forum_posts(2007).iter().any(|post| post.id == 3003));

    store.apply(Event::ChannelDelete { id: 3002 });
    assert!(!store.forum_posts(2007).iter().any(|post| post.id == 3002));
}

#[test]
fn thread_message_create_bumps_forum_post_and_thread_channel_loads() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Body",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        2,
    )];
    let mut ready = forum_ready(threads, Some(3000), None);
    ready.read_state.entries.push(model::ReadState {
        id: 3001,
        last_message_id: Some(3000),
        mention_count: 1,
        kind: 0,
    });
    let mut store = Store::from_ready(ready);
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(thread.unread);
    assert_eq!(thread.mentions, 1);
    let message: model::Message = serde_json::from_value(json!({
        "id": "3100",
        "channel_id": "3001",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T12:00:00Z",
        "content": "the post body",
        "attachments": [{
            "id": "88",
            "filename": "photo.jpg",
            "url": "https://cdn.discordapp.com/attachments/1/88/photo.jpg",
            "proxy_url": "https://media.discordapp.net/attachments/1/88/photo.jpg",
            "width": 300,
            "height": 500,
            "content_type": "image/jpeg"
        }]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message.clone())));
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3001)
        .unwrap();
    assert_eq!(post.message_count, 3);
    assert_eq!(
        post.last_activity,
        local_timestamp("2026-10-22T12:00:00Z").unwrap()
    );
    assert_eq!(post.preview.as_deref(), Some("the post body"));
    assert!(matches!(
        post.thumbnail,
        Some(ImageSpec::Attachment { id: 88, .. })
    ));
    assert!(post.unread);
    assert!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(3001)
            .unwrap()
            .unread
    );

    store.set_messages(3001, vec![message]);
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(thread.messages_loaded);
    assert_eq!(thread.messages.len(), 1);
    assert!(store.at_present(3001));
    assert!(store.can_send(3001));

    store.apply(Event::MessageAck {
        channel_id: 3001,
        message_id: 3100,
    });
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(!thread.unread);
    assert_eq!(thread.mentions, 0);
    assert!(!store.forum_posts(2007)[0].unread);
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
fn emoji_api_access_rules() {
    let store = Store::from_ready(ready());
    let usable = custom_emoji(1, 200, "usable");
    assert_eq!(store.emoji_access(&usable, 200, 2002), EmojiAccess::Usable);

    let mut unavailable = usable.clone();
    unavailable.available = false;
    assert_eq!(
        store.emoji_access(&unavailable, 200, 2002),
        EmojiAccess::Unavailable
    );

    let mut restricted = usable.clone();
    restricted.roles = vec![999];
    assert_eq!(
        store.emoji_access(&restricted, 200, 2002),
        EmojiAccess::NeedsRole
    );

    let mut animated = usable.clone();
    animated.animated = true;
    assert_eq!(
        store.emoji_access(&animated, 200, 2002),
        EmojiAccess::NeedsNitro
    );

    let external = custom_emoji(2, 100, "external");
    assert_eq!(
        store.emoji_access(&external, 200, 2002),
        EmojiAccess::NeedsNitro
    );

    let mut ready = ready();
    ready.user.premium_type = Some(2);
    let store = Store::from_ready(ready);
    assert!(store.has_nitro());
    assert_eq!(
        store.emoji_access(&animated, 200, 2002),
        EmojiAccess::Usable
    );
    assert_eq!(
        store.emoji_access(&external, 200, 2002),
        EmojiAccess::NoExternalPermission
    );
}

#[test]
fn emoji_api_search_orders_current_guild_then_others_then_unicode() {
    let mut ready = ready();
    ready
        .guilds
        .iter_mut()
        .find(|guild| guild.id == 200)
        .unwrap()
        .emojis = Some(vec![
        guild_emoji(1, "qqq_prefix"),
        guild_emoji(2, "xqqq_sub"),
    ]);
    ready
        .guilds
        .iter_mut()
        .find(|guild| guild.id == 100)
        .unwrap()
        .emojis = Some(vec![guild_emoji(3, "qqq_other")]);
    let store = Store::from_ready(ready);

    let results = store.search_emoji("qqq", 200, 2002, 10);
    let names: Vec<_> = results
        .iter()
        .map(|(choice, _)| match choice {
            EmojiChoice::Custom(emoji) => emoji.name.as_str(),
            EmojiChoice::Unicode { shortcode, .. } => shortcode.as_str(),
        })
        .collect();
    assert_eq!(names, ["qqq_prefix", "xqqq_sub", "qqq_other"]);
    assert_eq!(results[2].1, EmojiAccess::NeedsNitro);
    assert!(store.search_emoji("", 200, 2002, 10).is_empty());

    let unicode = store.search_emoji("wave", 200, 2002, 3);
    assert!(matches!(
        &unicode[0].0,
        EmojiChoice::Unicode { shortcode, .. } if shortcode == "wave"
    ));
    assert_eq!(unicode[0].1, EmojiAccess::Usable);
}

#[test]
fn emoji_api_guild_emojis_update_replaces_and_skips_malformed() {
    let mut store = Store::from_ready(ready());
    let update: model::GuildEmojisUpdate = serde_json::from_value(json!({
        "guild_id": "200",
        "emojis": [
            {"id":"9001","name":"party","animated":true,"available":false,
             "roles":["7"],"require_colons":true},
            {"name":"missing-id"},
            {"id":"bad","name":"bad-id"},
            {"id":"9002","name":"plain"}
        ]
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::GuildEmojisUpdate(update)),
        Change::Structure
    );
    let guild = store.guild_by_id(200).unwrap();
    let names: Vec<_> = guild
        .emojis
        .iter()
        .map(|emoji| emoji.name.as_str())
        .collect();
    assert_eq!(names, ["party", "plain"]);
    assert!(guild.emojis[0].animated && !guild.emojis[0].available);
    assert_eq!(guild.emojis[0].roles, [7]);

    let update: model::GuildEmojisUpdate = serde_json::from_value(json!({
        "guild_id": "200",
        "emojis": [{"id":"9003","name":"replacement"}]
    }))
    .unwrap();
    store.apply(Event::GuildEmojisUpdate(update));
    let names: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .emojis
        .iter()
        .map(|emoji| emoji.name.as_str())
        .collect();
    assert_eq!(names, ["replacement"]);
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
fn channel_permission_memo_is_reused_and_invalidated_by_role_updates() {
    let mut store = Store::from_ready(ready());
    assert!(!store.can_send(2002));
    assert_eq!(
        store.permission_cache.borrow().get(&2002),
        Some(&crate::discord::VIEW_CHANNEL)
    );
    assert!(!store.can_send(2002));
    assert_eq!(
        store.permission_cache.borrow().get(&2002),
        Some(&crate::discord::VIEW_CHANNEL)
    );

    assert_eq!(
        role_event(
            &mut store,
            json!({"guild_id":"200","role":{"id":"201","name":"Members","permissions":"2048","position":1}})
        ),
        Change::Structure
    );
    assert!(store.permission_cache.borrow().is_empty());
    assert!(store.can_send(2002));
    assert!(store.can_send(2004));

    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":[]})
        ),
        Change::Structure
    );
    assert!(store.permission_cache.borrow().is_empty());
    assert!(!store.can_send(2004));
}
