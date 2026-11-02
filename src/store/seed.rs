use super::*;

const STATUS_BOT_ID: u64 = 9;

impl Store {
    pub fn seed() -> Self {
        use Status::*;
        let users = [
            (
                "Demo User One",
                "D1",
                Online,
                false,
                Some(0xC77DFF),
                Some("DEMO"),
                Some("Building a Discord client"),
            ),
            (
                "Demo User Two",
                "D2",
                Online,
                false,
                Some(0x3BA55C),
                None,
                Some("Playing Example Adventure"),
            ),
            ("Demo User Three", "D3", Idle, false, None, None, None),
            (
                "Demo User Four",
                "D4",
                DoNotDisturb,
                false,
                Some(0xE67E22),
                Some("TEST"),
                Some("Focus mode"),
            ),
            ("Demo User Five", "D5", Online, false, None, None, None),
            ("Demo User Six", "D6", Offline, false, None, None, None),
            (
                "Demo User Seven",
                "D7",
                Offline,
                false,
                Some(0x3BA55C),
                None,
                None,
            ),
            ("Demo User Eight", "D8", Offline, false, None, None, None),
            (
                "Demo Status Bot",
                "SB",
                Online,
                true,
                Some(0xF23F43),
                None,
                Some("Watching over 100 demo users"),
            ),
            (
                "Demo Helper Bot",
                "HB",
                Offline,
                true,
                Some(0xF23F43),
                None,
                None,
            ),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(index, (name, initials, status, bot, role_color, tag, activity))| User {
                id: index as u64 + 1,
                username: name.into(),
                name: name.into(),
                initials: initials.into(),
                status,
                bot,
                role_color,
                tag: tag.map(|text: &str| GuildTag {
                    text: text.into(),
                    guild_id: 0,
                    badge: None,
                }),
                playing: false,
                activity: activity.map(Into::into),
                activity_kind: None,
                game: None,
                listening: None,
                live_stream: None,
                on_mobile: false,
                custom_status: None,
                avatar: None,
                discriminator: None,
                decoration: None,
                nameplate: None,
                name_style: None,
            },
        )
        .collect::<Vec<_>>();

        let snippets = [
            "Good morning, <@1>! **The next build is ready.** :smile:",
            "The plan is in <#100>. Thanks to <@&1001> for the review!",
            "That sounds great. ||The surprise is a new theme.||",
            "```rust\nfn main() { println!(\"hello\"); }\n```",
            "> Keep the important notes in the channel topic.\nAgreed! [Rust documentation](https://doc.rust-lang.org/)",
            "Just got back from a walk. The weather is finally getting cooler here.",
            "Sharing a longer update for anyone catching up: the first draft is ready, the feedback has been collected, and the next step is to review the details together.",
            "Nice work, everyone!",
            "lol",
            "anyone up for raids tonight?",
        ];
        let human_ids = [1_u64, 2, 3, 4, 5, 6, 7];

        let guild_specs: [(&str, &str, u32, &[&str], bool, u32); 6] = [
            (
                "Demo Server One",
                "S1",
                0x5865F2,
                &["general", "introductions", "projects", "show-and-tell"],
                true,
                0,
            ),
            (
                "Demo Server Two",
                "S2",
                0xE67E22,
                &["general", "raids", "gacha-salt", "fan-art"],
                true,
                790,
            ),
            (
                "Demo Server Three",
                "S3",
                0x2F3136,
                &["general", "music", "off-topic"],
                true,
                329,
            ),
            (
                "Demo Server Four",
                "S4",
                0xEB459E,
                &["general", "design", "inspiration", "feedback", "resources"],
                false,
                0,
            ),
            (
                "Demo Server Five",
                "S5",
                0xCE422B,
                &["general", "help", "showcase"],
                true,
                1200,
            ),
            (
                "Demo Server Six",
                "S6",
                0x23A55A,
                &["general", "schedule"],
                false,
                89,
            ),
        ];

        let mut next_message_id = 1;
        let guilds = guild_specs
            .into_iter()
            .enumerate()
            .map(
                |(guild_index, (name, initials, icon_color, channel_names, unread, mentions))| {
                    let guild_id = guild_index as u64 + 1;
                    let text_channels = channel_names
                        .iter()
                        .enumerate()
                        .map(|(channel_index, channel_name)| {
                            let count = if channel_index == 0 {
                                30
                            } else {
                                6 + (guild_index * 7 + channel_index * 5) % 20
                            };
                            let messages = seed_messages(
                                count,
                                guild_index + channel_index,
                                &human_ids,
                                &snippets,
                                &mut next_message_id,
                            );
                            Channel {
                                id: guild_id * 100 + channel_index as u64,
                                name: (*channel_name).into(),
                                kind: ChannelKind::Text,
                                parent_id: Some(guild_id * 10),
                                private: false,
                                unread: unread && channel_index % 2 == 1,
                                mentions: if channel_index == 0 { mentions } else { 0 },
                                new_posts: 0,
                                messages,
                                messages_loaded: true,
                            }
                        })
                        .collect();
                    let voice_channels = ["General", "Raid Room"]
                        .into_iter()
                        .take(1 + guild_index % 2)
                        .enumerate()
                        .map(|(index, name)| Channel {
                            id: guild_id * 100 + 50 + index as u64,
                            name: name.into(),
                            kind: ChannelKind::Voice,
                            parent_id: Some(guild_id * 10 + 1),
                            private: false,
                            unread: false,
                            mentions: 0,
                            new_posts: 0,
                            messages: Vec::new(),
                            messages_loaded: false,
                        })
                        .collect();

                    Guild {
                        id: guild_id,
                        name: name.into(),
                        initials: initials.into(),
                        icon_color,
                        unread,
                        mentions,
                        premium_tier: 0,
                        categories: vec![
                            Category {
                                id: guild_id * 10,
                                name: "Text Channels".into(),
                                channels: text_channels,
                            },
                            Category {
                                id: guild_id * 10 + 1,
                                name: "Voice Channels".into(),
                                channels: voice_channels,
                            },
                        ],
                        threads: Vec::new(),
                        member_ids: users.iter().map(|user| user.id).collect(),
                        icon: None,
                        banner: None,
                        emojis: Vec::new(),
                    }
                },
            )
            .collect();

        let mut store = Self {
            guilds,
            rail: Vec::new(),
            users: users.into_iter().map(|user| (user.id, user)).collect(),
            current_user_id: 1,
            account: None,
            dm_mentions: 77,
            next_message_id,
            premium_type: None,
            source: None,
            read_states: HashMap::new(),
            source_channels: HashMap::new(),
            channel_locations: HashMap::new(),
            thread_locations: HashMap::new(),
            forum_posts: HashMap::new(),
            thread_recent: HashMap::new(),
            thread_joins: HashMap::new(),
            guild_badges: Vec::new(),
            dm_badges: 77,
            private: Vec::new(),
            private_locations: HashMap::new(),
            relationships: HashMap::new(),
            notification_settings: HashMap::new(),
            guild_notification_defaults: HashMap::new(),
            request_previews: HashMap::new(),
            friend_suggestion_count: 0,
            guild_indices: HashMap::new(),
            folder_layout: Some(vec![
                settings_proto::GuildFolder {
                    id: None,
                    name: None,
                    color: None,
                    guild_ids: vec![1],
                },
                settings_proto::GuildFolder {
                    id: Some(1),
                    name: Some("Gaming".into()),
                    color: Some(0x5865F2),
                    guild_ids: vec![2, 3],
                },
                settings_proto::GuildFolder {
                    id: None,
                    name: None,
                    color: None,
                    guild_ids: vec![4, 5, 6],
                },
            ]),
            frecency: None,
            favorite_gifs: Vec::new(),
            rail_order: vec![1, 2, 3, 4, 5, 6],
            social: social::Social::default(),
            voice_start_times: HashMap::new(),
            permission_cache: RefCell::new(HashMap::new()),
        };
        for guild in &mut store.guilds {
            let moderator = guild.id * 1000 + 1;
            let member_role = guild.id * 1000 + 2;
            let mut social = social::GuildSocial::default();
            social.roles.insert(
                moderator,
                Role {
                    id: moderator,
                    name: "Moderators".into(),
                    color: Some(0x3BA55C),
                    position: 10,
                    hoist: true,
                },
            );
            social.roles.insert(
                member_role,
                Role {
                    id: member_role,
                    name: "Members".into(),
                    color: Some(0xF0B232),
                    position: 1,
                    hoist: true,
                },
            );
            for user in store.users.values() {
                let role = match user.id {
                    1 | 4 => Some(moderator),
                    2 | 3 => Some(member_role),
                    _ => None,
                };
                social.members.insert(
                    user.id,
                    social::MemberInfo {
                        nick: match user.id {
                            2 => Some("Demo Two".into()),
                            4 => Some("Demo Four".into()),
                            _ => None,
                        },
                        roles: role.into_iter().collect(),
                        avatar: None,
                        color: role
                            .and_then(|id| social.roles.get(&id))
                            .and_then(|r| r.color),
                        nameplate: None,
                        decoration: None,
                        name_style: None,
                    },
                );
            }
            let mut rows = Vec::new();
            for (name, ids) in [
                ("Moderators", vec![1, 4]),
                ("Members", vec![2, 3]),
                ("Online", vec![5, 9]),
                ("Offline", vec![6, 7, 8, 10]),
            ] {
                social.list_groups.push(Some(match name {
                    "Moderators" => moderator.to_string(),
                    "Members" => member_role.to_string(),
                    name => name.to_lowercase(),
                }));
                social
                    .list_groups
                    .extend(std::iter::repeat_n(None, ids.len()));
                rows.push(Some(MemberRow::Group {
                    name: name.into(),
                    count: ids.len() as u32,
                }));
                rows.extend(
                    ids.into_iter()
                        .map(|user_id| Some(MemberRow::Member { user_id })),
                );
            }
            social.list = Some(MemberList {
                id: "everyone".into(),
                member_count: store.users.len() as u32,
                online_count: store
                    .users
                    .values()
                    .filter(|u| u.status.is_online())
                    .count() as u32,
                rows,
            });
            store.social.guilds.insert(guild.id, social);
        }
        store.index_channels();
        store
    }
}

pub(super) const SEED_START: Timestamp = Timestamp {
    year: 2026,
    month: 10,
    day: 18,
    hour: 21,
    minute: 4,
};

fn seed_messages(
    count: usize,
    salt: usize,
    human_ids: &[u64],
    snippets: &[&str],
    next_message_id: &mut u64,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(count);
    let mut timestamp = SEED_START.plus_minutes((salt as u32 * 17) % 60);
    let mut burst = 0;
    let mut author = human_ids[salt % human_ids.len()];
    for index in 0..count {
        let (author_id, content, embed) = if index == 2 {
            (STATUS_BOT_ID, String::new(), Some(status_embed(salt)))
        } else {
            if burst == 0 {
                author = human_ids[(index * 3 + salt) % human_ids.len()];
                burst = 1 + (index + salt) % 3;
            }
            burst -= 1;
            (
                author,
                snippets[(index * 7 + salt) % snippets.len()].to_owned(),
                None,
            )
        };
        let gap = match messages.last() {
            Some(Message {
                author_id: previous,
                ..
            }) if *previous == author_id => 1,
            _ => 20 + ((index * 53 + salt * 11) % 240) as u32,
        };
        timestamp = timestamp.plus_minutes(gap);
        messages.push(Message {
            id: *next_message_id,
            send_order: None,
            author_id,
            webhook: None,
            timestamp,
            delivery: Delivery::Sent,
            content,
            embeds: embed.into_iter().collect(),
            components: Vec::new(),
            reactions: Vec::new(),
            attachments: Vec::new(),
            stickers: Vec::new(),
            kind: if index == 5 { 7 } else { 0 },
            edited: index == 3,
            reply: if index == 4 {
                messages.last().map(|message| {
                    ReplyState::Loaded(Reply {
                        message_id: message.id,
                        author_id: message.author_id,
                        webhook: message.webhook.clone(),
                        timestamp: message.timestamp,
                        content: if hides_embed_content(&message.content, &message.embeds) {
                            String::new()
                        } else {
                            message.content.clone()
                        },
                        attachments: false,
                        embeds: !message.embeds.is_empty() || !message.components.is_empty(),
                        sticker: None,
                    })
                })
            } else {
                None
            },
            forward: None,
            forwardable: false,
            mentions_me: false,
            uploads: Vec::new(),
            thread: None,
            reference_id: None,
        });
        *next_message_id += 1;
    }
    messages
}

fn status_embed(salt: usize) -> Embed {
    let items = [
        ("Arcarum Sephira Evolite", "252 days, 14 hours"),
        ("Sample Item One", "1 year, 238 days"),
        ("Sample Item Two", "16 days, 14 hours"),
        ("Sample Item Three", "92 days, 14 hours"),
        ("Sample Item Four", "1 year, 166 days"),
        ("Sample Item Five", "228 days, 14 hours"),
        ("Sample Item Six", "56 days, 14 hours"),
    ];
    mapping::map_embed(&model::Embed {
        color: Some([0x23A55A, 0x5865F2, 0xF0B232][salt % 3]),
        title: Some("Item Status".into()),
        description: Some(
            items
                .iter()
                .map(|(item, ago)| format!("{ago} ago since last {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        footer: Some(model::EmbedFooter {
            text: "Data Tracker || Demo Status Bot".into(),
            icon_url: None,
            proxy_icon_url: None,
        }),
        ..Default::default()
    })
}
