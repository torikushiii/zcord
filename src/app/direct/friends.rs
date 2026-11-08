use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;
use std::collections::HashSet;

use crate::{
    app::DiscordApp,
    discord::{Error, valid_friend_username},
    store::RelationshipKind,
    theme::{self, WithoutTextSelection, rpx},
};

/// Discord can answer a friend request with a captcha we can't show yet, so sending one by
/// username stays in the official client for now. Flip this once captchas are handled.
const SEND_REQUESTS: bool = false;
const SEND_FROM_DISCORD: &str = "Send friend requests from Discord for now";

#[derive(Clone, PartialEq, Eq)]
enum AddNote {
    Sent(String),
    Failed(&'static str),
}

#[derive(Default)]
pub(crate) struct Friends {
    busy: HashSet<u64>,
    error: Option<(u64, &'static str)>,
    confirm_remove: Option<u64>,
    username: Option<Entity<InputState>>,
    _username_events: Option<Subscription>,
    note: Option<AddNote>,
    sending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FriendAction {
    Add,
    Accept,
    Sent,
    Friends,
}

pub(crate) fn failure_text(error: &Error, rejected: &'static str) -> &'static str {
    match error {
        Error::Captcha => "Discord asked for verification. Try this in the official client.",
        Error::RateLimited { .. } => "You're doing that too fast. Try again in a moment.",
        Error::Rejected { .. } => rejected,
        _ => "Something went wrong. Try again.",
    }
}

impl DiscordApp {
    fn write_relationship(&mut self, user_id: u64, remove: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !self.home.friends.busy.insert(user_id) {
            return;
        }
        self.home.friends.error = None;
        cx.notify();
        let guild_id = self.request_guild();
        let channel_id = (self.selected_channel_id != 0).then_some(self.selected_channel_id);
        let request = Tokio::spawn(cx, async move {
            if remove {
                session.delete_relationship(user_id).await
            } else {
                session
                    .put_relationship(user_id, guild_id, channel_id)
                    .await
            }
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.home.friends.busy.remove(&user_id);
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        log::warn!("Could not change relationship: {error}");
                        let text = failure_text(
                            &error,
                            "Discord refused this. Their privacy settings may not allow it.",
                        );
                        app.home.friends.error = Some((user_id, text));
                    }
                    Err(error) => log::warn!("Relationship task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn add_friend(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.write_relationship(user_id, false, cx);
    }

    pub(crate) fn drop_friend_request(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.write_relationship(user_id, true, cx);
    }

    pub(crate) fn ask_remove_friend(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.home.friends.confirm_remove = Some(user_id);
        self.set_profile_menu(false, cx);
        cx.notify();
    }

    pub(crate) fn cancel_remove_friend(&mut self, cx: &mut Context<Self>) -> bool {
        if self.home.friends.confirm_remove.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn confirm_remove_friend(&mut self, cx: &mut Context<Self>) {
        if let Some(user_id) = self.home.friends.confirm_remove.take() {
            self.write_relationship(user_id, true, cx);
        }
    }

    pub(crate) fn friend_action(&self, user_id: u64) -> Option<FriendAction> {
        if !self.live
            || user_id == self.store.current_user_id
            || self.store.user(user_id).is_some_and(|user| user.bot)
        {
            return None;
        }
        match self.store.relationship(user_id).map(|found| found.kind) {
            None => Some(FriendAction::Add),
            Some(RelationshipKind::Incoming) => Some(FriendAction::Accept),
            Some(RelationshipKind::Outgoing) => Some(FriendAction::Sent),
            Some(RelationshipKind::Friend) => Some(FriendAction::Friends),
            Some(RelationshipKind::Blocked) => None,
        }
    }

    pub(crate) fn note_dm_failure(&mut self, user_id: u64, error: &Error, cx: &mut Context<Self>) {
        let text = match error {
            Error::Rejected { code: 50007, .. } => "This user isn't accepting direct messages.",
            _ => failure_text(error, "Discord refused to open this DM."),
        };
        self.home.friends.error = Some((user_id, text));
        cx.notify();
    }

    pub(crate) fn friend_busy(&self, user_id: u64) -> bool {
        self.home.friends.busy.contains(&user_id)
    }

    pub(crate) fn friend_error(&self, user_id: u64) -> Option<&'static str> {
        self.home
            .friends
            .error
            .filter(|(id, _)| *id == user_id)
            .map(|(_, text)| text)
    }

    pub(crate) fn pending_error(&self) -> Option<&'static str> {
        self.home.friends.error.map(|(_, text)| text)
    }

    pub(crate) fn ensure_friend_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home.friends.username.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Enter a username"));
        self.home.friends._username_events =
            Some(cx.subscribe(&input, |app, _, event, cx| match event {
                InputEvent::Change => {
                    app.home.friends.note = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => app.send_friend_request(cx),
                _ => {}
            }));
        self.home.friends.username = Some(input);
    }

    fn known_username(&self, username: &str) -> bool {
        self.store
            .friends()
            .into_iter()
            .chain(self.store.pending_requests())
            .filter_map(|id| self.store.user(id))
            .any(|user| user.username.eq_ignore_ascii_case(username))
    }

    fn send_friend_request(&mut self, cx: &mut Context<Self>) {
        if !SEND_REQUESTS {
            self.home.friends.note = Some(AddNote::Failed(SEND_FROM_DISCORD));
            cx.notify();
            return;
        }
        let Some(input) = &self.home.friends.username else {
            return;
        };
        let value = input.read(cx).value();
        let username = value.trim().trim_start_matches('@').to_lowercase();
        if username.is_empty() || self.home.friends.sending {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let refused = if !valid_friend_username(&username) {
            Some("Usernames are 2-32 letters, numbers, underscores or periods.")
        } else if self.known_username(&username) {
            Some("You're already friends, or a request is pending.")
        } else {
            None
        };
        if let Some(text) = refused {
            self.home.friends.note = Some(AddNote::Failed(text));
            cx.notify();
            return;
        }
        self.home.friends.sending = true;
        self.home.friends.note = None;
        cx.notify();
        let name = username.clone();
        let request = Tokio::spawn(cx, async move { session.send_friend_request(&name).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.home.friends.sending = false;
                app.home.friends.note = Some(match result {
                    Ok(Ok(())) => AddNote::Sent(username),
                    Ok(Err(error)) => {
                        log::warn!("Could not send friend request: {error}");
                        AddNote::Failed(failure_text(
                            &error,
                            "Hm, didn't work. Double check that the username is correct.",
                        ))
                    }
                    Err(error) => {
                        log::warn!("Friend request task failed: {error}");
                        AddNote::Failed("Something went wrong. Try again.")
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn add_friend_page(app: &DiscordApp) -> impl IntoElement + use<> {
    let friends = &app.home.friends;
    let can_send = SEND_REQUESTS && !friends.sending;
    let send = app.entity.clone();
    let (border, note) = match &friends.note {
        Some(AddNote::Sent(name)) => (
            theme::GREEN,
            Some((
                theme::GREEN,
                format!("Success! Your friend request to {name} was sent."),
            )),
        ),
        Some(AddNote::Failed(text)) => (theme::RED, Some((theme::RED, (*text).to_owned()))),
        None => (theme::BLURPLE, None),
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .px_6()
        .pt(rpx(20.))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Add Friend"),
        )
        .child(
            div()
                .mt(rpx(8.))
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child("You can add friends with their Discord username."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .mt(rpx(16.))
                .h(rpx(52.))
                .pl(rpx(12.))
                .pr(rpx(10.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgb(border))
                .children(friends.username.as_ref().map(|input| {
                    div().flex_1().min_w_0().child(
                        Input::new(input)
                            .appearance(false)
                            .px_0()
                            .text_size(rpx(16.)),
                    )
                }))
                .child(
                    div()
                        .id("friend-send")
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(rpx(32.))
                        .px(rpx(16.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::BLURPLE))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(0xFFFFFF))
                        .without_text_selection()
                        .when(can_send, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                                .on_click(move |_, _, cx| {
                                    send.update(cx, |app, cx| app.send_friend_request(cx)).ok();
                                })
                        })
                        .when(!can_send, |this| this.opacity(0.5))
                        .when(!SEND_REQUESTS, |this| {
                            this.relative()
                                .child(theme::hover_tooltip("friend-send-tip", SEND_FROM_DISCORD))
                        })
                        .child("Send Friend Request"),
                ),
        )
        .children(note.map(|(color, text)| {
            div()
                .mt(rpx(8.))
                .text_size(rpx(14.))
                .text_color(rgb(color))
                .child(text)
        }))
}

pub(crate) fn remove_dialog(app: &DiscordApp) -> Option<AnyElement> {
    let user_id = app.home.friends.confirm_remove?;
    let name = app.store.friend_name(user_id);
    let backdrop = app.entity.clone();
    let cancel = app.entity.clone();
    let confirm = app.entity.clone();
    let button = |id: &'static str, text: &'static str, bg: u32, hover: u32| {
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(rpx(44.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .bg(rgb(bg))
            .hover(move |style| style.bg(rgb(hover)))
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::strong()))
            .child(text)
    };
    Some(
        div()
            .id("friend-remove-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop
                    .update(cx, |app, cx| app.cancel_remove_friend(cx))
                    .ok();
            })
            .child(
                div()
                    .id("friend-remove-dialog")
                    .flex()
                    .flex_col()
                    .w(rpx(440.))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::strong()))
                            .child(format!("Remove '{name}'")),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(rpx(16.))
                            .text_color(rgb(theme::muted()))
                            .child(format!(
                                "Are you sure you want to remove {name} from your friends?"
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_6()
                            .child(
                                button(
                                    "friend-remove-cancel",
                                    "Cancel",
                                    theme::button_subtle(),
                                    theme::button_subtle_hover(),
                                )
                                .border_1()
                                .border_color(rgba(theme::border()))
                                .on_click(move |_, _, cx| {
                                    cancel
                                        .update(cx, |app, cx| app.cancel_remove_friend(cx))
                                        .ok();
                                }),
                            )
                            .child(
                                button(
                                    "friend-remove-confirm",
                                    "Remove Friend",
                                    0xD22D39,
                                    0xB82A33,
                                )
                                .on_click(move |_, _, cx| {
                                    confirm
                                        .update(cx, |app, cx| app.confirm_remove_friend(cx))
                                        .ok();
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
