use super::*;
use crate::media::images;
use crate::store::{Message, Timestamp};
use crate::theme::WithoutTextSelection as _;

mod mentions;
mod scheduled;
mod unreads;

pub(crate) use scheduled::{ScheduleEdit, schedule_edit_dialog};

const INBOX_WIDTH: f32 = 560.;
pub(super) const CARD_MESSAGE_WIDTH: f32 = 518.;
const LOAD_AHEAD: f32 = 400.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InboxTab {
    #[default]
    Unreads,
    Mentions,
    Bookmarks,
    Reminders,
    Scheduled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InboxMenu {
    MentionFilter,
    Notifications { guild_id: u64, channel_id: u64 },
}

pub(crate) struct Inbox {
    pub(super) tab: InboxTab,
    pub(super) loading: bool,
    pub(super) failed: bool,
    pub(super) presets_for: Option<u64>,
    pub(super) menu: Option<InboxMenu>,
    pub(super) unreads: unreads::Unreads,
    pub(super) mentions: mentions::Mentions,
    pub(super) scheduled: scheduled::Scheduled,
    scroll: ScrollHandle,
}

impl DiscordApp {
    pub(in crate::app) fn toggle_inbox(&mut self, tab: Option<InboxTab>, cx: &mut Context<Self>) {
        if self.inbox.is_some() && tab.is_none() {
            self.close_inbox(cx);
            return;
        }
        let tab = tab.unwrap_or_else(|| {
            if self.store.overdue_reminders(messages::saved::now_ms()) > 0 {
                InboxTab::Reminders
            } else {
                self.last_inbox_tab
            }
        });
        self.inbox = Some(Inbox {
            tab,
            loading: false,
            failed: false,
            presets_for: None,
            menu: None,
            unreads: Default::default(),
            mentions: Default::default(),
            scheduled: Default::default(),
            scroll: ScrollHandle::new(),
        });
        self.select_inbox_tab(tab, cx);
    }

    pub(in crate::app) fn close_inbox(&mut self, cx: &mut Context<Self>) -> bool {
        if self.inbox.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn select_inbox_tab(&mut self, tab: InboxTab, cx: &mut Context<Self>) {
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        inbox.tab = tab;
        inbox.presets_for = None;
        inbox.menu = None;
        inbox.scroll.set_offset(point(px(0.), px(0.)));
        self.last_inbox_tab = tab;
        match tab {
            InboxTab::Unreads => self.load_unreads(cx),
            InboxTab::Mentions => self.load_mentions(false, cx),
            InboxTab::Bookmarks => self.load_bookmarks(false, cx),
            InboxTab::Reminders => {}
            InboxTab::Scheduled => self.load_scheduled(cx),
        }
        cx.notify();
    }

    pub(in crate::app) fn close_inbox_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(inbox) = self.inbox.as_mut().filter(|inbox| inbox.menu.is_some()) else {
            return false;
        };
        inbox.menu = None;
        cx.notify();
        true
    }

    fn set_inbox_menu(&mut self, menu: Option<InboxMenu>, cx: &mut Context<Self>) {
        if let Some(inbox) = &mut self.inbox {
            inbox.menu = menu;
            cx.notify();
        }
    }

    fn inbox_scrolled(&mut self, cx: &mut Context<Self>) {
        let Some(inbox) = &self.inbox else {
            return;
        };
        let remaining = inbox.scroll.max_offset().y + inbox.scroll.offset().y;
        if remaining > px(LOAD_AHEAD) {
            return;
        }
        match inbox.tab {
            InboxTab::Unreads => self.load_more_unreads(cx),
            InboxTab::Mentions => self.load_mentions(true, cx),
            InboxTab::Bookmarks => self.load_bookmarks(true, cx),
            _ => {}
        }
    }

    fn view_friend_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_inbox(cx);
        self.open_friend_requests(window, cx);
    }
}

pub(super) fn card_button(id: ElementId, icon: &str, tip: &'static str) -> Stateful<Div> {
    div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgba(theme::control_bg()))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .hover(|style| style.bg(rgba(theme::control_selected())))
        .child(theme::icon(icon, 16., theme::text()))
        .child(theme::hover_tooltip(
            ElementId::NamedChild(std::sync::Arc::new(id), "tip".into()),
            tip,
        ))
}

pub(super) fn channel_header(
    app: &DiscordApp,
    guild_id: Option<u64>,
    channel_id: u64,
    cx: &App,
) -> (AnyElement, String, String) {
    let guild = guild_id.and_then(|id| app.store.guild_by_id(id));
    if let Some(guild) = guild {
        let image = guild
            .icon_spec()
            .and_then(|spec| images::resolve(&spec, theme::image_px(32.), cx));
        let icon = match image {
            Some(path) => img(path)
                .size(rpx(32.))
                .flex_none()
                .rounded(rpx(8.))
                .into_any_element(),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded(rpx(8.))
                .bg(rgb(theme::bg()))
                .text_size(rpx(14.))
                .text_color(rgb(theme::text()))
                .child(guild.initials.clone())
                .into_any_element(),
        };
        let name = guild.channel(channel_id).map_or_else(
            || "unknown-channel".to_owned(),
            |channel| channel.name.clone(),
        );
        let category = guild
            .categories
            .iter()
            .find(|category| {
                category
                    .channels
                    .iter()
                    .any(|channel| channel.id == channel_id)
            })
            .map(|category| category.name.clone())
            .filter(|name| !name.is_empty());
        let subtitle = match category {
            Some(category) => format!("{} › {category}", guild.name),
            None => guild.name.clone(),
        };
        return (icon, format!("# {name}"), subtitle);
    }
    let private = app.store.private_channel(channel_id);
    let recipient = private
        .filter(|channel| !channel.group)
        .and_then(|channel| channel.recipient_ids.first().copied());
    let icon = match recipient {
        Some(user_id) => theme::user_avatar(app.store.user(user_id), user_id, 32., None, false, cx)
            .into_any_element(),
        None => div()
            .size(rpx(32.))
            .flex_none()
            .rounded_full()
            .bg(rgb(theme::bg()))
            .into_any_element(),
    };
    let name = app.store.private_channel_name(channel_id);
    let subtitle = if private.is_some_and(|channel| channel.group) {
        "Group DM".to_owned()
    } else {
        "Direct Message".to_owned()
    };
    (icon, name, subtitle)
}

pub(super) fn card_header(
    app: &DiscordApp,
    guild_id: Option<u64>,
    channel_id: u64,
    badge: Option<AnyElement>,
    buttons: Div,
    cx: &App,
) -> Div {
    let (icon, name, subtitle) = channel_header(app, guild_id, channel_id, cx);
    div()
        .relative()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .p(rpx(12.))
        .child(icon)
        .child(theme::icon("chevron-right", 12., theme::muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::text()))
                                .child(name),
                        )
                        .children(badge),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::text()))
                        .child(subtitle),
                ),
        )
        .child(buttons)
}

pub(super) fn card(status: Option<AnyElement>, header: Div, body: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .mb(rpx(16.))
        .rounded(rpx(12.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .overflow_hidden()
        .children(status)
        .child(header)
        .child(body)
        .into_any_element()
}

pub(super) fn card_status(icon: Svg, text: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .p(rpx(12.))
        .bg(rgb(theme::bg()))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(icon)
        .child(text)
        .into_any_element()
}

pub(super) fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .py(rpx(32.))
        .text_center()
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(text.into())
        .into_any_element()
}

pub(super) fn message_body(app: &DiscordApp, message: &Message, cx: &App) -> AnyElement {
    div()
        .my(rpx(12.))
        .child(chat::render_search_message(
            app,
            message,
            CARD_MESSAGE_WIDTH,
            cx,
        ))
        .into_any_element()
}

fn tab_button(app: &DiscordApp, tab: InboxTab, label: String, selected: bool) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(("inbox-tab", tab as usize))
        .flex()
        .flex_1()
        .justify_center()
        .min_w_0()
        .pt(rpx(8.))
        .pb(rpx(10.))
        .mb(rpx(-1.))
        .border_b_2()
        .border_color(if selected {
            rgb(theme::BLURPLE).into()
        } else {
            transparent_black()
        })
        .cursor_pointer()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(if selected {
            theme::BLURPLE
        } else {
            theme::muted()
        }))
        .hover(|style| style.text_color(rgb(theme::strong())))
        .without_text_selection()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.select_inbox_tab(tab, cx))
                .ok();
        })
        .child(div().truncate().child(label))
}

fn header_button(id: &'static str, icon: &str, tip: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgba(if active {
            theme::control_selected()
        } else {
            theme::control_bg()
        }))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .hover(|style| style.bg(rgba(theme::control_selected())))
        .child(theme::icon(icon, 16., theme::text()))
        .child(theme::hover_tooltip((id, 0usize), tip))
}

fn friend_requests_pill(app: &DiscordApp) -> Option<AnyElement> {
    let count = app.store.pending_count();
    if count == 0 {
        return None;
    }
    let entity = app.entity.clone();
    Some(
        div()
            .id("inbox-friend-requests")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(34.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .bg(rgba(theme::control_bg()))
            .border_1()
            .border_color(rgba(theme::border_soft()))
            .hover(|style| style.bg(rgba(theme::control_selected())))
            .on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.view_friend_requests(window, cx))
                    .ok();
            })
            .child(theme::icon("users", 16., theme::text()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(rpx(16.))
                    .h(rpx(16.))
                    .px(rpx(4.))
                    .rounded_full()
                    .bg(rgba(theme::control_selected()))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(count.to_string()),
            )
            .child(theme::hover_tooltip(
                "inbox-friend-requests-tip",
                "View Friend Requests",
            ))
            .into_any_element(),
    )
}

pub(in crate::app) fn inbox_popout(
    app: &DiscordApp,
    window: &Window,
    cx: &App,
) -> Option<AnyElement> {
    let inbox = app.inbox.as_ref()?;
    let now = messages::saved::now_ms();
    let reminders_label = match app.store.overdue_reminders(now) {
        0 => "Reminders".to_owned(),
        count => format!("Reminders ({count})"),
    };
    let action = match inbox.tab {
        InboxTab::Unreads => Some(unreads::mark_all_button(app)),
        InboxTab::Mentions => Some(mentions::filter_button(app, inbox, window, cx)),
        _ => None,
    };
    let content: Vec<AnyElement> = match inbox.tab {
        InboxTab::Unreads => unreads::list(app, inbox, window, cx),
        InboxTab::Mentions => mentions::list(app, inbox, cx),
        InboxTab::Bookmarks | InboxTab::Reminders => {
            messages::saved::saved_list(app, inbox, now, cx)
        }
        InboxTab::Scheduled => scheduled::list(app, inbox, cx),
    };
    let close = app.entity.clone();
    let scrolled = app.entity.clone();
    let tabs = [
        (InboxTab::Unreads, "Unreads".to_owned()),
        (InboxTab::Mentions, "Mentions".to_owned()),
        (InboxTab::Bookmarks, "Bookmarks".to_owned()),
        (InboxTab::Reminders, reminders_label),
        (InboxTab::Scheduled, "Scheduled".to_owned()),
    ];
    Some(
        div()
            .id("inbox-popout")
            .absolute()
            .top(rpx(titlebar::HEIGHT + 4.))
            .right(rpx(8.))
            .flex()
            .flex_col()
            .w(rpx(INBOX_WIDTH))
            .max_w(relative(0.9))
            .max_h(relative(0.8))
            .rounded(rpx(8.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::border_bright()))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| {
                        if app.schedule_edit.is_none() {
                            app.close_inbox(cx);
                        }
                    })
                    .ok();
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .pt(rpx(16.))
                    .px(rpx(16.))
                    .border_b_1()
                    .border_color(rgba(theme::border()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(8.))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(rpx(8.))
                                    .text_size(rpx(20.))
                                    .line_height(rpx(24.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::strong()))
                                    .child(theme::icon("inbox", 20., theme::strong()))
                                    .child("Inbox"),
                            )
                            .children(action)
                            .children(friend_requests_pill(app)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(rpx(8.))
                            .mt(rpx(12.))
                            .mx(rpx(-16.))
                            .children(
                                tabs.into_iter().map(|(tab, label)| {
                                    tab_button(app, tab, label, inbox.tab == tab)
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .id("inbox-list")
                    .flex()
                    .flex_col()
                    .flex_shrink_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&inbox.scroll)
                    .on_scroll_wheel(move |_, _, cx| {
                        scrolled.update(cx, |app, cx| app.inbox_scrolled(cx)).ok();
                    })
                    .pt(rpx(16.))
                    .px(rpx(16.))
                    .children(content),
            )
            .into_any_element(),
    )
}
