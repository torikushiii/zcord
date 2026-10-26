use super::composer::{emoji_token_overlay, flush_textarea};
use super::media::{render_attachment, render_sticker};
use super::rows::is_system;
use super::*;

pub(in crate::app) fn body_context<'a>(app: &'a DiscordApp, message_id: u64) -> BodyContext<'a> {
    BodyContext {
        store: &app.store,
        guild_id: app.selected_guild_id,
        message_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: Default::default(),
    }
}

fn forward_key(message_id: u64) -> u64 {
    embeds::part_key(message_id, embeds::FORWARD_SLOT)
}

pub(in crate::app) fn author_name(app: &DiscordApp, user_id: u64) -> String {
    app.store
        .member_name(app.selected_guild_id, user_id)
        .unwrap_or("Unknown user")
        .to_owned()
}

fn author_label(app: &DiscordApp, message: &Message) -> AnyElement {
    let name = author_name(app, message.author_id);
    let style = app
        .store
        .member_name_style(app.selected_guild_id, message.author_id)
        .filter(|_| {
            app.store
                .member_color(app.selected_guild_id, message.author_id)
                .is_none()
        });
    match style {
        Some(style) => theme::display_name(&name, Some(style), theme::STRONG),
        None => label(("author", message.id), &name),
    }
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
                                    .child(label(
                                        ("system-author", message.id),
                                        &author_name(app, message.author_id),
                                    )),
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
                .child(theme::icon("image", 18., theme::MUTED)),
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
                .child(theme::icon("sticker", 18., theme::MUTED)),
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
        .line_height(rpx(18.))
        .text_color(rgb(theme::MUTED));
    let content = match reply {
        ReplyState::Loaded(reply) => {
            let parsed = app.parsed(reply.message_id, &reply.content);
            let context = body_context(app, reply.message_id);
            let mut counter = 0;
            let pieces = preview_pieces(&parsed.blocks, &context, &mut counter, cx);
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
            let target = reply.message_id;
            let entity = app.entity.clone();
            content
                .id(("reply-preview", message.id))
                .without_text_selection()
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| app.jump_to_message(target, window, cx))
                        .ok();
                })
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(11.))
                        .text_color(rgb(theme::FAINT))
                        .child(reply.timestamp.time()),
                )
                .child(theme::member_avatar(
                    &app.store,
                    app.selected_guild_id,
                    reply.author_id,
                    16.,
                    None,
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
                        .text_color(author_color(app, reply.author_id))
                        .child(label(
                            ("reply-author", message.id),
                            &format!("@{}", author_name(app, reply.author_id)),
                        )),
                )
                .children(
                    app.store
                        .user(reply.author_id)
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

fn render_forward(app: &DiscordApp, message_id: u64, forward: &Forward, cx: &App) -> AnyElement {
    let key = forward_key(message_id);
    let parsed = app.parsed(key, &forward.content);
    let context = body_context(app, key);
    let show_body =
        !parsed.blocks.is_empty() && !embeds::hides_content(&forward.content, &forward.embeds);
    let channel = forward
        .channel_id
        .and_then(|channel_id| app.store.channel_name(channel_id));
    let footer = (channel.is_some() || forward.timestamp.is_some()).then(|| {
        let text = match (channel, forward.timestamp) {
            (Some(channel), Some(timestamp)) => format!("#{channel} • {}", timestamp.full()),
            (Some(channel), None) => format!("#{channel}"),
            (None, timestamp) => timestamp
                .map(|timestamp| timestamp.full())
                .unwrap_or_default(),
        };
        div()
            .mt(rpx(2.))
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::FAINT))
            .child(text)
    });
    div()
        .flex()
        .flex_col()
        .w_full()
        .mt(rpx(2.))
        .py(rpx(2.))
        .pl(rpx(12.))
        .rounded_l(rpx(4.))
        .border_l_4()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(theme::icon("forward", 16., theme::MUTED))
                .child(
                    div()
                        .italic()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .child("Forwarded"),
                ),
        )
        .when(show_body, |this| {
            this.child(
                div()
                    .w_full()
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .child(render_body(
                        &parsed.blocks,
                        forward.edited,
                        parsed.jumbo,
                        &context,
                        cx,
                    )),
            )
        })
        .children(
            forward
                .attachments
                .iter()
                .map(|attachment| render_attachment(app, attachment, cx)),
        )
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
                .map(|(index, embed)| embeds::render_embed(app, key, index, embed, cx)),
        )
        .children(embeds::render_components(app, key, &forward.components, cx))
        .children(footer)
        .into_any_element()
}

pub(super) fn render_message(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    preview: bool,
    cx: &App,
) -> AnyElement {
    if is_system(message.kind)
        && let Some(element) = render_system(app, message)
    {
        return element;
    }
    let user = app.store.user(message.author_id);
    let parsed = app.parsed(message.id, &message.content);
    let context = body_context(app, message.id);
    let rich = !message.embeds.is_empty() || !message.components.is_empty();
    let trailing_edited = message.edited && parsed.blocks.is_empty() && rich;
    let show_body = (!parsed.blocks.is_empty() || message.edited)
        && !trailing_edited
        && !embeds::hides_content(&message.content, &message.embeds);
    let editing = app
        .editing
        .as_ref()
        .filter(|editing| !preview && editing.message_id == message.id);
    let replying = !preview
        && app
            .reply
            .is_some_and(|reply| reply.message_id == message.id);
    let entity = app.entity.clone();
    let id = message.id;
    div()
        .id(("message", message.id))
        .group("message")
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .pr_4()
        .py(rpx(2.))
        .when(!grouped && !preview, |this| this.mt_4())
        .when(!preview, |this| {
            this.on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, false, cx))
                    .ok();
            })
        })
        .map(|this| {
            if replying {
                this.bg(rgba(0x5865F21A)).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(rpx(2.))
                        .bg(rgb(theme::BLURPLE)),
                )
            } else if message.mentions_me {
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
                this.when(!preview, |this| {
                    this.hover(|style| style.bg(rgb(theme::HOVER)))
                })
            }
        })
        .children(
            app.jump_highlight
                .filter(|(id, _)| *id == message.id && !preview)
                .map(|(_, nonce)| {
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgba(0x5865F233))
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
                                gutter.pt(rpx(2.)).child(
                                    div()
                                        .id("author-avatar")
                                        .when(!preview, |this| {
                                            this.cursor_pointer().without_text_selection().on_click(
                                                profile::opener(
                                                    app,
                                                    message.author_id,
                                                    Anchor::TopLeft,
                                                ),
                                            )
                                        })
                                        .child(theme::member_avatar(
                                            &app.store,
                                            app.selected_guild_id,
                                            message.author_id,
                                            40.,
                                            None,
                                            !preview && app.hovered_message == Some(message.id),
                                            cx,
                                        )),
                                )
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
                                    .h(rpx(22.))
                                    .line_height(rpx(22.))
                                    .child(
                                        div()
                                            .id("author-name")
                                            .text_size(rpx(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(author_color(app, message.author_id))
                                            .when(!preview, |this| {
                                                this.cursor_pointer()
                                                    .without_text_selection()
                                                    .hover(|style| style.underline())
                                                    .on_click(profile::opener(
                                                        app,
                                                        message.author_id,
                                                        Anchor::TopLeft,
                                                    ))
                                            })
                                            .child(author_label(app, message)),
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
                        .map(|this| match editing {
                            Some(editing) => this.child(message_editor(app, editing)),
                            None if show_body => this.child(
                                div()
                                    .w_full()
                                    .text_size(rpx(16.))
                                    .line_height(rpx(22.))
                                    .when(message.delivery == Delivery::Pending, |this| {
                                        this.opacity(0.5)
                                    })
                                    .when(message.delivery == Delivery::Failed, |this| {
                                        this.text_color(rgb(theme::RED))
                                    })
                                    .child(render_body(
                                        &parsed.blocks,
                                        message.edited,
                                        parsed.jumbo,
                                        &context,
                                        cx,
                                    )),
                            ),
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
                        .children(message.embeds.iter().enumerate().map(|(index, embed)| {
                            embeds::render_embed(app, message.id, index, embed, cx)
                        }))
                        .children(embeds::render_components(
                            app,
                            message.id,
                            &message.components,
                            cx,
                        ))
                        .children(super::invite_card::cards(app, message, cx))
                        .children(
                            message
                                .attachments
                                .iter()
                                .map(|attachment| render_attachment(app, attachment, cx)),
                        )
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
                                    .text_color(rgb(theme::FAINT))
                                    .child("(edited)"),
                            )
                        })
                        .children(embeds::render_reactions(
                            app,
                            message.id,
                            &message.reactions,
                            cx,
                        )),
                ),
        )
        .when(!preview && editing.is_none(), |this| {
            this.children(message_actions(app, message))
        })
        .into_any_element()
}

fn message_editor(app: &DiscordApp, editing: &Editing) -> impl IntoElement {
    let cancel = app.entity.clone();
    let save = app.entity.clone();
    let link = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(LINK))
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
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .relative()
                .child(
                    flush_textarea(Textarea::new(&editing.input))
                        .py(rpx(11.))
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .text_color(rgb(theme::TEXT)),
                )
                .child(emoji_token_overlay(&editing.input)),
        )
        .child(
            div()
                .flex()
                .py(rpx(4.))
                .text_size(rpx(12.))
                .text_color(rgb(theme::TEXT))
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
        .failed_sends
        .get(&message_id)
        .map_or("Message failed to send.", |failed| failed.error.as_str())
        .to_owned();
    let action = |id: (&'static str, u64), text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(LINK))
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

fn message_actions(app: &DiscordApp, message: &Message) -> Option<AnyElement> {
    let id = message.id;
    if !(app.hovered_message == Some(id) || app.hovered_actions == Some(id))
        || message.delivery != Delivery::Sent
        || is_system(message.kind) && !matches!(message.kind, 19)
    {
        return None;
    }
    let channel_id = app.selected_channel_id;
    let own = message.author_id == app.store.current_user_id;
    let can_reply = app.store.can_send(channel_id);
    let can_delete = own || app.store.can_manage_messages(channel_id);
    let button = |key: &'static str, icon: &str, color: u32, tip: &'static str| {
        theme::icon_button((key, id), icon, 20., color)
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
    };
    let hover = app.entity.clone();
    let edit = app.entity.clone();
    let reply = app.entity.clone();
    let delete = app.entity.clone();
    Some(
        div()
            .id(("message-actions", id))
            .absolute()
            .top(rpx(-16.))
            .right(rpx(14.))
            .flex()
            .p(rpx(2.))
            .rounded(rpx(8.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .block_mouse_except_scroll()
            .on_hover(move |hovered, _, cx| {
                hover
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, true, cx))
                    .ok();
            })
            .when(own && matches!(message.kind, 0 | 19), |this| {
                this.child(button("edit", "pencil", theme::MUTED, "Edit").on_click(
                    move |_, window, cx| {
                        edit.update(cx, |app, cx| app.start_edit(id, window, cx))
                            .ok();
                    },
                ))
            })
            .when(can_reply, |this| {
                this.child(button("reply", "reply", theme::MUTED, "Reply").on_click(
                    move |_, window, cx| {
                        reply
                            .update(cx, |app, cx| app.start_reply(id, window, cx))
                            .ok();
                    },
                ))
            })
            .when(can_delete, |this| {
                this.child(button("delete", "trash-2", theme::RED, "Delete").on_click(
                    move |event, window, cx| {
                        let confirmed = event.modifiers().shift;
                        delete
                            .update(cx, |app, cx| app.request_delete(id, confirmed, window, cx))
                            .ok();
                    },
                ))
            })
            .into_any_element(),
    )
}
