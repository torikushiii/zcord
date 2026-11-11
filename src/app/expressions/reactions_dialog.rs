use gpui_component::scroll::ScrollableElement as _;

use super::super::*;
use crate::{
    discord::model,
    store::{self, Reaction},
};

const WIDTH: f32 = 480.;
const HEIGHT: f32 = 800.;
const REACTION_COLUMN: f32 = 122.;
const PAGE: u32 = 100;
const MAX_PAGES: usize = 10;

#[derive(Default)]
struct Reactors {
    users: Vec<model::User>,
    loading: bool,
    burst: bool,
    after: Option<u64>,
    pages: usize,
    complete: bool,
}

pub(crate) struct ReactionsDialog {
    opening: u64,
    channel_id: u64,
    guild_id: Option<u64>,
    message_id: u64,
    selected: String,
    reactors: HashMap<String, Reactors>,
    focus: FocusHandle,
}

impl DiscordApp {
    pub(crate) fn open_reactions_dialog(
        &mut self,
        message_id: u64,
        emoji: Option<&ReactionEmoji>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = match emoji {
            Some(emoji) => reaction_emoji_text(emoji),
            None => {
                let Some(first) = self.message_reactions(message_id).first() else {
                    return;
                };
                reaction_emoji_text(&first.emoji)
            }
        };
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.menus.reactions_openings += 1;
        self.menus.reactions = Some(ReactionsDialog {
            opening: self.menus.reactions_openings,
            channel_id: self
                .message_channel(message_id)
                .map_or(self.selected_channel_id, |channel| channel.id),
            guild_id: self.request_guild(),
            message_id,
            selected: selected.clone(),
            reactors: HashMap::new(),
            focus,
        });
        self.load_reactors(selected, cx);
        cx.notify();
    }

    pub(crate) fn close_reactions_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menus.reactions.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn select_reaction(&mut self, emoji: String, cx: &mut Context<Self>) {
        let Some(dialog) = self.menus.reactions.as_mut() else {
            return;
        };
        dialog.selected = emoji.clone();
        self.load_reactors(emoji, cx);
        cx.notify();
    }

    pub(super) fn message_reactions(&self, message_id: u64) -> &[Reaction] {
        self.message_channel(message_id)
            .and_then(|channel| {
                channel
                    .messages
                    .iter()
                    .find(|message| message.id == message_id)
            })
            .map_or(&[], |message| message.reactions.as_slice())
    }

    pub(super) fn message_channel(&self, message_id: u64) -> Option<&Channel> {
        let holds = |channel: &&Channel| {
            channel
                .messages
                .iter()
                .any(|message| message.id == message_id)
        };
        self.channel()
            .filter(holds)
            .or_else(|| self.thread_pane_channel().filter(holds))
    }

    fn load_reactors(&mut self, emoji: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(dialog) = self.menus.reactions.as_mut() else {
            return;
        };
        let reactors = dialog.reactors.entry(emoji.clone()).or_default();
        if reactors.loading || reactors.complete || reactors.pages >= MAX_PAGES {
            return;
        }
        reactors.loading = true;
        reactors.pages += 1;
        let page = model::ReactorPage {
            limit: PAGE,
            after: reactors.after,
            burst: reactors.burst,
        };
        let (opening, guild_id, channel_id, message_id) = (
            dialog.opening,
            dialog.guild_id,
            dialog.channel_id,
            dialog.message_id,
        );
        let request = Tokio::spawn(cx, {
            let emoji = emoji.clone();
            async move {
                session
                    .reaction_users(guild_id, channel_id, message_id, &emoji, page)
                    .await
            }
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let Some(reactors) = app
                    .menus
                    .reactions
                    .as_mut()
                    .filter(|dialog| dialog.opening == opening)
                    .and_then(|dialog| dialog.reactors.get_mut(&emoji))
                    .filter(|reactors| {
                        reactors.after == page.after && reactors.burst == page.burst
                    })
                else {
                    return;
                };
                reactors.loading = false;
                match result {
                    Ok(Ok(users)) => {
                        let last_page = users.len() < PAGE as usize;
                        reactors.after = users.last().map(|user| user.id);
                        for user in users {
                            if !reactors.users.iter().any(|seen| seen.id == user.id) {
                                reactors.users.push(user);
                            }
                        }
                        if last_page && !reactors.burst {
                            reactors.burst = true;
                            reactors.after = None;
                        } else if last_page {
                            reactors.complete = true;
                        }
                    }
                    Ok(Err(error)) => {
                        log::warn!("Loading reactors failed: {error}");
                        reactors.complete = true;
                    }
                    Err(error) => {
                        log::warn!("Reactor task failed: {error}");
                        reactors.complete = true;
                    }
                }
                app.load_reactors(emoji, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn reaction_tab(
    app: &DiscordApp,
    reaction: &Reaction,
    selected: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    let key = reaction_emoji_text(&reaction.emoji);
    div()
        .id(SharedString::from(format!("reactions-tab-{key}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(32.))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.bg(rgba(theme::selected()))
            } else {
                this.hover(|style| style.bg(rgba(theme::hover())))
            }
        })
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.select_reaction(key.clone(), cx))
                .ok();
        })
        .child(
            div()
                .flex_none()
                .size(rpx(24.))
                .child(messages::embeds::emoji_element(&reaction.emoji, 24., cx)),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::text()))
                .child(reaction.count.to_string()),
        )
}

fn reactor_row(app: &DiscordApp, user: &model::User, cx: &App) -> impl IntoElement + use<> {
    let guild_id = app.request_guild().unwrap_or(0);
    let spec = app
        .store
        .member_avatar_spec(guild_id, user.id)
        .unwrap_or_else(|| {
            store::avatar_spec(
                user.id,
                user.avatar.as_deref(),
                user.discriminator.as_deref(),
            )
        });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(10.))
        .h(rpx(52.))
        .child(theme::spec_avatar(&spec, user.id, &user.username, 32., cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child(app.reactor_name(user)),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::muted()))
                        .child(crate::settings::shown_username(&user.username)),
                ),
        )
}

pub(crate) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let dialog = app
        .menus
        .reactions
        .as_ref()
        .filter(|dialog| dialog.channel_id == app.selected_channel_id)?;
    let reactions = app.message_reactions(dialog.message_id);
    let reactors = dialog.reactors.get(&dialog.selected);
    let (backdrop, close) = (app.entity.clone(), app.entity.clone());
    let tabs = reactions.iter().map(|reaction| {
        let selected = reaction_emoji_text(&reaction.emoji) == dialog.selected;
        reaction_tab(app, reaction, selected, cx)
    });
    let rows = reactors
        .into_iter()
        .flat_map(|reactors| &reactors.users)
        .map(|user| reactor_row(app, user, cx));
    Some(
        div()
            .id("reactions-backdrop")
            .absolute()
            .inset_0()
            .top(rpx(titlebar::HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop
                    .update(cx, |app, cx| app.close_reactions_dialog(cx))
                    .ok();
            })
            .child(theme::selection_guard())
            .child(
                div()
                    .id("reactions-dialog")
                    .track_focus(&dialog.focus)
                    .flex()
                    .flex_col()
                    .w(rpx(WIDTH))
                    .h(rpx(HEIGHT))
                    .max_w_full()
                    .max_h(relative(0.9))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::dialog_bg()))
                    .border_1()
                    .border_color(rgba(theme::dialog_border()))
                    .shadow_lg()
                    .overflow_hidden()
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_between()
                            .p(rpx(24.))
                            .border_b_1()
                            .border_color(rgba(theme::border()))
                            .child(
                                div()
                                    .text_size(rpx(20.))
                                    .line_height(rpx(24.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::strong()))
                                    .child("Reactions"),
                            )
                            .child(
                                div()
                                    .id("reactions-close")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(rpx(32.))
                                    .rounded(rpx(8.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(theme::hover())))
                                    .on_click(move |_, _, cx| {
                                        close
                                            .update(cx, |app, cx| app.close_reactions_dialog(cx))
                                            .ok();
                                    })
                                    .child(theme::icon("x", 20., theme::muted())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .py(rpx(16.))
                            .child(
                                div()
                                    .id("reactions-tabs")
                                    .flex()
                                    .flex_col()
                                    .flex_none()
                                    .gap(rpx(8.))
                                    .w(rpx(REACTION_COLUMN))
                                    .px(rpx(24.))
                                    .border_r_1()
                                    .border_color(rgba(theme::border()))
                                    .overflow_y_scrollbar()
                                    .children(tabs),
                            )
                            .child(
                                div()
                                    .id("reactions-reactors")
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .px(rpx(16.))
                                    .overflow_y_scrollbar()
                                    .children(rows),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
