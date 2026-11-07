use gpui_tokio::Tokio;

use super::*;
use crate::app::menu::PopupButton as _;

const PAGE: usize = 25;

#[derive(Default)]
pub(in crate::app) struct Mentions {
    loading: bool,
    failed: bool,
    done: bool,
    messages: Vec<(u64, Message)>,
}

#[derive(Clone, Copy)]
enum Filter {
    Everyone,
    Roles,
}

impl DiscordApp {
    pub(super) fn load_mentions(&mut self, more: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let settings = crate::settings::get(cx);
        let (everyone, roles) = (
            !settings.mentions_everyone_off,
            !settings.mentions_roles_off,
        );
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        let mentions = &mut inbox.mentions;
        if mentions.loading || (more && mentions.done) {
            return;
        }
        if !more {
            *mentions = Mentions::default();
        }
        let before = mentions.messages.last().map(|(_, message)| message.id);
        mentions.loading = true;
        mentions.failed = false;
        let request = Tokio::spawn(cx, async move {
            session.mentions(before, roles, everyone).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = match result {
                    Ok(Ok(page)) => Some(page),
                    Ok(Err(error)) => {
                        log::warn!("Could not load mentions: {error}");
                        None
                    }
                    Err(error) => {
                        log::warn!("Mentions task failed: {error}");
                        None
                    }
                };
                let mapped = page.as_ref().map(|page| {
                    page.iter()
                        .filter_map(|message| {
                            let mapped = app.store.inbox_messages(std::slice::from_ref(message));
                            Some((message.channel_id, mapped.into_iter().next()?))
                        })
                        .collect::<Vec<_>>()
                });
                let Some(inbox) = &mut app.inbox else {
                    return;
                };
                let mentions = &mut inbox.mentions;
                mentions.loading = false;
                match (page, mapped) {
                    (Some(page), Some(mapped)) => {
                        mentions.done = page.len() < PAGE;
                        mentions.messages.extend(mapped);
                    }
                    _ => mentions.failed = true,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn remove_mention(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if let Some(inbox) = &mut self.inbox {
            inbox
                .mentions
                .messages
                .retain(|(_, message)| message.id != message_id);
        }
        cx.notify();
        let request = Tokio::spawn(cx, async move { session.delete_mention(message_id).await });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not remove mention: {error}"),
            Err(error) => log::warn!("Remove mention task failed: {error}"),
        })
        .detach();
    }

    fn toggle_mention_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| match filter {
            Filter::Everyone => settings.mentions_everyone_off ^= true,
            Filter::Roles => settings.mentions_roles_off ^= true,
        });
        self.load_mentions(false, cx);
    }
}

pub(super) fn filter_button(
    app: &DiscordApp,
    inbox: &Inbox,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let open = inbox.menu == Some(InboxMenu::MentionFilter);
    let entity = app.entity.clone();
    div()
        .relative()
        .child(
            header_button("inbox-mention-filter", "sliders-horizontal", "Filter", open)
                .on_popup_click(
                    crate::app::menu::shown(app, |app| {
                        app.inbox.as_ref().and_then(|inbox| inbox.menu)
                            == Some(InboxMenu::MentionFilter)
                    }),
                    move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.set_inbox_menu((!open).then_some(InboxMenu::MentionFilter), cx)
                            })
                            .ok();
                    },
                ),
        )
        .when(open, |this| {
            this.child(
                div()
                    .absolute()
                    .top(rpx(36.))
                    .right_0()
                    .size_0()
                    .child(filter_menu(app, window, cx)),
            )
        })
        .into_any_element()
}

fn filter_menu(app: &DiscordApp, window: &Window, cx: &App) -> AnyElement {
    let settings = crate::settings::get(cx);
    let row = |id: &'static str, title: &'static str, filter: Filter, on: bool| {
        let entity = app.entity.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(12.))
            .min_h(rpx(40.))
            .px(rpx(12.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::selected())))
            .without_text_selection()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| app.toggle_mention_filter(filter, cx))
                    .ok();
            })
            .child(
                div()
                    .flex_1()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(24.))
                    .rounded(rpx(6.))
                    .map(|this| {
                        if on {
                            this.bg(rgb(theme::BLURPLE))
                        } else {
                            this.border_1().border_color(rgba(theme::border_bright()))
                        }
                    })
                    .when(on, |this| {
                        this.child(theme::icon("check-bold", 16., 0xFFFFFF))
                    }),
            )
    };
    let close = app.entity.clone();
    let menu = div()
        .id("inbox-mention-filter-menu")
        .flex()
        .flex_col()
        .w(rpx(296.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border_bright()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| app.set_inbox_menu(None, cx))
                .ok();
        })
        .child(row(
            "inbox-filter-everyone",
            "Include @everyone mentions",
            Filter::Everyone,
            !settings.mentions_everyone_off,
        ))
        .child(row(
            "inbox-filter-roles",
            "Include @role mentions",
            Filter::Roles,
            !settings.mentions_roles_off,
        ));
    let margin = rpx(8.).to_pixels(window.rem_size());
    deferred(
        anchored()
            .anchor(Anchor::TopRight)
            .snap_to_window_with_margin(Edges::all(margin))
            .child(menu),
    )
    .with_priority(3)
    .into_any_element()
}

pub(super) fn list(app: &DiscordApp, inbox: &Inbox, cx: &App) -> Vec<AnyElement> {
    let mentions = &inbox.mentions;
    let mut items: Vec<AnyElement> = mentions
        .messages
        .iter()
        .map(|(channel_id, message)| {
            let (channel_id, message_id) = (*channel_id, message.id);
            let guild_id = app.store.channel_guild_id(channel_id);
            let jump = app.entity.clone();
            let read = app.entity.clone();
            let buttons = div()
                .flex()
                .flex_none()
                .gap(rpx(8.))
                .child(
                    card_button(
                        ElementId::NamedInteger("inbox-mention-jump".into(), message_id),
                        "message-jump",
                        "Jump to Message",
                    )
                    .on_click(move |_, window, cx| {
                        jump.update(cx, |app, cx| {
                            app.jump_to_saved(guild_id, channel_id, message_id, window, cx)
                        })
                        .ok();
                    }),
                )
                .child(
                    card_button(
                        ElementId::NamedInteger("inbox-mention-read".into(), message_id),
                        "check",
                        "Mark As Read",
                    )
                    .on_click(move |_, _, cx| {
                        read.update(cx, |app, cx| app.remove_mention(message_id, cx))
                            .ok();
                    }),
                );
            let header = card_header(app, guild_id, channel_id, None, buttons, cx)
                .border_b_1()
                .border_color(rgba(theme::outline()));
            card(None, header, message_body(app, message, cx))
        })
        .collect();
    if mentions.loading {
        items.push(note("Loading\u{2026}"));
    } else if mentions.failed {
        items.push(note("Could not load mentions."));
    } else if items.is_empty() {
        items.push(note("You don't have any recent mentions."));
    }
    items
}
