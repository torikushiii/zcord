use super::actions::{has_actions, message_actions};
use super::composer::{emoji_token_overlay, flush_textarea};
use super::media::{render_attachments, render_sticker};
use super::rows::is_system;
use super::*;
use crate::app::display_name::{self, Surface};
use crate::app::menu::PopupButton as _;
use crate::app::messages::message_body::render_body_scaled;
use crate::app::messages::rich_text::{Action, Piece};

pub(in crate::app) fn body_context<'a>(app: &'a DiscordApp, message_id: u64) -> BodyContext<'a> {
    BodyContext {
        store: &app.store,
        guild_id: app.selected_guild_id,
        message_id,
        revealed: &app.revealed_spoilers,
        reveal_all: app.spoilers_open(),
        entity: app.entity.clone(),
        colors: Default::default(),
    }
}

fn forward_key(message_id: u64) -> u64 {
    embeds::part_key(message_id, embeds::FORWARD_SLOT)
}

fn render_body_for_chat(
    blocks: &[crate::text::markdown::Block],
    edited: bool,
    jumbo: bool,
    scale: f32,
    context: &BodyContext,
    cx: &App,
) -> AnyElement {
    if scale == 1.0 {
        render_body(blocks, edited, jumbo, context, cx)
    } else {
        render_body_scaled(blocks, edited, jumbo, scale, context, cx)
    }
}

pub(in crate::app) fn author_name(app: &DiscordApp, user_id: u64) -> String {
    app.store
        .member_name(app.selected_guild_id, user_id)
        .unwrap_or("Unknown user")
        .to_owned()
}

pub(super) fn message_author_name(app: &DiscordApp, message: &Message) -> String {
    message.webhook.as_ref().map_or_else(
        || author_name(app, message.author_id),
        |author| author.name.clone(),
    )
}

fn author_label(app: &DiscordApp, message: &Message, cx: &App) -> AnyElement {
    let name = message_author_name(app, message);
    let style = app
        .store
        .member_name_style(app.selected_guild_id, message.author_id)
        .filter(|_| message.webhook.is_none());
    display_name::render(("author", message.id), &name, style, Surface::FontOnly, cx)
}

fn author_avatar(
    app: &DiscordApp,
    author_id: u64,
    webhook: Option<&crate::store::WebhookAuthor>,
    size: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    match webhook {
        Some(author) => {
            theme::webhook_avatar(author_id, author, size, animate, cx).into_any_element()
        }
        None => theme::member_avatar(
            &app.store,
            app.selected_guild_id,
            author_id,
            size,
            None,
            animate,
            cx,
        )
        .into_any_element(),
    }
}

fn author_color(app: &DiscordApp, user_id: u64) -> Hsla {
    app.store
        .member_name_color(app.selected_guild_id, user_id)
        .map_or(rgb(theme::strong()).into(), |color| rgb(color).into())
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

fn call_duration(secs: u64) -> String {
    let days = secs as f64 / 86_400.;
    let round = |value: f64| value.round() as u64;
    let (minutes, hours) = (round(secs as f64 / 60.), round(secs as f64 / 3_600.));
    let (months, years) = (
        round(days * 4_800. / 146_097.),
        round(days * 400. / 146_097.),
    );
    let days = round(days);
    let (count, one, unit) = if secs <= 44 {
        return "a few seconds".into();
    } else if minutes < 45 {
        (minutes, "a minute", "minutes")
    } else if hours < 22 {
        (hours, "an hour", "hours")
    } else if days < 26 {
        (days, "a day", "days")
    } else if months < 11 {
        (months, "a month", "months")
    } else {
        (years, "a year", "years")
    };
    if count <= 1 {
        one.into()
    } else {
        format!("{count} {unit}")
    }
}

pub(super) fn call_active(app: &DiscordApp, message: &Message) -> bool {
    message.call.as_ref().is_some_and(|record| {
        app.store
            .dm_call(record.channel_id)
            .is_some_and(|call| call.message_id == Some(message.id))
    })
}

fn call_text(app: &DiscordApp, message: &Message) -> (&'static str, u32, String) {
    let call = message.call.as_ref();
    let missed = !call_active(app, message)
        && call.is_some_and(|call| !call.participants.contains(&app.store.current_user_id));
    let lasted = call.and_then(|call| call.duration_secs).map(call_duration);
    match (missed, lasted) {
        (true, Some(lasted)) => (
            "phone-missed",
            theme::muted(),
            format!("You missed a call from {{}} that lasted {lasted}."),
        ),
        (true, None) => (
            "phone-missed",
            theme::muted(),
            "You missed a call from {}.".into(),
        ),
        (false, Some(lasted)) => (
            "phone",
            theme::GREEN,
            format!("{{}} started a call that lasted {lasted}."),
        ),
        (false, None) => ("phone", theme::GREEN, "{} started a call.".into()),
    }
}

fn system_text(app: &DiscordApp, message: &Message) -> Option<(&'static str, u32, String)> {
    let template = match message.kind {
        3 => call_text(app, message),
        7 => {
            let created = (message.id >> 22) + DISCORD_EPOCH_MS;
            return Some((
                "arrow-right",
                theme::GREEN,
                JOIN_MESSAGES[(created % JOIN_MESSAGES.len() as u64) as usize].into(),
            ));
        }
        6 => ("pin", theme::muted(), "{}".into()),
        8 => ("gem", 0xFF73FA, "{} just boosted the server!".into()),
        9..=11 => (
            "gem",
            0xFF73FA,
            format!(
                "{{}} just boosted the server! This server has achieved Level {}!",
                message.kind - 8
            ),
        ),
        18 => ("thread", theme::muted(), "{}".into()),
        _ => return None,
    };
    Some(template)
}

fn render_system(app: &DiscordApp, message: &Message, cx: &App) -> Option<AnyElement> {
    let (icon, color, template) = system_text(app, message)?;
    let (before, after) = template.split_once("{}").unwrap_or((&template, ""));
    Some(
        div()
            .id(("message", message.id))
            .flex()
            .flex_col()
            .w_full()
            .pr_4()
            .py(rpx(2.))
            .mt(rpx(4.))
            .hover(|style| style.bg(rgba(theme::hover())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w_full()
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
                            .text_color(rgb(theme::muted()))
                            .child(
                                div()
                                    .flex()
                                    .child(before.to_owned())
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(author_color(app, message.author_id))
                                            .child(display_name::render(
                                                ("system-author", message.id),
                                                &author_name(app, message.author_id),
                                                app.store.member_name_style(
                                                    app.selected_guild_id,
                                                    message.author_id,
                                                ),
                                                Surface::FontOnly,
                                                cx,
                                            )),
                                    )
                                    .child(after.to_owned()),
                            )
                            .children(super::thread_ui::system_tail(app, message))
                            .child(
                                div()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(theme::faint()))
                                    .child(message.timestamp.header()),
                            ),
                    ),
            )
            .children(super::thread_ui::thread_notice_chip(app, message, cx))
            .into_any_element(),
    )
}

pub(super) fn attachment_reply_preview() -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .italic()
                .debug_selector(|| "reply-attachment-label".into())
                .child("Click to see attachment"),
        )
        .child(
            div()
                .flex_none()
                .debug_selector(|| "reply-attachment-icon".into())
                .child(theme::icon("image", 18., theme::muted())),
        )
}

fn sticker_reply_preview(name: &str) -> Div {
    let label = if name.is_empty() {
        "Sticker".to_owned()
    } else {
        format!("Sticker: {name}")
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .italic()
                .debug_selector(|| "reply-sticker-label".into())
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .debug_selector(|| "reply-sticker-icon".into())
                .child(theme::icon("sticker", 18., theme::muted())),
        )
}

pub(super) fn passive_reply_pieces(mut pieces: Vec<Piece>) -> Vec<Piece> {
    for piece in &mut pieces {
        if matches!(&piece.style.action, Some(Action::OpenUrl(_))) {
            piece.style.color.get_or_insert(rgb(theme::link()).into());
        }
        piece.style.action = None;
    }
    pieces
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
        .border_color(rgb(theme::divider()));
    let content = div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()));
    let content = match reply {
        ReplyState::Loaded(reply) => {
            let parsed = app.parsed(reply.message_id, &reply.content);
            let context = body_context(app, reply.message_id);
            let mut counter = 0;
            let pieces =
                passive_reply_pieces(preview_pieces(&parsed.blocks, &context, &mut counter, cx));
            let preview: AnyElement = if pieces.is_empty() {
                if let Some(name) = &reply.sticker {
                    sticker_reply_preview(name).into_any_element()
                } else if reply.attachments || reply.embeds {
                    attachment_reply_preview().into_any_element()
                } else {
                    div().into_any_element()
                }
            } else {
                rich_text(("reply", message.id), pieces)
                    .single_line()
                    .line_height(rpx(18.))
                    .emoji_size(rpx(16.))
                    .into_any_element()
            };
            let source = message.id;
            let target = reply.message_id;
            let entity = app.entity.clone();
            content
                .id(("reply-preview", message.id))
                .without_text_selection()
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| {
                            if !app.jump_in_thread_pane(source, target) {
                                app.jump_to_quoted(source, target, window, cx);
                            }
                        })
                        .ok();
                })
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(11.))
                        .text_color(rgb(theme::faint()))
                        .child(reply.timestamp.time()),
                )
                .child(author_avatar(
                    app,
                    reply.author_id,
                    reply.webhook.as_ref(),
                    16.,
                    false,
                    cx,
                ))
                .child(
                    div()
                        .flex_none()
                        .max_w(relative(0.5))
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if reply.webhook.is_some() {
                            rgb(theme::strong()).into()
                        } else {
                            author_color(app, reply.author_id)
                        })
                        .child(display_name::render(
                            ("reply-author", message.id),
                            &format!(
                                "@{}",
                                reply.webhook.as_ref().map_or_else(
                                    || author_name(app, reply.author_id),
                                    |author| author.name.clone(),
                                )
                            ),
                            app.store
                                .member_name_style(app.selected_guild_id, reply.author_id)
                                .filter(|_| reply.webhook.is_none()),
                            Surface::FontOnly,
                            cx,
                        )),
                )
                .children(
                    app.store
                        .user(reply.author_id)
                        .filter(|_| reply.webhook.is_none())
                        .and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().child(preview))
                .into_any_element()
        }
        ReplyState::Deleted => content
            .child(div().italic().child("Original message was deleted"))
            .into_any_element(),
        ReplyState::Unavailable => content
            .child(div().italic().child("Message could not be loaded"))
            .into_any_element(),
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

fn forward_footer(
    app: &DiscordApp,
    message_id: u64,
    forward: &Forward,
    cx: &App,
) -> Option<Stateful<Div>> {
    let guild = forward
        .guild_id
        .and_then(|guild_id| app.store.guild_by_id(guild_id));
    let target = forward
        .channel_id
        .zip(forward.message_id)
        .filter(|(channel_id, _)| match forward.guild_id {
            Some(_) => guild.is_some(),
            None => app.store.private_channel(*channel_id).is_some(),
        });
    let stamp = forward.timestamp.map(|timestamp| timestamp.forwarded());
    if guild.is_none() && stamp.is_none() {
        return None;
    }
    let guild_id = forward.guild_id;
    let entity = app.entity.clone();
    Some(
        div()
            .id(("forward-source", message_id))
            .flex()
            .flex_none()
            .self_start()
            .max_w_full()
            .items_center()
            .gap(rpx(4.))
            .mt(rpx(4.))
            .py(rpx(1.))
            .rounded(rpx(4.))
            .min_w_0()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::faint()))
            .children(guild.map(|guild| crate::app::titlebar::guild_icon(guild, 4., cx)))
            .children(guild.map(|guild| div().min_w_0().truncate().child(guild.name.clone())))
            .when(guild.is_some() && stamp.is_some(), |this| {
                this.child(div().px(rpx(3.5)).child("•"))
            })
            .children(stamp.map(|stamp| div().flex_none().child(stamp)))
            .when_some(target, |this, (channel_id, source_id)| {
                this.cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgba(theme::control_bg()))
                            .text_color(rgb(theme::strong()))
                    })
                    .child(theme::icon("chevron-right", 12., theme::faint()))
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.open_message(guild_id, channel_id, source_id, window, cx)
                            })
                            .ok();
                    })
            }),
    )
}

fn render_forward(app: &DiscordApp, message_id: u64, forward: &Forward, cx: &App) -> AnyElement {
    let key = forward_key(message_id);
    let font_size = crate::settings::chat_font_size();
    let line_height = font_size * 22.0 / 16.0;
    let text_scale = font_size / 16.0;
    let parsed = app.parsed(key, &forward.content);
    let context = body_context(app, key);
    let show_body =
        !parsed.blocks.is_empty() && !embeds::hides_content(&forward.content, &forward.embeds);
    let footer = forward_footer(app, message_id, forward, cx);
    let content = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .mt(rpx(2.))
                .mb(rpx(4.))
                .child(theme::icon("forward", 16., theme::faint()))
                .child(
                    div()
                        .italic()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::faint()))
                        .child("Forwarded"),
                ),
        )
        .when(show_body, |this| {
            this.child(
                div()
                    .w_full()
                    .text_size(rpx(font_size))
                    .line_height(rpx(line_height))
                    .child(render_body_for_chat(
                        &parsed.blocks,
                        forward.edited,
                        parsed.jumbo,
                        text_scale,
                        &context,
                        cx,
                    )),
            )
        })
        .children(render_attachments(app, &forward.attachments, cx))
        .children(
            forward
                .stickers
                .iter()
                .map(|sticker| render_sticker(app, message_id, sticker, cx)),
        )
        .children(
            forward
                .embeds
                .iter()
                .enumerate()
                .filter(|(_, embed)| embeds::visible(embed, &forward.content))
                .map(|(index, embed)| embeds::render_embed(app, key, index, embed, cx)),
        )
        .children(embeds::render_components(app, key, &forward.components, cx))
        .children(footer);
    div()
        .flex()
        .w_full()
        .py(rpx(2.))
        .child(
            div()
                .flex_none()
                .w(rpx(4.))
                .mr(rpx(12.))
                .rounded(rpx(2.))
                .bg(rgba(theme::outline())),
        )
        .child(content)
        .into_any_element()
}

pub(in crate::app) fn render_search_message(
    app: &DiscordApp,
    message: &Message,
    width: f32,
    cx: &App,
) -> AnyElement {
    with_media_limit(width - GUTTER - MESSAGE_RIGHT_PADDING, || {
        render_message(app, message, false, true, cx)
    })
}

pub(super) fn render_message(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    preview: bool,
    cx: &App,
) -> AnyElement {
    render_message_with(app, message, grouped, preview, false, cx)
}

pub(super) fn render_pane_message(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    cx: &App,
) -> AnyElement {
    render_message_with(app, message, grouped, true, true, cx)
}

fn render_message_with(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    preview: bool,
    pane: bool,
    cx: &App,
) -> AnyElement {
    if is_system(message.kind)
        && let Some(element) = render_system(app, message, cx)
    {
        return element;
    }
    let user = app
        .store
        .user(message.author_id)
        .filter(|_| message.webhook.is_none());
    let font_size = crate::settings::chat_font_size();
    let line_height = font_size * 22.0 / 16.0;
    let text_scale = font_size / 16.0;
    let parsed = app.parsed(message.id, &message.content);
    let context = body_context(app, message.id);
    let rich = message
        .embeds
        .iter()
        .any(|embed| embeds::visible(embed, &message.content))
        || !message.components.is_empty();
    let trailing_edited = message.edited && parsed.blocks.is_empty() && rich;
    let show_body = (!parsed.blocks.is_empty() || message.edited)
        && !trailing_edited
        && !embeds::hides_content(&message.content, &message.embeds);
    let editing = app
        .compose
        .editing
        .as_ref()
        .filter(|editing| !preview && editing.message_id == message.id);
    let replying = !preview
        && app
            .compose
            .reply
            .as_ref()
            .is_some_and(|reply| reply.message_id == message.id);
    let entity = app.entity.clone();
    let menu_entity = app.entity.clone();
    let id = message.id;
    let display = crate::settings::display();
    let compact = display.message_display_compact && (!preview || pane);
    let compact_avatar = compact && display.display_compact_avatars;
    let group_gap = (!grouped && !preview)
        .then(|| crate::settings::message_group_spacing(compact))
        .filter(|gap| *gap > 0);
    let grouped = grouped && !compact;
    let author = |size: f32| {
        div()
            .id("author-avatar")
            .flex_none()
            .when(!preview && message.webhook.is_none(), |this| {
                this.cursor_pointer()
                    .without_text_selection()
                    .on_popup_click(
                        profile::shown(app, message.author_id),
                        profile::opener(app, message.author_id, Anchor::TopLeft),
                    )
            })
            .child(author_avatar(
                app,
                message.author_id,
                message.webhook.as_ref(),
                size,
                !preview && app.hover.message == Some(message.id),
                cx,
            ))
    };
    let body = || {
        div()
            .w_full()
            .text_size(rpx(font_size))
            .line_height(rpx(line_height))
            .when(message.delivery == Delivery::Pending, |this| {
                this.opacity(0.5)
            })
            .when(message.delivery == Delivery::Failed, |this| {
                this.text_color(rgb(theme::RED))
            })
            .child(render_body_for_chat(
                &parsed.blocks,
                message.edited,
                parsed.jumbo,
                text_scale,
                &context,
                cx,
            ))
    };
    div()
        .id(("message", message.id))
        .group("message")
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .pr_4()
        .py(rpx(2.))
        .when_some(group_gap, |this, gap| this.mt(rpx(gap as f32)))
        .when(!preview, |this| {
            this.on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, false, cx))
                    .ok();
            })
        })
        .when(!preview && has_actions(message), |this| {
            this.on_mouse_down(MouseButton::Right, move |event, _, cx| {
                let position = event.position;
                menu_entity
                    .update(cx, |app, cx| app.open_message_menu_at(id, position, cx))
                    .ok();
            })
        })
        .map(|this| {
            if replying {
                this.bg(rgba(theme::reply_bg())).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(rpx(2.))
                        .bg(rgb(theme::BLURPLE)),
                )
            } else if message.mentions_me {
                this.bg(rgba(theme::mention_row_bg()))
                    .hover(|style| style.bg(rgba(theme::mention_row_hover())))
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
                this.when(!preview, |this| {
                    this.hover(|style| style.bg(rgba(theme::hover())))
                })
            }
        })
        .children(
            app.jumps
                .highlight
                .filter(|(id, _)| *id == message.id && !preview)
                .map(|(_, nonce)| {
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgba(theme::jump_highlight()))
                        .with_animation(
                            ("jump-highlight", nonce),
                            Animation::new(JUMP_HIGHLIGHT),
                            |this, delta| this.opacity(1. - ((delta - 0.25) / 0.75).clamp(0., 1.)),
                        )
                }),
        )
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
                            if compact {
                                gutter.pt(rpx(3.)).child(
                                    div()
                                        .text_size(rpx(11.))
                                        .line_height(rpx(line_height - 3.))
                                        .text_color(rgb(theme::faint()))
                                        .child(message.timestamp.time()),
                                )
                            } else if grouped {
                                gutter.pt(rpx(4.)).child(
                                    div()
                                        .text_size(rpx(11.))
                                        .text_color(transparent_black())
                                        .group_hover("message", |style| {
                                            style.text_color(rgb(theme::faint()))
                                        })
                                        .child(message.timestamp.time()),
                                )
                            } else {
                                gutter.pt(rpx(2.)).child(author(40.))
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
                                    .gap(rpx(4.))
                                    .when(!compact, |this| this.h(rpx(22.)))
                                    .line_height(rpx(22.))
                                    .when(compact, |this| this.items_start().flex_wrap())
                                    .when(compact_avatar, |this| {
                                        this.child(div().pt(rpx(3.)).child(author(16.)))
                                    })
                                    .child(
                                        div()
                                            .id("author-name")
                                            .text_size(rpx(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(if message.webhook.is_some() {
                                                rgb(theme::strong()).into()
                                            } else {
                                                author_color(app, message.author_id)
                                            })
                                            .when(!preview && message.webhook.is_none(), |this| {
                                                this.cursor_pointer()
                                                    .without_text_selection()
                                                    .hover(|style| style.underline())
                                                    .on_popup_click(
                                                        profile::shown(app, message.author_id),
                                                        profile::opener(
                                                            app,
                                                            message.author_id,
                                                            Anchor::TopLeft,
                                                        ),
                                                    )
                                            })
                                            .child(author_label(app, message, cx)),
                                    )
                                    .children(
                                        message
                                            .webhook
                                            .is_none()
                                            .then(|| {
                                                app.store.member_role_dot(
                                                    app.selected_guild_id,
                                                    message.author_id,
                                                )
                                            })
                                            .flatten()
                                            .map(|color| {
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .h(rpx(22.))
                                                    .child(theme::role_dot(color))
                                            }),
                                    )
                                    .when(
                                        message.webhook.is_some()
                                            || user.is_some_and(|user| user.bot),
                                        |this| {
                                            this.child(theme::app_tag(match &message.webhook {
                                                Some(webhook) => webhook.verified_bot,
                                                None => user.is_some_and(|user| user.verified_bot),
                                            }))
                                        },
                                    )
                                    .children(
                                        user.and_then(|user| user.tag.as_ref())
                                            .map(|tag| theme::guild_tag(tag, cx)),
                                    )
                                    .when(!compact, |this| {
                                        this.child(
                                            div()
                                                .ml(rpx(4.))
                                                .text_size(rpx(12.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(theme::faint()))
                                                .child(message.timestamp.header()),
                                        )
                                    })
                                    .when(compact && editing.is_none() && show_body, |this| {
                                        this.child(div().flex_1().min_w(rpx(160.)).child(body()))
                                    }),
                            )
                        })
                        .map(|this| match editing {
                            Some(editing) => this.child(message_editor(app, editing, cx)),
                            None if show_body && !compact => this.child(body()),
                            None => this,
                        })
                        .children(
                            message
                                .forward
                                .as_ref()
                                .map(|forward| render_forward(app, message.id, forward, cx)),
                        )
                        .when(message.delivery == Delivery::Failed && !preview, |this| {
                            this.child(send_failure(app, message.id))
                        })
                        .children(
                            message
                                .embeds
                                .iter()
                                .enumerate()
                                .filter(|(_, embed)| embeds::visible(embed, &message.content))
                                .map(|(index, embed)| {
                                    embeds::render_embed(app, message.id, index, embed, cx)
                                }),
                        )
                        .children(embeds::render_components(
                            app,
                            message.id,
                            &message.components,
                            cx,
                        ))
                        .children(super::invite_card::cards(app, message, cx))
                        .children(render_attachments(app, &message.attachments, cx))
                        .children(super::media::upload_card(app, message.id, &message.uploads))
                        .children(
                            message
                                .stickers
                                .iter()
                                .map(|sticker| render_sticker(app, message.id, sticker, cx)),
                        )
                        .when(trailing_edited, |this| {
                            this.child(
                                div()
                                    .mt(rpx(2.))
                                    .text_size(rpx(10.))
                                    .text_color(rgb(theme::faint()))
                                    .child("(edited)"),
                            )
                        })
                        .children(embeds::render_reactions(
                            app,
                            message.id,
                            &message.reactions,
                            cx,
                        ))
                        .when(!preview, |this| {
                            this.children(super::thread_ui::thread_chip(app, message, cx))
                        }),
                ),
        )
        .when(!preview && editing.is_none(), |this| {
            this.children(message_actions(app, message, cx))
        })
        .into_any_element()
}

fn message_editor(app: &DiscordApp, editing: &Editing, cx: &App) -> impl IntoElement {
    let cancel = app.entity.clone();
    let save = app.entity.clone();
    let link = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(theme::link()))
            .hover(|style| style.underline())
            .child(text)
    };
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(
            div()
                .mt_1()
                .px(rpx(16.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .relative()
                .child(
                    flush_textarea(Textarea::new(&editing.input))
                        .py(rpx(11.))
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .text_color(rgb(theme::text())),
                )
                .child(emoji_token_overlay(&editing.input))
                .children(crate::app::messages::mentions::suggestions(app, cx))
                .children(crate::app::expressions::emoji_picker::suggestions(app, cx)),
        )
        .child(
            div()
                .flex()
                .py(rpx(4.))
                .text_size(rpx(12.))
                .text_color(rgb(theme::text()))
                .child("escape to\u{a0}")
                .child(
                    link("edit-cancel", "cancel").on_click(move |_, window, cx| {
                        cancel
                            .update(cx, |app, cx| app.cancel_edit(window, cx))
                            .ok();
                    }),
                )
                .child("\u{a0}• enter to\u{a0}")
                .child(link("edit-save", "save").on_click(move |_, window, cx| {
                    save.update(cx, |app, cx| app.save_edit(window, cx)).ok();
                })),
        )
}

fn send_failure(app: &DiscordApp, message_id: u64) -> impl IntoElement {
    let retry = app.entity.clone();
    let dismiss = app.entity.clone();
    let error = app
        .compose
        .failed_sends
        .get(&message_id)
        .map_or("Message failed to send.", |failed| failed.error.as_str())
        .to_owned();
    let action = |id: (&'static str, u64), text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(theme::link()))
            .hover(|style| style.underline())
            .child(text)
    };
    div()
        .flex()
        .flex_wrap()
        .gap(rpx(6.))
        .text_size(rpx(12.))
        .text_color(rgb(theme::RED))
        .child(error)
        .child(
            action(("retry-send", message_id), "Retry").on_click(move |_, window, cx| {
                retry
                    .update(cx, |app, cx| app.retry_message(message_id, window, cx))
                    .ok();
            }),
        )
        .child(
            action(("dismiss-send", message_id), "Dismiss").on_click(move |_, window, cx| {
                dismiss
                    .update(cx, |app, cx| app.discard_message(message_id, window, cx))
                    .ok();
            }),
        )
}
