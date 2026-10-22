use gpui::{prelude::*, *};
use gpui_component::input::Input;

use crate::{
    app::{
        DiscordApp, member_list,
        message_body::{BodyContext, preview_pieces, render_body},
        rich_text::rich_text,
    },
    images,
    store::{Attachment, Embed, Message, ReplyState},
    theme::{self, rpx},
};

const GROUP_MINUTES: u32 = 7;
const GUTTER: f32 = 72.;
const BOTTOM_SPACER: f32 = 24.;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let channel = app.channel();
    let channel_name = channel.map_or("", |channel| &channel.name).to_owned();
    let guild_name = app.guild().map_or("", |guild| &guild.name).to_owned();
    let entity = cx.entity();
    let search = div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .text_size(rpx(14.))
        .text_color(rgb(theme::FAINT))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(format!("Search {guild_name}")),
        )
        .child(theme::icon("search", 16., theme::MUTED));
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(rpx(48.))
                .flex_none()
                .pl_4()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .child(theme::icon("hash", 20., theme::FAINT))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::STRONG))
                        .child(channel_name.clone()),
                )
                .child(div().flex_1())
                .child(theme::icon_button(
                    "threads",
                    "messages-square",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button(
                    "notifications",
                    "bell",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button("pins", "pin", 20., theme::MUTED))
                .child(
                    theme::icon_button(
                        "members",
                        "users",
                        20.,
                        if app.show_members {
                            theme::STRONG
                        } else {
                            theme::MUTED
                        },
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_members(cx))),
                )
                .map(|this| {
                    if app.show_members {
                        this.child(
                            div()
                                .flex_none()
                                .w(rpx(member_list::WIDTH))
                                .px(rpx(8.))
                                .child(search),
                        )
                    } else {
                        this.child(div().flex_none().w(rpx(244.)).ml_2().mr_2().child(search))
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(messages_column(app, entity))
                .when(app.show_members, |this| {
                    this.child(member_list::render(app, cx))
                }),
        )
}

fn messages_column(app: &DiscordApp, entity: Entity<DiscordApp>) -> impl IntoElement + use<> {
    let channel = app.channel();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .map(|this| {
            if channel.is_some_and(|channel| app.live && !channel.messages_loaded) {
                if app.is_loading(app.selected_channel_id) {
                    this.child(skeleton(app.selected_channel_id))
                } else {
                    this.child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(theme::MUTED))
                            .child("Couldn't load messages."),
                    )
                }
            } else {
                this.child(
                    list(app.messages.clone(), move |index, _, cx| {
                        render_row(entity.read(cx), index, cx)
                    })
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                )
            }
        })
        .child(
            div().px_2().pb_2().flex_none().child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_h(rpx(56.))
                    .pl(rpx(12.))
                    .pr(rpx(8.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::ELEVATED))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .child(theme::icon_button(
                        "attach",
                        "circle-plus",
                        24.,
                        theme::MUTED,
                    ))
                    .child(
                        div().flex_1().min_w_0().ml_4().child(
                            Input::new(&app.composer)
                                .appearance(false)
                                .w_full()
                                .h(rpx(56.))
                                .px_0()
                                .py(rpx(17.))
                                .text_size(rpx(16.))
                                .line_height(rpx(22.))
                                .text_color(rgb(theme::TEXT)),
                        ),
                    )
                    .child(theme::icon_button("gift", "gift", 24., theme::MUTED))
                    .child(theme::icon_button(
                        "translate",
                        "languages",
                        24.,
                        theme::MUTED,
                    ))
                    .child(theme::icon_button("gif", "image-play", 24., theme::MUTED))
                    .child(theme::icon_button("stickers", "sticker", 24., theme::MUTED))
                    .child(theme::icon_button("emoji", "smile", 24., theme::MUTED))
                    .child(theme::icon_button("apps", "layout-grid", 24., theme::MUTED)),
            ),
        )
}

struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self, range: std::ops::Range<u32>) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        range.start + ((self.0 >> 33) as u32) % (range.end - range.start)
    }
}

const SKELETON_GROUPS: usize = 14;

fn skeleton(seed: u64) -> impl IntoElement {
    let mut shuffle = Shuffle(seed ^ 0x9E37_79B9_7F4A_7C15);
    let bar = |width: f32, height: f32, color: u32| {
        div()
            .flex_none()
            .w(rpx(width))
            .h(rpx(height))
            .rounded(rpx(height / 2.))
            .bg(rgb(color))
    };
    let groups: Vec<_> = (0..SKELETON_GROUPS)
        .map(|_| {
            let name = shuffle.next(60..140) as f32;
            let lines: Vec<_> = (0..shuffle.next(1..4))
                .map(|_| {
                    let words: Vec<_> = (0..shuffle.next(2..9))
                        .map(|_| bar(shuffle.next(24..90) as f32, 14., theme::SELECTED))
                        .collect();
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(rpx(4.))
                        .min_h(rpx(22.))
                        .children(words)
                })
                .collect();
            let image = (shuffle.next(0..6) == 0).then(|| {
                div()
                    .mt_1()
                    .w(rpx(shuffle.next(200..320) as f32))
                    .h(rpx(shuffle.next(120..200) as f32))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::SELECTED))
            });
            div()
                .flex()
                .flex_none()
                .w_full()
                .mt_4()
                .py(rpx(2.))
                .pr_4()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(rpx(GUTTER))
                        .child(
                            div()
                                .mt(rpx(2.))
                                .size(rpx(40.))
                                .rounded_full()
                                .bg(rgb(theme::SELECTED)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().flex().items_center().h(rpx(22.)).child(bar(
                            name,
                            16.,
                            theme::PLACEHOLDER_STRONG,
                        )))
                        .children(lines)
                        .children(image),
                )
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .justify_end()
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_hidden()
        .pb(rpx(BOTTOM_SPACER))
        .children(groups)
}

fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let Some(messages) = app.channel().map(|channel| &channel.messages) else {
        return div().into_any_element();
    };
    let Some(message) = messages.get(index) else {
        return div().into_any_element();
    };
    let previous = index.checked_sub(1).and_then(|index| messages.get(index));
    let new_day = previous.is_some_and(|previous| !previous.timestamp.same_day(message.timestamp));
    let grouped = previous.is_some_and(|previous| {
        !new_day
            && previous.author_id == message.author_id
            && message.timestamp.minutes_since(previous.timestamp) < GROUP_MINUTES
            && message.reply.is_none()
            && !is_system(previous.kind)
            && !is_system(message.kind)
    });
    div()
        .w_full()
        .when(new_day, |this| this.child(date_divider(message)))
        .when(index + 1 == messages.len(), |this| {
            this.pb(rpx(BOTTOM_SPACER))
        })
        .child(render_message(app, message, grouped, cx))
        .into_any_element()
}

fn is_system(kind: u8) -> bool {
    !matches!(kind, 0 | 19 | 20 | 23)
}

fn date_divider(message: &Message) -> impl IntoElement {
    let line = || div().flex_1().h(rpx(1.)).bg(rgb(theme::BORDER));
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_4()
        .mt_6()
        .mb_2()
        .child(line())
        .child(
            div()
                .text_size(rpx(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::FAINT))
                .child(message.timestamp.date()),
        )
        .child(line())
}

fn body_context<'a>(app: &'a DiscordApp, message_id: u64) -> BodyContext<'a> {
    BodyContext {
        store: &app.store,
        guild_id: app.selected_guild_id,
        message_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
    }
}

fn author_name(app: &DiscordApp, user_id: u64) -> String {
    app.store
        .member_name(app.selected_guild_id, user_id)
        .unwrap_or("Unknown user")
        .to_owned()
}

fn author_color(app: &DiscordApp, user_id: u64) -> Hsla {
    app.store
        .member_color(app.selected_guild_id, user_id)
        .map_or(rgb(theme::STRONG).into(), |color| rgb(color).into())
}

const JOIN_MESSAGES: [&str; 13] = [
    "{} joined the party.",
    "{} is here.",
    "Welcome, {}. We hope you brought pizza.",
    "A wild {} appeared.",
    "{} just landed.",
    "{} just slid into the server.",
    "{} just showed up!",
    "Welcome {}. Say hi!",
    "{} hopped into the server.",
    "Everyone welcome {}!",
    "Glad you're here, {}.",
    "Good to see you, {}.",
    "Yay you made it, {}!",
];

const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

fn system_text(message: &Message) -> Option<(&'static str, u32, String)> {
    let template = match message.kind {
        7 => {
            let created = (message.id >> 22) + DISCORD_EPOCH_MS;
            return Some((
                "arrow-right",
                theme::GREEN,
                JOIN_MESSAGES[(created % JOIN_MESSAGES.len() as u64) as usize].into(),
            ));
        }
        6 => (
            "pin",
            theme::MUTED,
            "{} pinned a message to this channel.".into(),
        ),
        8 => ("gem", 0xFF73FA, "{} just boosted the server!".into()),
        9..=11 => (
            "gem",
            0xFF73FA,
            format!(
                "{{}} just boosted the server! This server has achieved Level {}!",
                message.kind - 8
            ),
        ),
        18 => (
            "messages-square",
            theme::MUTED,
            "{} started a thread.".into(),
        ),
        _ => return None,
    };
    Some(template)
}

fn render_system(app: &DiscordApp, message: &Message) -> Option<AnyElement> {
    let (icon, color, template) = system_text(message)?;
    let (before, after) = template.split_once("{}").unwrap_or((&template, ""));
    Some(
        div()
            .id(("message", message.id))
            .flex()
            .items_center()
            .w_full()
            .pr_4()
            .py(rpx(2.))
            .mt(rpx(4.))
            .hover(|style| style.bg(rgb(theme::HOVER)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w(rpx(GUTTER))
                    .child(theme::icon(icon, 16., color)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap(rpx(4.))
                    .min_w_0()
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .text_color(rgb(theme::MUTED))
                    .child(
                        div()
                            .flex()
                            .child(before.to_owned())
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(author_color(app, message.author_id))
                                    .child(author_name(app, message.author_id)),
                            )
                            .child(after.to_owned()),
                    )
                    .child(
                        div()
                            .text_size(rpx(12.))
                            .text_color(rgb(theme::FAINT))
                            .child(message.timestamp.full()),
                    ),
            )
            .into_any_element(),
    )
}

fn reply_bar(app: &DiscordApp, message: &Message, reply: &ReplyState, cx: &App) -> AnyElement {
    let spine = div()
        .absolute()
        .left(rpx(GUTTER / 2.))
        .top(rpx(11.))
        .w(rpx(GUTTER / 2. - 6.))
        .h(rpx(13.))
        .border_l_2()
        .border_t_2()
        .rounded_tl(rpx(6.))
        .border_color(rgb(0x4E5058));
    let content = div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(14.))
        .text_color(rgb(theme::MUTED));
    let content = match reply {
        ReplyState::Loaded(reply) => {
            let parsed = app.parsed(reply.message_id, &reply.content);
            let context = body_context(app, reply.message_id);
            let mut counter = 0;
            let pieces = preview_pieces(&parsed.blocks, &context, &mut counter, cx);
            let preview: AnyElement = if pieces.is_empty() {
                let label = if reply.attachments || reply.embeds {
                    "Click to see attachment"
                } else {
                    ""
                };
                div().italic().child(label).into_any_element()
            } else {
                rich_text(("reply", message.id), pieces)
                    .single_line()
                    .line_height(rpx(18.))
                    .emoji_size(rpx(16.))
                    .into_any_element()
            };
            content
                .child(theme::member_avatar(
                    &app.store,
                    app.selected_guild_id,
                    reply.author_id,
                    16.,
                    None,
                    cx,
                ))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(author_color(app, reply.author_id))
                        .child(author_name(app, reply.author_id)),
                )
                .child(div().min_w_0().overflow_hidden().child(preview))
        }
        ReplyState::Deleted => content.child(div().italic().child("Original message was deleted")),
        ReplyState::Unavailable => {
            content.child(div().italic().child("Message could not be loaded"))
        }
    };
    div()
        .relative()
        .flex()
        .items_center()
        .h(rpx(22.))
        .child(spine)
        .child(div().w(rpx(GUTTER)).flex_none())
        .child(content)
        .into_any_element()
}

fn render_message(app: &DiscordApp, message: &Message, grouped: bool, cx: &App) -> AnyElement {
    if is_system(message.kind)
        && let Some(element) = render_system(app, message)
    {
        return element;
    }
    let user = app.store.user(message.author_id);
    let parsed = app.parsed(message.id, &message.content);
    let context = body_context(app, message.id);
    div()
        .id(("message", message.id))
        .group("message")
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .pr_4()
        .py(rpx(2.))
        .when(!grouped, |this| this.mt_4())
        .map(|this| {
            if message.mentions_me {
                this.bg(rgba(0xF0B2321A))
                    .hover(|style| style.bg(rgba(0xF0B23224)))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(rpx(2.))
                            .bg(rgb(theme::YELLOW)),
                    )
            } else {
                this.hover(|style| style.bg(rgb(theme::HOVER)))
            }
        })
        .children(
            message
                .reply
                .as_ref()
                .map(|reply| reply_bar(app, message, reply, cx)),
        )
        .child(
            div()
                .flex()
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(rpx(GUTTER))
                        .map(|gutter| {
                            if grouped {
                                gutter.pt(rpx(4.)).child(
                                    div()
                                        .text_size(rpx(11.))
                                        .text_color(transparent_black())
                                        .group_hover("message", |style| {
                                            style.text_color(rgb(theme::FAINT))
                                        })
                                        .child(message.timestamp.time()),
                                )
                            } else {
                                gutter.pt(rpx(2.)).child(theme::member_avatar(
                                    &app.store,
                                    app.selected_guild_id,
                                    message.author_id,
                                    40.,
                                    None,
                                    cx,
                                ))
                            }
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .when(!grouped, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(6.))
                                    .child(
                                        div()
                                            .text_size(rpx(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(author_color(app, message.author_id))
                                            .cursor_pointer()
                                            .child(author_name(app, message.author_id)),
                                    )
                                    .when(user.is_some_and(|user| user.bot), |this| {
                                        this.child(theme::app_tag())
                                    })
                                    .children(
                                        user.and_then(|user| user.tag.as_ref())
                                            .map(|tag| theme::guild_tag(tag, cx)),
                                    )
                                    .child(
                                        div()
                                            .text_size(rpx(12.))
                                            .text_color(rgb(theme::FAINT))
                                            .child(message.timestamp.full()),
                                    ),
                            )
                        })
                        .when(!parsed.blocks.is_empty() || message.edited, |this| {
                            this.child(
                                div()
                                    .w_full()
                                    .text_size(rpx(16.))
                                    .line_height(rpx(22.))
                                    .child(render_body(
                                        &parsed.blocks,
                                        message.edited,
                                        parsed.jumbo,
                                        &context,
                                        cx,
                                    )),
                            )
                        })
                        .children(message.embed.as_ref().map(render_embed))
                        .children(
                            message
                                .attachments
                                .iter()
                                .map(|attachment| render_attachment(attachment, cx)),
                        ),
                ),
        )
        .into_any_element()
}

const MEDIA_MAX: (f32, f32) = (550., 350.);

fn render_attachment(attachment: &Attachment, cx: &App) -> AnyElement {
    let Some(spec) = attachment.image_spec() else {
        return file_card(attachment).into_any_element();
    };
    let (width, height) = (
        attachment.width.unwrap_or(1).max(1) as f32,
        attachment.height.unwrap_or(1).max(1) as f32,
    );
    let scale = (MEDIA_MAX.0 / width).min(MEDIA_MAX.1 / height).min(1.);
    let (width, height) = ((width * scale).round(), (height * scale).round());
    let image = images::resolve(&spec, theme::image_px(width.max(height)), cx);
    div()
        .mt_1()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::ELEVATED))
        .children(image.map(|path| img(path).size_full().rounded(rpx(8.))))
        .into_any_element()
}

fn file_card(attachment: &Attachment) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .child(theme::icon("file", 32., theme::MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(0x00A8FC))
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(file_size(attachment.size)),
                ),
        )
}

fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.2} KB", bytes as f64 / 1024.),
        _ => format!("{:.2} MB", bytes as f64 / 1_048_576.),
    }
}

fn render_embed(embed: &Embed) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .max_w(rpx(520.))
        .mt_1()
        .py_2()
        .pl_3()
        .pr_4()
        .rounded(rpx(4.))
        .border_l_4()
        .border_color(rgb(embed.color))
        .bg(rgb(theme::ELEVATED))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child(embed.title.clone()),
        )
        .children(embed.lines.iter().map(|line| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::TEXT))
                .child(line.clone())
        }))
        .child(
            div()
                .mt_1()
                .text_size(rpx(12.))
                .text_color(rgb(theme::MUTED))
                .child(embed.footer.clone()),
        )
}
