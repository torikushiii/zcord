use super::dm_menu;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{
    app::{
        ComposerDraft, DiscordApp, chat,
        display_name::{self, Surface},
        friends, member_list,
        message_body::label,
        profile::presence_art,
        quests, search, set_text, voice,
    },
    discord::{Error, Event, ProfileKind, model::PublicApplication},
    media::images::{self, ImageSpec},
    store::{
        ActiveVoice, GameActivity, LiveStream, PrivateChannel, RelationshipKind, Status, Store,
        User,
    },
    theme::{self, WithoutTextSelection, rpx},
};

const NAV_HEIGHT: f32 = 42.;
const DM_HEIGHT: f32 = 42.;
const DM_AVATAR: f32 = 32.;
const ACTIVE_NOW_WIDTH: f32 = 420.;
const ACTIVE_ICON: f32 = 24.;
const ACTIVE_ART: f32 = 32.;
const ACTIVE_BADGE: f32 = 16.;
const ACTIVE_CARD_GROUP: &str = "active-card";
const ACTIVE_PARTY_AVATAR: f32 = 24.;
const ACTION_SIZE: f32 = 36.;
const SPOTIFY_GREEN: u32 = 0x1DB954;
const AFFINITY_REFRESH: Duration = Duration::from_secs(8 * 60 * 60);
const TWITCH_PURPLE: u32 = 0x9146FF;
const YOUTUBE_RED: u32 = 0xFF0000;
const TWITCH_PREVIEW_WIDTH: f32 = 354.;
const TWITCH_PREVIEW_HEIGHT: f32 = 197.;
const TWITCH_PREVIEW_REFRESH: u64 = 300;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FriendsTab {
    #[default]
    Online,
    All,
    Pending,
    Suggestions,
    Add,
}

impl FriendsTab {
    fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::All => "All",
            Self::Pending => "Pending",
            Self::Suggestions => "Suggestions",
            Self::Add => "Add Friend",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HomePage {
    #[default]
    Friends,
    Requests,
    Quests,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RequestsTab {
    #[default]
    Requests,
    Spam,
}

#[derive(Default)]
pub(crate) struct Home {
    pub(crate) page: HomePage,
    tab: FriendsTab,
    pub(crate) friends: friends::Friends,
    pub(crate) quests: quests::QuestHome,
    requests_tab: RequestsTab,
    previews_requested: HashSet<u64>,
    suggestions_loading: bool,
    /// The people row under the pointer; the dividers on either side of it hide.
    hovered_row: Option<u64>,
    /// The row the pointer last left, which plays its slide-back animation.
    left_row: Option<u64>,
    /// Mutual server ids per message request sender, from their profile.
    mutuals: HashMap<u64, Vec<u64>>,
    mutuals_requested: HashSet<u64>,
    /// Failed profile loads per sender, so a failing one is retried a few times, spaced out.
    mutuals_failures: HashMap<u64, u32>,
    requests_scroll: UniformListScrollHandle,
    applications: HashMap<u64, Arc<PublicApplication>>,
    applications_requested: HashSet<u64>,
    search: Option<Entity<InputState>>,
    _search_events: Option<Subscription>,
}

pub(crate) fn placeholder(store: &Store, channel_id: u64, name: &str) -> String {
    match store.private_channel(channel_id) {
        Some(dm) if !dm.group => format!("Message @{name}"),
        _ => format!("Message {name}"),
    }
}

impl DiscordApp {
    fn enter_home(&mut self) {
        if self.is_home() {
            return;
        }
        if self.hovered_rail != Some(self.selected_guild_id) {
            self.rail_morphs.start(self.selected_guild_id, false);
        }
        self.member_rows.reset(0);
        self.member_layout = member_list::Layout::default();
        self.member_top = 0;
        self.dismiss_stream_card();
        self.selected_guild_id = 0;
    }

    pub(crate) fn open_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.enter_home();
        let saved = crate::settings::get(cx).guild_channels.get(&0).copied();
        match saved.filter(|id| self.store.private_channel(*id).is_some()) {
            Some(id) => self.select_channel(id, window, cx),
            None => self.open_friends(window, cx),
        }
    }

    pub(crate) fn open_friends(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_home_page(HomePage::Friends, window, cx);
        if self.home.tab == FriendsTab::Suggestions {
            self.load_friend_suggestions(cx);
        }
    }

    pub(crate) fn open_friend_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.enter_home();
        self.home.tab = FriendsTab::Pending;
        self.open_friends(window, cx);
    }

    fn open_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_home_page(HomePage::Requests, window, cx);
    }

    fn open_quests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_home_page(HomePage::Quests, window, cx);
        self.load_quests(cx);
    }

    /// Lets a failed sender profile load again once its row is next drawn, after a delay that
    /// grows with each failure (at least what a rate limit asks), until it gives up.
    fn retry_request_profile(&mut self, user_id: u64, error: &Error, cx: &mut Context<Self>) {
        const ATTEMPTS: u32 = 3;
        const FIRST_DELAY: Duration = Duration::from_secs(15);
        let failures = self.home.mutuals_failures.entry(user_id).or_insert(0);
        *failures += 1;
        if *failures >= ATTEMPTS {
            return;
        }
        let mut delay = FIRST_DELAY * 2u32.pow(*failures - 1);
        if let Error::RateLimited { retry_after } = error {
            delay = delay.max(*retry_after);
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |app, cx| {
                app.home.mutuals_requested.remove(&user_id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Loads what the visible request rows show: message previews and each sender's mutual
    /// servers. Like Discord, only rows on screen are fetched, all at once, so scrolling a new
    /// row in costs one profile and one preview request.
    fn load_visible_requests(&mut self, rows: Vec<(u64, Option<u64>)>, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let previews: Vec<u64> = rows
            .iter()
            .map(|(channel_id, _)| *channel_id)
            .filter(|channel_id| self.home.previews_requested.insert(*channel_id))
            .collect();
        if !previews.is_empty() {
            let session = session.clone();
            let request = Tokio::spawn(cx, async move {
                session.message_request_previews(previews).await
            });
            cx.spawn(async move |this, cx| {
                let result = request.await;
                this.update(cx, |app, cx| match result {
                    Ok(Ok(previews)) => {
                        app.store.set_request_previews(previews);
                        cx.notify();
                    }
                    Ok(Err(error)) => log::warn!("Could not load message requests: {error}"),
                    Err(error) => log::warn!("Message request task failed: {error}"),
                })
                .ok();
            })
            .detach();
        }
        let profiles: Vec<(u64, u64)> = rows
            .into_iter()
            .filter_map(|(channel_id, user_id)| Some((user_id?, channel_id)))
            .filter(|(user_id, _)| self.home.mutuals_requested.insert(*user_id))
            .collect();
        if profiles.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let requests: Vec<_> = profiles
                .into_iter()
                .map(|(user_id, channel_id)| {
                    let session = session.clone();
                    Tokio::spawn(cx, async move {
                        let profile = session
                            .profile(user_id, None, Some(channel_id), ProfileKind::Modal)
                            .await;
                        (user_id, profile)
                    })
                })
                .collect();
            let results = futures::future::join_all(requests).await;
            this.update(cx, |app, cx| {
                for result in results {
                    match result {
                        Ok((user_id, Ok(profile))) => {
                            let ids = profile
                                .mutual_guilds
                                .iter()
                                .flatten()
                                .map(|guild| guild.id)
                                .collect();
                            app.home.mutuals.insert(user_id, ids);
                        }
                        // Discord answers 404 for many request senders (deleted or private
                        // accounts) and shows "No Mutual Servers" for them.
                        Ok((user_id, Err(Error::Http(404) | Error::Rejected { .. }))) => {
                            app.home.mutuals.insert(user_id, Vec::new());
                        }
                        Ok((user_id, Err(error))) => {
                            log::warn!("Could not load request sender profile: {error}");
                            app.retry_request_profile(user_id, &error, cx);
                        }
                        Err(error) => log::warn!("Request profile task failed: {error}"),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_home_page(&mut self, page: HomePage, window: &mut Window, cx: &mut Context<Self>) {
        self.enter_home();
        self.dismiss_call_view();
        self.home.page = page;
        self.revealed_spoilers.clear();
        let previous = self.selected_channel_id;
        let draft = self.composer.read(cx).value().to_string();
        if draft.is_empty() {
            self.drafts.remove(&previous);
        } else {
            self.drafts.insert(
                previous,
                ComposerDraft {
                    content: draft,
                    mentions: self.mention_names.clone(),
                },
            );
        }
        self.composer
            .update(cx, |input, cx| set_text(input, String::new(), window, cx));
        self.profile = None;
        self.reply = None;
        self.editing = None;
        self.confirm_delete = None;
        self.stop_player(Some(window), cx);
        self.close_lightbox(cx);
        self.close_reactions_dialog(cx);
        self.dismiss_reaction_tooltip();
        self.selected_channel_id = 0;
        self.reset_mention_draft();
        self.unread_since = None;
        self.unread_bar = false;
        self.sync_channel_rows();
        self.messages.reset(0);
        if self.home.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
            self.home._search_events = Some(cx.subscribe(&search, |_, _, event, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }));
            self.home.search = Some(search);
        }
        self.ensure_friend_input(window, cx);
        if self.live {
            crate::settings::update(cx, |settings| {
                settings.guild_channels.remove(&0);
                settings.last_channel = Some(crate::settings::SavedChannel {
                    guild_id: 0,
                    channel_id: 0,
                });
            });
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Discord keeps this list for eight hours before asking for a new one.
    pub(super) fn load_user_affinities(&mut self, cx: &mut Context<Self>) {
        self.affinity_refresh = None;
        if !self.store.personalization_consented() {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.affinity_refresh = Some(cx.spawn(async move |this, cx| {
            loop {
                let session = session.clone();
                let request = Tokio::spawn(cx, async move { session.user_affinities().await });
                let result = request.await;
                let alive = this.update(cx, |app, cx| match result {
                    Ok(Ok(affinities)) => {
                        app.store.set_user_affinities(&affinities);
                        cx.notify();
                    }
                    Ok(Err(error)) => log::warn!("Could not load user affinities: {error}"),
                    Err(error) => log::warn!("User affinities task failed: {error}"),
                });
                if alive.is_err() {
                    return;
                }
                cx.background_executor().timer(AFFINITY_REFRESH).await;
            }
        }));
    }

    /// Fetches app icons for Active Now; Discord shows the game's icon even when the
    /// presence carries no image of its own.
    pub(crate) fn load_active_now_apps(&mut self, cx: &mut Context<Self>) {
        if !self.showing_home_page() || self.home.page != HomePage::Friends {
            return;
        }
        let ids: Vec<u64> = self
            .store
            .active_now()
            .into_iter()
            .filter_map(|id| self.store.user(id)?.game.as_ref()?.application_id)
            .collect();
        self.load_game_apps(ids, cx);
    }

    pub(crate) fn load_game_apps(&mut self, ids: Vec<u64>, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let ids: Vec<u64> = ids
            .into_iter()
            .filter(|id| self.home.applications_requested.insert(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let requested = ids.clone();
        let request = Tokio::spawn(cx, async move { session.public_applications(&ids).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(applications)) => {
                    for application in applications {
                        app.home.applications.insert(application.id, application);
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                    app.views.members.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }
                Ok(Err(error)) => {
                    log::warn!("Could not load game applications: {error}");
                    app.forget_requested_apps(&requested);
                }
                Err(error) => {
                    log::warn!("Game applications task failed: {error}");
                    app.forget_requested_apps(&requested);
                }
            })
            .ok();
        })
        .detach();
    }

    fn forget_requested_apps(&mut self, ids: &[u64]) {
        for id in ids {
            self.home.applications_requested.remove(id);
        }
    }

    pub(crate) fn game_icon_spec(&self, application_id: u64) -> Option<ImageSpec> {
        let hash = self.home.applications.get(&application_id)?.icon.clone()?;
        Some(ImageSpec::AppImage {
            application_id,
            hash,
            uploaded: false,
            animated: false,
        })
    }

    fn accept_request(&mut self, channel_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let change = self.store.accept_request_locally(channel_id);
        self.apply_change(change, window, cx);
        let request = Tokio::spawn(cx, async move {
            session.accept_message_request(channel_id).await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::warn!("Could not accept message request: {error}"),
            Err(error) => log::warn!("Accept request task failed: {error}"),
        })
        .detach();
    }

    fn ignore_request(&mut self, channel_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let open = self.selected_channel_id == channel_id;
        let change = self.store.remove_private_locally(channel_id);
        if open {
            self.open_requests(window, cx);
        } else {
            self.apply_change(change, window, cx);
        }
        let request = Tokio::spawn(cx, async move {
            session.reject_message_request(channel_id).await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not ignore message request: {error}"),
            Err(error) => log::warn!("Ignore request task failed: {error}"),
        })
        .detach();
    }

    fn select_requests_tab(&mut self, tab: RequestsTab, cx: &mut Context<Self>) {
        self.home.requests_tab = tab;
        self.home
            .requests_scroll
            .scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn hover_people_row(&mut self, user_id: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(user_id)
        } else if self.home.hovered_row == Some(user_id) {
            None
        } else {
            return;
        };
        if self.home.hovered_row != next {
            if let Some(previous) = self.home.hovered_row {
                self.home.left_row = Some(previous);
            }
            self.home.hovered_row = next;
            cx.notify();
        }
    }

    fn select_friends_tab(&mut self, tab: FriendsTab, cx: &mut Context<Self>) {
        self.home.tab = tab;
        if tab == FriendsTab::Suggestions {
            self.load_friend_suggestions(cx);
        }
        cx.notify();
    }

    /// `READY` only carries the count; the list is fetched when the tab first opens, and again
    /// after a reconnect replaces the store.
    fn load_friend_suggestions(&mut self, cx: &mut Context<Self>) {
        if self.home.suggestions_loading || self.store.friend_suggestions_loaded() {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.home.suggestions_loading = true;
        let request = Tokio::spawn(cx, async move { session.friend_suggestions().await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.home.suggestions_loading = false;
                match result {
                    Ok(Ok(suggestions)) => app.store.set_friend_suggestions(suggestions),
                    Ok(Err(error)) => log::warn!("Could not load friend suggestions: {error}"),
                    Err(error) => log::warn!("Friend suggestions task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn select_dm(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_home();
        self.select_channel(channel_id, window, cx);
        self.request_dm_call(channel_id);
    }

    /// Creates the DM on Discord first when none exists yet.
    pub(crate) fn open_dm(&mut self, user_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(channel_id) = self.store.dm_channel_with(user_id) {
            self.select_dm(channel_id, window, cx);
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, async move { session.open_dm(user_id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| match result {
                Ok(Ok(channel)) => {
                    let channel_id = channel.id;
                    let change = app.store.apply(Event::ChannelUpsert(Box::new(channel)));
                    app.apply_change(change, window, cx);
                    app.select_dm(channel_id, window, cx);
                }
                Ok(Err(error)) => {
                    log::warn!("Could not open DM: {error}");
                    app.note_dm_failure(user_id, &error, cx);
                }
                Err(error) => log::warn!("Open DM task failed: {error}"),
            })
            .ok();
        })
        .detach();
    }
}

fn count_label(count: u32) -> String {
    if count >= 1000 {
        format!("{}K+", count / 1000)
    } else {
        count.to_string()
    }
}

fn count_pill(count: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(rpx(16.))
        .min_w(rpx(16.))
        .px(rpx(4.))
        .rounded_full()
        .bg(rgb(theme::NOTIFICATION))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child(count_label(count))
}

pub(crate) fn sidebar(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    let find = div()
        .id("find-conversation")
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(8.))
        .bg(rgba(theme::hover()))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(rgba(theme::active()))
                .text_color(rgb(theme::text()))
        })
        .child("Find or start a conversation")
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.toggle_quick_switcher(window, cx))
                .ok();
        });
    let friends_selected = app.showing_home_page() && app.home.page == HomePage::Friends;
    let requests_selected = app.showing_home_page() && app.home.page == HomePage::Requests;
    let quests_selected = app.showing_home_page() && app.home.page == HomePage::Quests;
    let entity = app.entity.clone();
    let friends = nav_row(
        "home-friends",
        "users",
        "Friends",
        friends_selected,
        app.store.pending_count(),
    )
    .cursor_pointer()
    .when(!friends_selected, |this| {
        this.hover(|style| {
            style
                .bg(rgba(theme::hover()))
                .text_color(rgb(theme::text()))
        })
    })
    .on_click(move |_, window, cx| {
        entity
            .update(cx, |app, cx| app.open_friends(window, cx))
            .ok();
    });
    let selected = app.selected_channel_id;
    let rows: Vec<AnyElement> = app
        .store
        .dm_list()
        .into_iter()
        .map(|dm| dm_row(app, dm, dm.channel.id == selected, cx).into_any_element())
        .collect();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(rpx(chat::HEADER_HEIGHT))
                .px_2()
                .border_b_1()
                .border_color(rgba(theme::border()))
                .child(find),
        )
        .child(
            div()
                .id("dm-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_2()
                .py_2()
                .child(friends)
                // Not built yet; shown so the layout matches Discord's.
                .child(nav_row("home-library", "library", "Library", false, 0).opacity(0.5))
                .child(
                    nav_row("home-quests", "laurel-wreath", "Quests", quests_selected, 0)
                        .cursor_pointer()
                        .when(!quests_selected, |this| {
                            this.hover(|style| {
                                style
                                    .bg(rgba(theme::hover()))
                                    .text_color(rgb(theme::text()))
                            })
                        })
                        .on_click({
                            let entity = app.entity.clone();
                            move |_, window, cx| {
                                entity
                                    .update(cx, |app, cx| app.open_quests(window, cx))
                                    .ok();
                            }
                        }),
                )
                .child(
                    nav_row(
                        "home-requests",
                        "mail",
                        "Message Requests",
                        requests_selected,
                        app.store.message_request_count(),
                    )
                    .cursor_pointer()
                    .when(!requests_selected, |this| {
                        this.hover(|style| {
                            style
                                .bg(rgba(theme::hover()))
                                .text_color(rgb(theme::text()))
                        })
                    })
                    .on_click({
                        let entity = app.entity.clone();
                        move |_, window, cx| {
                            entity
                                .update(cx, |app, cx| app.open_requests(window, cx))
                                .ok();
                        }
                    }),
                )
                .child(
                    div()
                        .flex_none()
                        .h(rpx(1.))
                        .mx_2()
                        .my_2()
                        .bg(rgba(theme::border())),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .h(rpx(32.))
                        .px_2()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child("Direct Messages")
                        .child(
                            theme::icon_button("dm-create", "plus", 16., theme::muted()).on_click(
                                {
                                    let entity = app.entity.clone();
                                    move |_, _, cx| {
                                        entity
                                            .update(cx, |app, cx| {
                                                app.open_dm_dialog(
                                                    dm_menu::DmDialog::Recipients {
                                                        channel_id: None,
                                                        selected: Vec::new(),
                                                    },
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    }
                                },
                            ),
                        ),
                )
                .children(rows),
        )
}

fn nav_row(
    id: &'static str,
    icon: &'static str,
    text: &'static str,
    selected: bool,
    badge: u32,
) -> Stateful<Div> {
    let color = if selected {
        theme::strong()
    } else {
        theme::muted()
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(NAV_HEIGHT))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .child(theme::icon(icon, 20., color))
        .child(div().flex_1().child(text))
        .when(badge > 0, |this| this.child(count_pill(badge)))
}

fn dm_avatar(
    app: &DiscordApp,
    dm: &PrivateChannel,
    ring: u32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    if !dm.group {
        let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
        let user = app.store.user(user_id);
        let status = user.map_or(Status::Offline, |user| user.status);
        return theme::user_avatar(user, user_id, DM_AVATAR, Some((status, ring)), animate, cx)
            .into_any_element();
    }
    let image = dm
        .icon
        .as_ref()
        .map(|hash| ImageSpec::ChannelIcon {
            channel_id: dm.channel.id,
            hash: hash.clone(),
        })
        .and_then(|spec| images::resolve(&spec, theme::image_px(DM_AVATAR), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(DM_AVATAR))
            .flex_none()
            .rounded_full()
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(DM_AVATAR))
            .rounded_full()
            .bg(theme::user_color(dm.channel.id))
            .child(theme::icon("users", 18., theme::strong()))
            .into_any_element(),
    }
}

fn subtitle(app: &DiscordApp, dm: &PrivateChannel) -> Option<(String, Option<&'static str>)> {
    if dm.group {
        let members = dm.recipient_ids.len() + 1;
        return Some((format!("{members} Members"), None));
    }
    let user = app.store.user(*dm.recipient_ids.first()?)?;
    let text = user.activity.clone().filter(|_| user.status.is_online())?;
    Some((text, theme::activity_icon(user)))
}

fn dm_row(
    app: &DiscordApp,
    dm: &PrivateChannel,
    selected: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let id = dm.channel.id;
    let unread = (dm.channel.unread || dm.channel.mentions > 0) && !selected;
    let name = app.store.private_channel_name(id);
    let subtitle = subtitle(app, dm);
    let text = if selected || unread {
        theme::strong()
    } else {
        theme::muted()
    };
    let ring = if selected {
        theme::composite(theme::selected(), theme::base_lower())
    } else {
        theme::base_lower()
    };
    let recipient = dm.recipient_ids.first().copied().filter(|_| !dm.group);
    let user = recipient.and_then(|user_id| app.store.user(user_id));
    let hovered = recipient.is_some() && app.hovered_member == recipient;
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx));
    let dm_name_line = div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(if unread {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .child(div().min_w_0().truncate().child(display_name::render(
            ("dm-name", id),
            &name,
            user.and_then(|user| user.name_style.as_ref()),
            Surface::Account { hovered },
            cx,
        )))
        .children(tag);
    let dm_subtitle = subtitle.map(|(text, icon)| {
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .min_w_0()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::muted()))
            .children(icon.map(|icon| theme::icon(icon, 14., theme::GREEN)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(label(("dm-subtitle", id), &text)),
            )
    });
    let nameplate = user.zip(recipient).and_then(|(user, user_id)| {
        member_list::member_nameplate_layer(
            user.nameplate.as_ref()?,
            user_id,
            DM_HEIGHT,
            hovered,
            member_list::ROW_RADIUS,
            cx,
        )
    });
    let plated = nameplate.is_some();
    let blocked = recipient.is_some_and(|user_id| {
        app.store
            .relationship(user_id)
            .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
    });
    let dim = rgba(
        if selected {
            ring
        } else if hovered {
            theme::composite(theme::hover(), theme::base_lower())
        } else {
            theme::base_lower()
        } << 8
            | member_list::NAMEPLATE_DIM,
    );
    let entity = app.entity.clone();
    let row = div()
        .id(("dm", id))
        .relative()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .w_full()
        .h(rpx(DM_HEIGHT))
        .px_2()
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .text_color(rgb(text))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .when(blocked && !selected, |this| this.opacity(0.35))
        .when(!selected, |this| {
            this.hover(|style| {
                style
                    .bg(rgba(theme::hover()))
                    .text_color(rgb(theme::text()))
                    .opacity(1.)
            })
        })
        .when_some(recipient, |this, user_id| {
            let entity = app.entity.clone();
            this.on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        if *hovered || app.hovered_member == Some(user_id) {
                            app.hovered_member = hovered.then_some(user_id);
                            cx.notify();
                        }
                    })
                    .ok();
            })
        })
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.select_dm(id, window, cx))
                .ok();
        })
        .on_mouse_down(MouseButton::Right, {
            let entity = app.entity.clone();
            move |event, _, cx| {
                let position = event.position;
                entity
                    .update(cx, |app, cx| app.open_dm_menu(id, position, cx))
                    .ok();
            }
        })
        .group("dm-row")
        .children(nameplate)
        .when(plated, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(rpx(8. + DM_AVATAR + 12.))
                    .rounded_l(rpx(member_list::ROW_RADIUS))
                    .bg(dim),
            )
        })
        .child(dm_avatar(app, dm, ring, hovered, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .items_start()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .max_w_full()
                        .min_w_0()
                        .when(plated, |this| {
                            this.children(member_list::nameplate_mask(dim))
                        })
                        .child(dm_name_line)
                        .children(dm_subtitle),
                ),
        )
        .when(!dm.message_request, |this| {
            let close = app.entity.clone();
            let group = dm.group;
            this.child(
                div()
                    .id(("dm-close", id))
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(blocked, |this| {
                        this.child(
                            div()
                                .absolute()
                                .group_hover("dm-row", |style| style.opacity(0.))
                                .child(theme::icon("ban", 18., theme::muted())),
                        )
                    })
                    .child(
                        div()
                            .opacity(0.)
                            .group_hover("dm-row", |style| style.opacity(1.))
                            .child(theme::icon("x", 16., theme::muted())),
                    )
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        close
                            .update(cx, |app, cx| {
                                if group {
                                    app.open_dm_dialog(
                                        dm_menu::DmDialog::LeaveGroup {
                                            channel_id: id,
                                            silent: false,
                                        },
                                        cx,
                                    );
                                } else {
                                    app.close_dm(id, window, cx);
                                }
                            })
                            .ok();
                    }),
            )
        });
    div()
        .relative()
        .flex_none()
        .mt(rpx(2.))
        .when(unread, |this| {
            this.child(
                div()
                    .absolute()
                    .left(rpx(-8.))
                    .top(rpx(17.))
                    .w(rpx(4.))
                    .h(rpx(8.))
                    .rounded_r(rpx(4.))
                    .bg(rgb(theme::strong())),
            )
        })
        .child(row)
}

pub(crate) fn friends_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |tab: FriendsTab, badge: u32| {
        let selected = app.home.tab == tab;
        let entity = app.entity.clone();
        div()
            .id(tab.label())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            // Discord's tab `.item` min width, and 8px before its `.badge`.
            .min_w(rpx(40.))
            .gap(rpx(8.))
            .h(rpx(28.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .text_color(rgb(if selected {
                theme::strong()
            } else {
                theme::muted()
            }))
            .when(selected, |this| this.bg(rgba(theme::selected())))
            .when(!selected, |this| {
                this.hover(|style| {
                    style
                        .bg(rgba(theme::hover()))
                        .text_color(rgb(theme::text()))
                })
            })
            .child(tab.label())
            .when(badge > 0, |this| this.child(count_pill(badge)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_friends_tab(tab, cx))
                    .ok();
            })
    };
    let suggestions = app.store.friend_suggestion_count();
    let adding = app.home.tab == FriendsTab::Add;
    let open_add = app.entity.clone();
    let add_tab = div()
        .id("friends-add-tab")
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(28.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if adding { theme::BLURPLE } else { 0xFFFFFF }))
        .when(adding, |this| this.bg(rgba(theme::selected())))
        .when(!adding, |this| {
            this.bg(rgb(theme::BLURPLE))
                .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
        })
        .child(FriendsTab::Add.label())
        .on_click(move |_, _, cx| {
            open_add
                .update(cx, |app, cx| app.select_friends_tab(FriendsTab::Add, cx))
                .ok();
        });
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("users", 20., theme::faint()))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Friends"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::faint())),
        )
        .child(tab(FriendsTab::Online, 0))
        .child(tab(FriendsTab::All, 0))
        .child(tab(FriendsTab::Pending, app.store.pending_count()))
        .when(
            suggestions > 0 || app.home.tab == FriendsTab::Suggestions,
            |this| this.child(tab(FriendsTab::Suggestions, suggestions)),
        )
        .child(add_tab)
}

fn search_query(app: &DiscordApp, cx: &App) -> String {
    app.home
        .search
        .as_ref()
        .map(|search| search.read(cx).value().trim().to_lowercase())
        .unwrap_or_default()
}

fn status_text(user: Option<&User>, kind: RelationshipKind) -> String {
    match kind {
        RelationshipKind::Incoming => return "Incoming Friend Request".into(),
        RelationshipKind::Outgoing => return "Outgoing Friend Request".into(),
        _ => {}
    }
    let Some(user) = user else {
        return "Offline".into();
    };
    if let Some(activity) = user.activity.as_ref().filter(|_| user.status.is_online()) {
        return activity.clone();
    }
    match user.status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Offline",
    }
    .into()
}

pub(crate) fn friends_page(app: &DiscordApp, cx: &App) -> AnyElement {
    let query = search_query(app, cx);
    let tab = app.home.tab;
    if tab == FriendsTab::Add {
        return div()
            .flex()
            .size_full()
            .child(friends::add_friend_page(app))
            .child(active_now(app, cx))
            .into_any_element();
    }
    let ids = match tab {
        FriendsTab::Online => app.store.online_friends(),
        FriendsTab::All => app.store.friends(),
        FriendsTab::Pending => app.store.pending_requests(),
        FriendsTab::Suggestions => app
            .store
            .friend_suggestions()
            .into_iter()
            .map(|suggestion| suggestion.user_id)
            .collect(),
        FriendsTab::Add => Vec::new(),
    };
    let ids: Vec<u64> = ids
        .into_iter()
        .filter(|id| {
            query.is_empty()
                || app.store.friend_name(*id).to_lowercase().contains(&query)
                || app
                    .store
                    .user(*id)
                    .is_some_and(|user| user.username.to_lowercase().contains(&query))
        })
        .collect();
    let heading = format!(
        "{} — {}",
        match tab {
            FriendsTab::Online => "Online",
            FriendsTab::All => "All friends",
            FriendsTab::Pending => "Pending",
            FriendsTab::Suggestions => "Friend Suggestions",
            FriendsTab::Add => "",
        },
        ids.len()
    );
    let empty = ids.is_empty();
    let mut rows: Vec<AnyElement> = Vec::with_capacity(ids.len() * 2);
    for (index, id) in ids.iter().copied().enumerate() {
        if index > 0 {
            rows.push(row_divider(app, ids[index - 1], id).into_any_element());
        }
        rows.push(match tab {
            FriendsTab::Suggestions => suggestion_row(app, id, cx).into_any_element(),
            _ => friend_row(app, id, cx).into_any_element(),
        });
    }
    div()
        .flex()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .children(app.home.search.as_ref().map(|search| {
                    // Discord's `.searchBar`: 12px above, 8px below, 24px/16px at the sides.
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .mt(rpx(12.))
                        .mb(rpx(8.))
                        .ml(rpx(24.))
                        .mr(rpx(16.))
                        .h(rpx(36.))
                        .px(rpx(10.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::elevated()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(search)
                                    .appearance(false)
                                    .px_0()
                                    .text_size(rpx(16.)),
                            ),
                        )
                        .child(theme::icon("search", 18., theme::muted()))
                }))
                .child(
                    // Discord's `.tableHeader`.
                    div()
                        .flex_none()
                        .pt(rpx(16.))
                        .px(rpx(30.))
                        .pb(rpx(8.))
                        .border_b_1()
                        .border_color(rgba(theme::border_subtle()))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child(heading),
                )
                .children(
                    matches!(tab, FriendsTab::Pending | FriendsTab::Suggestions)
                        .then(|| app.pending_error())
                        .flatten()
                        .map(|text| {
                            div()
                                .flex_none()
                                .px(rpx(30.))
                                .pt(rpx(8.))
                                .text_size(rpx(14.))
                                .text_color(rgb(theme::RED))
                                .child(text)
                        }),
                )
                .child(
                    div()
                        .id("friends-list")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pb(rpx(8.))
                        .children(rows)
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .mt(rpx(40.))
                                    .text_center()
                                    .text_size(rpx(14.))
                                    .text_color(rgb(theme::muted()))
                                    .child(match tab {
                                        FriendsTab::Pending => {
                                            "There are no pending friend requests."
                                        }
                                        FriendsTab::Suggestions
                                            if !app.store.friend_suggestions_loaded() =>
                                        {
                                            "Loading suggestions…"
                                        }
                                        FriendsTab::Suggestions => {
                                            "No friend suggestions right now."
                                        }
                                        _ => "No one's around to play with Wumpus.",
                                    }),
                            )
                        }),
                ),
        )
        .child(active_now(app, cx))
        .into_any_element()
}

/// Discord's `.actionButton`: a 36px circle whose 20px icon brightens on hover and whose fill
/// deepens while `row_group` is hovered.
fn round_action(
    id: impl Into<ElementId>,
    icon: &'static str,
    row_group: impl Into<SharedString>,
) -> Stateful<Div> {
    let row_group = row_group.into();
    let group = SharedString::from(format!("{row_group}-{icon}"));
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(ACTION_SIZE))
        .rounded_full()
        .bg(rgb(theme::base_lower()))
        .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
        .cursor_pointer()
        .active(|style| style.bg(rgba(theme::active())))
        .child(
            theme::icon(icon, 20., theme::text())
                .group_hover(group, |style| style.text_color(rgb(theme::strong()))),
        )
}

/// A disabled `round_action` that only explains itself, at Discord's disabled opacity. Friend
/// request writes can trip Discord's captcha, so they point to the official client for now.
fn muted_action(
    id: (&'static str, u64),
    icon: &'static str,
    tip: &'static str,
    row_group: SharedString,
) -> Stateful<Div> {
    let tip_key = (tip, id.1);
    // The tooltip sits outside the dimmed circle so it keeps full opacity.
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(rpx(ACTION_SIZE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .rounded_full()
                .opacity(0.3)
                .bg(rgb(theme::base_lower()))
                .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
                .child(theme::icon(icon, 20., theme::text())),
        )
        .child(theme::hover_tooltip(tip_key, tip))
        // Keeps the row's click (open DM) from firing through the button.
        .on_click(|_, _, cx| cx.stop_propagation())
}

fn row_group(user_id: u64) -> SharedString {
    format!("people-row-{user_id}").into()
}

/// Discord's `.divider` between people rows: inset 30px/20px, hidden beside a hovered row so the
/// row's rounded highlight stands alone.
fn row_divider(app: &DiscordApp, above: u64, below: u64) -> Div {
    let hidden = matches!(app.home.hovered_row, Some(id) if id == above || id == below);
    div()
        .flex_none()
        .h(rpx(1.))
        .ml(rpx(30.))
        .mr(rpx(20.))
        .when(!hidden, |this| this.bg(rgba(theme::border_subtle())))
}

/// Discord's `.peopleListItem`: 14px/10px outside, 16px/10px inside, and a height that comes from
/// its content rather than a fixed value.
fn people_row(app: &DiscordApp, id: ElementId, user_id: u64) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .group(row_group(user_id))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .ml(rpx(14.))
        .mr(rpx(10.))
        .py(rpx(16.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .hover(|style| style.bg(rgba(theme::hover())))
        .without_text_selection()
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| app.hover_people_row(user_id, *hovered, cx))
                .ok();
        })
}

fn name_line(user_id: u64, name: &str, username: Option<String>, tag: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("people-name", user_id), name)),
        )
        .children(tag)
        .children(username.map(|username| {
            div()
                .invisible()
                .group_hover(row_group(user_id), |style| style.visible())
                .ml(rpx(4.))
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::text()))
                .child(username)
        }))
}

fn subtext_line(id: (&'static str, u64), text: &str, icon: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::muted()))
        .children(icon.map(|icon| theme::icon(icon, 12., theme::GREEN)))
        .child(div().flex_1().min_w_0().truncate().child(label(id, text)))
}

fn friend_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let kind = app
        .store
        .relationship(user_id)
        .map_or(RelationshipKind::Friend, |relationship| relationship.kind);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| user.username.clone());
    let status = user.map_or(Status::Offline, |user| user.status);
    let icon = user
        .filter(|user| user.status.is_online())
        .and_then(theme::activity_icon);
    let line = status_text(user, kind);
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx).into_any_element());
    let entity = app.entity.clone();
    let group = row_group(user_id);
    let pending = matches!(
        kind,
        RelationshipKind::Incoming | RelationshipKind::Outgoing
    );
    people_row(app, ("friend", user_id).into(), user_id)
        .cursor_pointer()
        // Like Discord: a friend opens the DM, someone with a pending request their profile.
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| {
                    if pending {
                        app.open_full_profile_for(user_id, cx);
                    } else {
                        app.open_dm(user_id, window, cx);
                    }
                })
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            DM_AVATAR,
            Some((status, theme::bg())),
            false,
            cx,
        ))
        .child(div().flex().flex_col().flex_1().min_w_0().map(|this| {
            if pending {
                // Like suggestions: the username sits under the name instead of
                // appearing on hover, since a request is usually from a stranger.
                let username = username.unwrap_or(line);
                this.child(name_line(user_id, &name, None, tag))
                    .child(subtext_line(("friend-status", user_id), &username, None))
            } else {
                this.child(name_line(user_id, &name, username, tag))
                    .child(subtext_line(("friend-status", user_id), &line, icon))
            }
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .when(kind == RelationshipKind::Incoming, |this| {
                    this.child(muted_action(
                        ("friend-accept", user_id),
                        "check",
                        "Accept friend requests from Discord for now",
                        group.clone(),
                    ))
                })
                .when(pending, |this| {
                    let tip = if kind == RelationshipKind::Incoming {
                        "Ignore friend requests from Discord for now"
                    } else {
                        "Cancel friend requests from Discord for now"
                    };
                    this.child(muted_action(
                        ("friend-drop", user_id),
                        "x",
                        tip,
                        group.clone(),
                    ))
                })
                .when(!pending, |this| {
                    this.child(round_action(
                        ("friend-message", user_id),
                        "message-circle",
                        group.clone(),
                    ))
                }),
        )
}

/// A suggested user with the name Discord matched them by, plus add and dismiss buttons.
fn suggestion_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| user.username.clone()).unwrap_or_default();
    let reason = app
        .store
        .friend_suggestions()
        .into_iter()
        .find(|suggestion| suggestion.user_id == user_id)
        .and_then(|suggestion| suggestion.reason.clone())
        .unwrap_or_else(|| username.clone());
    let group = row_group(user_id);
    let entity = app.entity.clone();
    people_row(app, ("suggestion", user_id).into(), user_id)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.open_full_profile_for(user_id, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user, user_id, DM_AVATAR, None, false, cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(name_line(user_id, &name, None, None))
                .child(subtext_line(("suggestion-reason", user_id), &reason, None)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .child(muted_action(
                    ("suggestion-add", user_id),
                    "check",
                    "Send friend requests from Discord for now",
                    group.clone(),
                ))
                .child(muted_action(
                    ("suggestion-dismiss", user_id),
                    "x",
                    "Dismiss suggestions from Discord for now",
                    group,
                )),
        )
}

fn elapsed(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h", minutes / 60)
    }
}

fn activity_elapsed(started: Option<SystemTime>, now: SystemTime) -> Option<Duration> {
    now.duration_since(started?).ok()
}

fn shows_elapsed_line(game: &GameActivity) -> bool {
    game.started_at.is_some() && (game.details.is_some() || game.state.is_some())
}

pub(super) fn needs_active_now_clock(app: &DiscordApp) -> bool {
    app.showing_home_page()
        && app.home.page == HomePage::Friends
        && app.store.active_now().into_iter().any(|id| {
            app.store
                .user(id)
                .and_then(|user| user.game.as_ref())
                .is_some_and(shows_elapsed_line)
        })
}

fn active_art(image: AnyElement, badge: Option<AnyElement>) -> Div {
    div()
        .relative()
        .flex_none()
        .size(rpx(ACTIVE_ART))
        .rounded(rpx(ACTIVE_ART / 5.))
        .bg(rgba(theme::selected()))
        .child(image)
        .children(badge.map(|badge| {
            div()
                .absolute()
                .right(rpx(-4.))
                .bottom(rpx(-4.))
                .size(rpx(ACTIVE_BADGE + 4.))
                .p(rpx(2.))
                .rounded_full()
                .bg(rgb(theme::bg()))
                .child(badge)
        }))
}

fn active_detail_text(
    id: u64,
    detail: &str,
    state: Option<&str>,
    duration: Option<Duration>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("active-detail", id), detail)),
        )
        .children(state.map(|state| {
            div()
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(label(("active-state", id), state))
        }))
        .children(duration.map(|duration| {
            div()
                .debug_selector(|| "active-elapsed".into())
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(format!("{} elapsed", voice::format_elapsed(duration)))
        }))
}

fn active_header(
    app: &DiscordApp,
    id: u64,
    user: &User,
    headline: &str,
    icon: Div,
    cx: &App,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .bg(rgb(theme::elevated()))
        .group_hover(ACTIVE_CARD_GROUP, |style| {
            style.bg(rgb(theme::composite(theme::hover(), theme::elevated())))
        })
        .child(theme::user_avatar(
            Some(user),
            id,
            DM_AVATAR,
            Some((user.status, theme::elevated())),
            false,
            cx,
        ))
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(display_name::render(
                            ("active-name", id),
                            &app.store.friend_name(id),
                            user.name_style.as_ref(),
                            Surface::FontOnly,
                            cx,
                        )),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::muted()))
                        .child(label(("active-game", id), headline)),
                ),
        )
        .child(icon)
}

fn active_card_frame() -> Div {
    div()
        .group(ACTIVE_CARD_GROUP)
        .flex()
        .flex_col()
        .flex_none()
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgba(theme::border()))
        .overflow_hidden()
        .hover(|style| style.bg(rgba(theme::hover())))
}

fn service_icon(service: &str, size: f32) -> Div {
    if service == "Twitch" {
        // Icons take one color, so the white panel of the logo is its own layer.
        return div()
            .relative()
            .flex_none()
            .size(rpx(size))
            .child(
                theme::icon("connections/twitch-fill", size, 0xFFFFFF)
                    .absolute()
                    .top_0()
                    .left_0(),
            )
            .child(
                theme::icon("connections/twitch", size, TWITCH_PURPLE)
                    .absolute()
                    .top_0()
                    .left_0(),
            );
    }
    let (icon, color) = match service {
        "YouTube" => ("connections/youtube", YOUTUBE_RED),
        _ => ("monitor-up", theme::muted()),
    };
    div()
        .flex_none()
        .size(rpx(size))
        .child(theme::icon(icon, size, color))
}

/// Twitch rebuilds this image every few minutes, so the bucket keeps it from being cached forever.
fn twitch_preview(login: &str, cx: &App) -> Option<Arc<std::path::Path>> {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let bucket = now / TWITCH_PREVIEW_REFRESH;
    let url = format!(
        "https://static-cdn.jtvnw.net/previews-ttv/live_user_{login}-900x500.jpg?t={bucket}"
    );
    images::resolve(
        &ImageSpec::Media {
            url,
            width: 0,
            height: 0,
            animated: false,
        },
        theme::image_px(TWITCH_PREVIEW_WIDTH),
        cx,
    )
}

fn live_stream_preview(id: u64, stream: &LiveStream, cx: &App) -> Div {
    let image = stream
        .twitch_login
        .as_deref()
        .and_then(|login| twitch_preview(login, cx));
    let url = stream.url.clone();
    div().child(
        div()
            .id(("active-stream-preview", id))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(rpx(TWITCH_PREVIEW_HEIGHT))
            .rounded(rpx(8.))
            .overflow_hidden()
            .bg(rgba(theme::selected()))
            .when_some(url, |this, url| {
                this.cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
            .children(image.map(|path| {
                img(path)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(rpx(8.))
                    .object_fit(ObjectFit::Cover)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(48.))
                    .rounded_full()
                    .bg(rgba(0x00000099))
                    .child(theme::icon("video-play", 24., 0xFFFFFF)),
            ),
    )
}

fn live_stream_card(
    app: &DiscordApp,
    id: u64,
    user: &User,
    stream: &LiveStream,
    cx: &App,
) -> AnyElement {
    let headline = format!("Streaming on {}", stream.service);
    let preview = stream
        .twitch_login
        .is_some()
        .then(|| live_stream_preview(id, stream, cx));
    active_card_frame()
        .child(active_header(
            app,
            id,
            user,
            &headline,
            service_icon(&stream.service, ACTIVE_ICON),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(12.))
                        .child(service_icon(&stream.service, ACTIVE_ART))
                        .child(active_detail_text(
                            id,
                            &stream.service,
                            stream.title.as_deref(),
                            None,
                        )),
                )
                .children(preview),
        )
        .into_any_element()
}

fn active_card(
    app: &DiscordApp,
    id: u64,
    user: &User,
    game: &GameActivity,
    cx: &App,
) -> AnyElement {
    let listening = user.game.is_none();
    let duration = activity_elapsed(game.started_at, SystemTime::now());
    let headline = if listening {
        format!("Listening to {}", game.name)
    } else {
        match duration {
            Some(duration) => format!("{} – {}", game.name, elapsed(duration)),
            None => game.name.clone(),
        }
    };
    let app_icon = |size: f32| {
        let spec = app.game_icon_spec(game.application_id?)?;
        let path = images::resolve(&spec, theme::image_px(size), cx)?;
        Some(
            img(path)
                .size_full()
                .rounded(rpx(size / 5.))
                .into_any_element(),
        )
    };
    let presence = |size: f32, key: &'static str| {
        let image = game.large_image.as_ref()?;
        presence_art(&image.url, size, size / 5., (key, id), cx)
    };
    let fallback = |size: f32| {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(rpx(size / 5.))
            .child(theme::icon("gamepad-2", size * 0.55, theme::muted()))
            .into_any_element()
    };
    // `presence_art` and the app icon fill their parent, so each needs a sized box.
    let icon =
        div()
            .flex_none()
            .size(rpx(ACTIVE_ICON))
            .child(if listening && game.name == "Spotify" {
                theme::icon("connections/spotify", ACTIVE_ICON, SPOTIFY_GREEN).into_any_element()
            } else {
                app_icon(ACTIVE_ICON)
                    .or_else(|| presence(ACTIVE_ICON, "active-icon"))
                    .unwrap_or_else(|| fallback(ACTIVE_ICON))
            });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            ACTIVE_BADGE,
            ACTIVE_BADGE,
            ("active-badge", id),
            cx,
        )
    });
    let art = active_art(
        presence(ACTIVE_ART, "active-art")
            .or_else(|| app_icon(ACTIVE_ART))
            .unwrap_or_else(|| fallback(ACTIVE_ART)),
        badge,
    );
    let detail = game.details.clone().or_else(|| game.state.clone());
    let state = game.details.as_ref().and(game.state.clone());
    active_card_frame()
        .child(active_header(app, id, user, &headline, icon, cx))
        .child(match detail {
            Some(detail) => div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(art)
                .child(active_detail_text(
                    id,
                    &detail,
                    state.as_deref(),
                    duration.filter(|_| !listening),
                )),
            None if listening => div(),
            None => div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(art)
                .child(active_detail_text(id, &game.name, Some("1 Person"), None))
                .child(theme::undecorated_member_avatar(
                    &app.store,
                    0,
                    id,
                    ACTIVE_PARTY_AVATAR,
                    cx,
                )),
        })
        .into_any_element()
}

fn voice_title(names: &[String], members: usize) -> String {
    let others = members.saturating_sub(names.len());
    let others_label =
        |count: usize| format!("{count} {}", if count == 1 { "other" } else { "others" });
    match (names, others) {
        ([], _) => others_label(others),
        ([a], 0) => a.clone(),
        ([a], _) => format!("{a} and {}", others_label(others)),
        ([a, b], 0) => format!("{a} and {b}"),
        ([a, b, ..], _) => format!("{a}, {b}, and {}", others_label(others)),
    }
}

fn voice_card(app: &DiscordApp, voice: &ActiveVoice, cx: &App) -> Option<AnyElement> {
    let lead_id = *voice.friend_ids.first()?;
    let lead = app.store.user(lead_id)?;
    let names: Vec<String> = voice
        .friend_ids
        .iter()
        .filter(|id| **id != app.store.current_user_id)
        .take(2)
        .map(|id| app.store.friend_name(*id))
        .collect();
    let guild = app
        .store
        .guilds
        .iter()
        .find(|guild| guild.id == voice.guild_id)?;
    let channel = app
        .store
        .channel_name(voice.channel_id)
        .unwrap_or_default()
        .to_owned();
    let icon = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(ACTIVE_ART), cx))
        .map(|path| img(path).size_full().rounded_full().into_any_element())
        .unwrap_or_else(|| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .rounded_full()
                .bg(rgb(guild.icon_color))
                .text_size(rpx(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white())
                .child(guild.initials.clone())
                .into_any_element()
        });
    let art = div()
        .relative()
        .flex_none()
        .size(rpx(ACTIVE_ART))
        .child(icon)
        .child(
            div()
                .absolute()
                .left(rpx(18.))
                .top(rpx(18.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(16.))
                .rounded_full()
                .bg(rgb(theme::bg()))
                .child(theme::icon("volume-2", 10., theme::text())),
        );
    let shown = voice.member_ids.len().min(3);
    let extra = voice.member_ids.len() - shown;
    let party = div()
        .flex()
        .flex_none()
        .items_center()
        .children(
            voice
                .member_ids
                .iter()
                .take(shown)
                .enumerate()
                .map(|(index, id)| {
                    div().when(index > 0, |this| this.ml(rpx(-2.))).child(
                        theme::undecorated_member_avatar(
                            &app.store,
                            0,
                            *id,
                            ACTIVE_PARTY_AVATAR,
                            cx,
                        ),
                    )
                }),
        )
        .when(extra > 0, |this| {
            this.child(
                div()
                    .ml(rpx(4.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::faint()))
                    .child(format!("+{extra}")),
            )
        });
    let header = div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .bg(rgb(theme::elevated()))
        .group_hover(ACTIVE_CARD_GROUP, |style| {
            style.bg(rgb(theme::composite(theme::hover(), theme::elevated())))
        })
        .child(theme::user_avatar(
            Some(lead),
            lead_id,
            DM_AVATAR,
            Some((lead.status, theme::elevated())),
            false,
            cx,
        ))
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::text()))
                        .child(voice_title(&names, voice.member_ids.len())),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::faint()))
                        .child("In a Voice Channel"),
                ),
        );
    let body = div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::text()))
                        .child(guild.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::faint()))
                        .child(channel),
                ),
        )
        .child(party);
    Some(
        active_card_frame()
            .child(header)
            .child(body)
            .into_any_element(),
    )
}

fn active_now(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let voice_cards = app
        .store
        .active_voice()
        .iter()
        .filter_map(|voice| voice_card(app, voice, cx))
        .collect::<Vec<_>>();
    let cards: Vec<AnyElement> = voice_cards
        .into_iter()
        .chain(app.store.active_now().into_iter().filter_map(|id| {
            let user = app.store.user(id)?;
            if let Some(stream) = &user.live_stream {
                return Some(live_stream_card(app, id, user, stream, cx));
            }
            let activity = user.game.as_ref().or(user.listening.as_ref())?;
            Some(active_card(app, id, user, activity, cx))
        }))
        .collect();
    let empty = cards.is_empty();
    div()
        .id("active-now")
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(ACTIVE_NOW_WIDTH))
        .h_full()
        .gap(rpx(16.))
        .p(rpx(16.))
        .overflow_y_scrollbar()
        .border_l_1()
        .border_color(rgba(theme::border()))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Active Now"),
        )
        .children(cards)
        .when(empty, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .mt_4()
                    .text_center()
                    .child(
                        div()
                            .text_size(rpx(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child("It's quiet for now..."),
                    )
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child(
                                "When a friend starts an activity, like playing a game, we'll show it here!",
                            ),
                    ),
            )
        })
}

pub(crate) fn dm_header(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let channel_id = app.selected_channel_id;
    let dm = app.store.private_channel(channel_id);
    let name = app.store.private_channel_name(channel_id);
    let user = dm
        .filter(|dm| !dm.group)
        .and_then(|dm| dm.recipient_ids.first())
        .and_then(|id| app.store.user(*id));
    let game = user
        .filter(|user| user.status.is_online() && user.playing)
        .and_then(|user| user.activity.clone());
    let group = dm.is_some_and(|dm| dm.group);
    let toggle = app.entity.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .pl_4()
        .pr_2()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .children(dm.map(|dm| {
            div()
                .flex_none()
                .size(rpx(24.))
                .child(header_avatar(app, dm, cx))
        }))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("header-dm", channel_id), &name)),
        )
        .children(game.map(|game| {
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .min_w_0()
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child(theme::icon("gamepad-2", 14., theme::GREEN))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label(("header-game", channel_id), &game)),
                )
        }))
        .children(dm_menu::header_error(app, channel_id))
        .child(div().flex_1())
        .child(dm_menu::header_actions(app, channel_id))
        .child(chat::pins_button(app))
        .child(
            theme::icon_button(
                "dm-members",
                if group { "users" } else { "user" },
                20.,
                if app.show_members {
                    theme::strong()
                } else {
                    theme::muted()
                },
            )
            .on_click(move |_, _, cx| {
                toggle.update(cx, |app, cx| app.toggle_members(cx)).ok();
            }),
        )
        .child(
            div()
                .flex_none()
                .w(rpx(244.))
                .ml_2()
                .mr_2()
                .child(search::search_box(app, cx)),
        )
}

fn header_avatar(app: &DiscordApp, dm: &PrivateChannel, cx: &App) -> AnyElement {
    if dm.group {
        return div()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(24.))
            .rounded_full()
            .bg(theme::user_color(dm.channel.id))
            .child(theme::icon("users", 14., theme::strong()))
            .into_any_element();
    }
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    theme::user_avatar(app.store.user(user_id), user_id, 24., None, false, cx).into_any_element()
}

pub(crate) fn dm_welcome(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    dm_intro(app, app.selected_channel_id, cx)
}

/// The top of a DM's history. A message request also gets Discord's sidebar controls under
/// the greeting: the sender's mutual servers, then Accept DM, Ignore and Report Spam.
pub(crate) fn dm_intro(app: &DiscordApp, channel_id: u64, cx: &App) -> Option<AnyElement> {
    let dm = app.store.private_channel(channel_id)?;
    let name = app.store.private_channel_name(channel_id);
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    let user = (!dm.group).then(|| app.store.user(user_id)).flatten();
    let text = if dm.group {
        format!("Welcome to the beginning of the {name} group.")
    } else {
        format!("This is the beginning of your direct message history with {name}.")
    };
    Some(
        div()
            .flex()
            .flex_col()
            .mx_4()
            .mt_4()
            .child(if dm.group {
                header_avatar(app, dm, cx)
            } else {
                theme::user_avatar(user, user_id, 80., None, false, cx).into_any_element()
            })
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(32.))
                    .line_height(rpx(40.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(rgb(theme::strong()))
                    .child(label(("welcome-dm", channel_id), &name)),
            )
            .children(user.map(|user| {
                div()
                    .text_size(rpx(20.))
                    .text_color(rgb(theme::text()))
                    .child(user.username.clone())
            }))
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(16.))
                    .text_color(rgb(theme::muted()))
                    .child(text),
            )
            .when(dm.message_request && !dm.group, |this| {
                this.child(request_intro_controls(app, dm, user_id, cx))
            })
            .into_any_element(),
    )
}

fn request_intro_controls(
    app: &DiscordApp,
    dm: &PrivateChannel,
    user_id: u64,
    cx: &App,
) -> impl IntoElement {
    let id = dm.channel.id;
    let buttons = if dm.spam {
        [RequestButton::NotSpam, RequestButton::Report].as_slice()
    } else {
        [
            RequestButton::Accept,
            RequestButton::Ignore,
            RequestButton::Report,
        ]
        .as_slice()
    };
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(rpx(8.))
        .mt(rpx(12.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
        .children(
            app.home
                .mutuals
                .get(&user_id)
                .map(|guild_ids| mutual_servers(app, guild_ids, cx)),
        )
        .when(app.home.mutuals.contains_key(&user_id), |this| {
            this.child(div().text_color(rgb(theme::faint())).child("•"))
        })
        .children(buttons.iter().map(|kind| request_button(app, id, *kind)))
}

fn header_tab(
    id: &'static str,
    text: impl Into<SharedString>,
    selected: bool,
    badge: u32,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(28.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme::strong()
        } else {
            theme::muted()
        }))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .when(!selected, |this| {
            this.hover(|style| {
                style
                    .bg(rgba(theme::hover()))
                    .text_color(rgb(theme::text()))
            })
        })
        .child(text.into())
        .when(badge > 0, |this| this.child(count_pill(badge)))
}

pub(crate) fn requests_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |id: &'static str, tab: RequestsTab, text: String| {
        let entity = app.entity.clone();
        header_tab(id, text, app.home.requests_tab == tab, 0).on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.select_requests_tab(tab, cx))
                .ok();
        })
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("mail", 20., theme::faint()))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Message Requests"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::faint())),
        )
        // Like Discord: no pill on Requests, and the spam count in the label.
        .child(tab(
            "requests-tab",
            RequestsTab::Requests,
            "Requests".to_owned(),
        ))
        .child(tab(
            "spam-tab",
            RequestsTab::Spam,
            match app.store.spam_request_count() {
                0 => "Spam".to_owned(),
                count => format!("Spam ({count})"),
            },
        ))
}

/// Accepting or ignoring a request can trip Discord's captcha like friend requests do, so both
/// stay in the official client for now. Flip this once captchas are handled.
const REQUEST_ACTIONS: bool = false;
const ACCEPT_FROM_DISCORD: &str = "Accept message requests from Discord for now";
const IGNORE_FROM_DISCORD: &str = "Ignore message requests from Discord for now";
/// Discord's message request avatar (`SIZE_40`).
const REQUEST_AVATAR: f32 = 40.;
const MUTUAL_ICON: f32 = 16.;
/// Height of the request row's second line, which slides between mutual servers and preview.
const REQUEST_LINE: f32 = 18.;
/// How far each stacked server icon sits from the previous one.
const MUTUAL_STEP: f32 = 12.;
/// Discord's `LIST_ROW_HEIGHT`: a 1px top border over a 72px row.
const REQUEST_ROW_HEIGHT: f32 = 73.;

pub(crate) fn requests_page(app: &DiscordApp, _cx: &App) -> AnyElement {
    let spam = app.home.requests_tab == RequestsTab::Spam;
    let count = if spam {
        app.store.spam_request_count()
    } else {
        app.store.message_request_count()
    } as usize;
    let heading = format!(
        "{} — {count}",
        if spam {
            "Spam Requests"
        } else {
            "Pending Requests"
        }
    );
    let title = div()
        .flex_none()
        .ml(rpx(10.))
        .mb(rpx(10.))
        .text_size(rpx(14.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(heading);
    let page = div()
        .flex()
        .flex_col()
        .size_full()
        .px(rpx(24.))
        .pt(rpx(24.))
        .child(title);
    if count == 0 {
        return page
            .child(
                div()
                    .mt(rpx(40.))
                    .text_center()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(if spam {
                        "There are no spam requests."
                    } else {
                        "There are no pending message requests."
                    }),
            )
            .into_any_element();
    }
    let entity = app.entity.clone();
    let now_ms = chrono::Utc::now().timestamp_millis();
    // Virtualized like Discord's list: only rows on screen are built, and only they fetch.
    let list = uniform_list("requests-list", count, move |range, _, cx| {
        let Some(app_entity) = entity.upgrade() else {
            return Vec::new();
        };
        let (items, missing, live, app_weak) = {
            let cx: &App = cx;
            let app = app_entity.read(cx);
            let requests = if app.home.requests_tab == RequestsTab::Spam {
                app.store.spam_requests()
            } else {
                app.store.message_requests()
            };
            let hovered = app.home.hovered_row;
            let mut missing = Vec::new();
            let items: Vec<AnyElement> = range
                .filter_map(|index| {
                    let dm = *requests.get(index)?;
                    let id = dm.channel.id;
                    let user_id = (!dm.group)
                        .then(|| dm.recipient_ids.first().copied())
                        .flatten();
                    if !app.home.previews_requested.contains(&id)
                        || user_id.is_some_and(|user| !app.home.mutuals_requested.contains(&user))
                    {
                        missing.push((id, user_id));
                    }
                    // Discord's border sits on top of every row but the first, and hides beside a
                    // hovered row.
                    let above = index.checked_sub(1).and_then(|above| requests.get(above));
                    let border = above.is_some_and(|above| {
                        let lit = |row: u64| hovered == Some(row) || app.thread_pane_shows(row);
                        !lit(id) && !lit(above.channel.id)
                    });
                    Some(
                        // Virtualized items size to their content unless told to fill the row.
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .h(rpx(REQUEST_ROW_HEIGHT))
                            .child(
                                div()
                                    .flex_none()
                                    .h(rpx(1.))
                                    .when(border, |this| this.bg(rgba(theme::border_subtle()))),
                            )
                            .child(
                                request_row(app, dm, now_ms, cx)
                                    .w_full()
                                    .h(rpx(REQUEST_ROW_HEIGHT - 1.))
                                    .py(rpx(0.)),
                            )
                            .into_any_element(),
                    )
                })
                .collect();
            (items, missing, app.live, app.entity.clone())
        };
        // Fetching mutates the app, which can't happen mid-paint, so it waits for the frame.
        if live && !missing.is_empty() {
            cx.defer(move |cx| {
                app_weak
                    .update(cx, |app, cx| app.load_visible_requests(missing, cx))
                    .ok();
            });
        }
        items
    })
    .track_scroll(&app.home.requests_scroll)
    .flex_1()
    .min_h_0()
    .pb(rpx(24.));
    page.child(list).into_any_element()
}

/// Discord's request age: minutes, hours and days up to 30, then ">30d ago" until 360 days,
/// then the full date.
fn request_age(sent_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - sent_ms).max(0) / 1000;
    match seconds {
        0..60 => "1m ago".to_owned(),
        60..3_600 => format!("{}m ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
        2_592_000..31_104_000 => ">30d ago".to_owned(),
        _ => chrono::DateTime::from_timestamp_millis(sent_ms)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%B %-d, %Y")
                    .to_string()
            })
            .unwrap_or_default(),
    }
}

/// One line of the latest message, as Discord previews it: line breaks become spaces, and
/// user mentions and custom emoji read as names rather than raw markup.
fn request_preview_line(store: &Store, content: &str) -> String {
    let flat = content.replace(['\r', '\n'], " ");
    let mut out = String::with_capacity(flat.len());
    let mut rest = flat.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            rest = "";
            break;
        };
        let inner = &tail[1..end];
        let mention = inner
            .strip_prefix("@!")
            .or_else(|| inner.strip_prefix('@'))
            .and_then(|id| id.parse::<u64>().ok());
        let emoji = inner
            .strip_prefix("a:")
            .or_else(|| inner.strip_prefix(':'))
            .and_then(|body| body.split_once(':'))
            .map(|(name, _)| name);
        match (mention, emoji) {
            (Some(id), _) => {
                out.push('@');
                out.push_str(&store.friend_name(id));
            }
            (None, Some(name)) => {
                out.push(':');
                out.push_str(name);
                out.push(':');
            }
            (None, None) => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestButton {
    Ignore,
    Accept,
    NotSpam,
    Report,
}

/// Discord's small request buttons. Each is muted with a tooltip while `REQUEST_ACTIONS` is
/// off; Report has no client support yet, so it always is.
fn request_button(app: &DiscordApp, id: u64, kind: RequestButton) -> AnyElement {
    let (key, text, tip, background, hover) = match kind {
        RequestButton::Ignore => (
            "request-ignore",
            "Ignore",
            IGNORE_FROM_DISCORD,
            rgba(theme::control()),
            rgba(theme::hover()),
        ),
        RequestButton::Accept => (
            "request-accept",
            "Accept DM",
            ACCEPT_FROM_DISCORD,
            rgb(theme::BLURPLE),
            rgb(theme::BLURPLE_ACTIVE),
        ),
        RequestButton::NotSpam => (
            "request-not-spam",
            "Not Spam",
            "Mark requests as not spam from Discord for now",
            rgba(theme::control()),
            rgba(theme::hover()),
        ),
        RequestButton::Report => (
            "request-report",
            "Report Spam",
            "Report spam from Discord for now",
            rgb(theme::NOTIFICATION),
            rgb(theme::NOTIFICATION),
        ),
    };
    let live = REQUEST_ACTIONS && kind != RequestButton::Report;
    let entity = app.entity.clone();
    let button = div()
        .id((key, id))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(rpx(32.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .bg(background)
        .text_size(rpx(14.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(
            if kind == RequestButton::Ignore || kind == RequestButton::NotSpam {
                rgb(theme::bright()).into()
            } else {
                white()
            },
        )
        .child(text)
        .when(live, move |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| match kind {
                            RequestButton::Ignore => app.ignore_request(id, window, cx),
                            // Discord's "not spam" accepts the request.
                            RequestButton::Accept | RequestButton::NotSpam => {
                                app.accept_request(id, window, cx)
                            }
                            RequestButton::Report => {}
                        })
                        .ok();
                })
        });
    if live {
        return button.into_any_element();
    }
    let tip_key = (tip, id);
    div()
        .id((key, id.wrapping_add(1)))
        .relative()
        .flex_none()
        .child(div().opacity(0.5).child(button))
        .child(theme::hover_tooltip(tip_key, tip))
        // Keeps the row's click (open DM) from firing through the button.
        .on_click(|_, _, cx| cx.stop_propagation())
        .into_any_element()
}

/// Up to three server icons and the count, like Discord's "3 Mutual Servers".
fn mutual_servers(app: &DiscordApp, guild_ids: &[u64], cx: &App) -> Div {
    let label = match guild_ids.len() {
        0 => "No Mutual Servers".to_owned(),
        1 => "1 Mutual Server".to_owned(),
        count => format!("{count} Mutual Servers"),
    };
    let shown = guild_ids.len().min(3);
    let size = MUTUAL_ICON + 2.;
    // Absolute positions inside a fixed-width box: the overlap is drawn, not laid out, so the
    // label always starts after the last icon.
    let icons = guild_ids
        .iter()
        .take(shown)
        .enumerate()
        .map(|(index, guild_id)| {
            let image = app
                .store
                .guild_by_id(*guild_id)
                .and_then(|guild| guild.icon_spec())
                .and_then(|spec| images::resolve(&spec, theme::image_px(MUTUAL_ICON), cx));
            div()
                .absolute()
                .top_0()
                .left(rpx(index as f32 * MUTUAL_STEP))
                .size(rpx(size))
                .rounded(rpx(5.))
                .border_1()
                .border_color(rgb(theme::bg()))
                .bg(rgba(theme::selected()))
                .overflow_hidden()
                .children(image.map(|path| img(path).size_full().object_fit(ObjectFit::Cover)))
        });
    let width = (shown > 0).then(|| size + (shown - 1) as f32 * MUTUAL_STEP);
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .children(width.map(|width| {
            div()
                .relative()
                .flex_none()
                .w(rpx(width))
                .h(rpx(size))
                .children(icons)
        }))
        .child(div().min_w_0().truncate().child(label))
}

fn request_row(app: &DiscordApp, dm: &PrivateChannel, now_ms: i64, cx: &App) -> Stateful<Div> {
    let id = dm.channel.id;
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    let user = app.store.user(user_id);
    let name = app.store.private_channel_name(id);
    let username = user
        .map(|user| user.username.clone())
        .filter(|username| !username.is_empty() && *username != name);
    // Discord dates the row by the preview message, falling back to the channel's last message.
    let sent_id = Some(app.store.latest_message_id(id))
        .filter(|id| *id != 0)
        .unwrap_or(0);
    let age = (sent_id != 0).then(|| {
        let sent_ms = (sent_id >> 22).saturating_add(crate::app::DISCORD_EPOCH_MS) as i64;
        request_age(sent_ms, now_ms)
    });
    let hovered = app.home.hovered_row == Some(id);
    let content = app
        .store
        .request_preview(id)
        .map(|preview| request_preview_line(&app.store, &preview.content))
        .filter(|content| !content.trim().is_empty())
        .unwrap_or_else(|| "Sent a message".to_owned());
    let line = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .min_w_0()
            .h(rpx(REQUEST_LINE))
    };
    // Both lines stack in a one-line window; hovering slides the stack up so the latest
    // message replaces the mutual servers, like Discord's row and our account panel.
    let lines = div()
        .absolute()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .child(
            line().children(
                app.home
                    .mutuals
                    .get(&user_id)
                    .map(|guild_ids| mutual_servers(app, guild_ids, cx)),
            ),
        )
        .child(
            line().child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(label(("request-preview", id), &content)),
            ),
        );
    // Only the row being entered or left animates, so rows never slide back on first paint.
    let animate = !cx.reduce_motion() && (hovered || app.home.left_row == Some(id));
    let second_line = if animate {
        lines
            .with_animation(
                // A new key per direction restarts the slide; snowflakes fit in 63 bits.
                ("request-line", (id << 1) | u64::from(hovered)),
                Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                move |this, delta| {
                    let shown = if hovered { delta } else { 1. - delta };
                    this.top(rpx(-REQUEST_LINE * shown))
                },
            )
            .into_any_element()
    } else {
        lines
            .top(rpx(if hovered { -REQUEST_LINE } else { 0. }))
            .into_any_element()
    };
    let entity = app.entity.clone();
    let active = app.thread_pane_shows(id);
    people_row(app, ("request", id).into(), id)
        .ml(rpx(0.))
        .mr(rpx(0.))
        .cursor_pointer()
        // Discord's `.active` row: the one open in the sidebar keeps the hover fill.
        .when(active, |this| this.bg(rgba(theme::hover())))
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.open_thread_pane(id, window, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            REQUEST_AVATAR,
            None,
            false,
            cx,
        ))
        .child(
            // Discord's `.userPreview`: 16px clear of the buttons.
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .mr(rpx(16.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .min_w_0()
                        .child(
                            div()
                                .flex_none()
                                .max_w(relative(0.6))
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(label(("request-name", id), &name)),
                        )
                        .children(username.map(|username| {
                            div()
                                .min_w_0()
                                .ml(rpx(4.))
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .text_color(rgb(theme::muted()))
                                .child(username)
                        }))
                        .children(age.map(|age| {
                            // `.timestampWithPreview`: 8px after the name, body-xs muted.
                            div()
                                .flex_none()
                                .ml(rpx(8.))
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .text_color(rgb(theme::muted()))
                                .child(age)
                        })),
                )
                .child(
                    div()
                        .relative()
                        .min_w_0()
                        .h(rpx(REQUEST_LINE))
                        .mt(rpx(2.))
                        .overflow_hidden()
                        .text_size(rpx(14.))
                        .line_height(rpx(REQUEST_LINE))
                        .text_color(rgb(theme::muted()))
                        .child(second_line),
                ),
        )
        .child(div().flex().flex_none().gap(rpx(16.)).map(|this| {
            if dm.spam {
                this.child(request_button(app, id, RequestButton::NotSpam))
                    .child(request_button(app, id, RequestButton::Report))
            } else {
                this.child(request_button(app, id, RequestButton::Ignore))
                    .child(request_button(app, id, RequestButton::Accept))
            }
        }))
}

/// Replaces the composer while an unaccepted message request is open, in the main view or
/// the side pane.
pub(crate) fn request_bar(app: &DiscordApp, id: u64) -> Option<AnyElement> {
    let dm = app
        .store
        .private_channel(id)
        .filter(|dm| dm.message_request)?;
    let name = app.store.private_channel_name(id);
    let button =
        |key: &'static str, text: &'static str, background: Rgba, hover: Rgba, color: Hsla| {
            div()
                .id(key)
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(32.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .bg(background)
                .when(REQUEST_ACTIONS, move |this| {
                    this.hover(move |style| style.bg(hover)).cursor_pointer()
                })
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(text)
        };
    let accept = {
        let entity = app.entity.clone();
        button(
            "request-bar-accept",
            "Accept",
            rgb(theme::BLURPLE),
            rgb(theme::BLURPLE),
            white(),
        )
        .when(REQUEST_ACTIONS, move |this| {
            this.on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.accept_request(id, window, cx))
                    .ok();
            })
        })
    };
    let ignore = {
        let entity = app.entity.clone();
        button(
            "request-bar-ignore",
            "Ignore",
            rgba(theme::control()),
            rgba(theme::hover()),
            rgb(theme::bright()).into(),
        )
        .when(REQUEST_ACTIONS, move |this| {
            this.on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.ignore_request(id, window, cx))
                    .ok();
            })
        })
    };
    // While request actions are off, each button dims, drops its click, and explains itself.
    let gate = |button: Stateful<Div>, key: &'static str, tip: &'static str| -> AnyElement {
        if REQUEST_ACTIONS {
            return button.into_any_element();
        }
        div()
            .id(key)
            .relative()
            .flex_none()
            .child(div().opacity(0.5).child(button))
            .child(theme::hover_tooltip(key, tip))
            .into_any_element()
    };
    let accept = gate(accept, "request-bar-accept-tip", ACCEPT_FROM_DISCORD);
    let ignore = gate(ignore, "request-bar-ignore-tip", IGNORE_FROM_DISCORD);
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(12.))
            .mx_4()
            .mb(rpx(24.))
            .p(rpx(16.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(format!("{name} wants to message you")),
            )
            .child(
                div()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(if dm.spam {
                        "Discord flagged this as possible spam. Ignoring removes it."
                    } else {
                        "Accept to move this conversation into your Direct Messages."
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(rpx(8.))
                    .child(ignore)
                    .child(accept),
            )
            .into_any_element(),
    )
}

pub(crate) fn blocked_bar(app: &DiscordApp, id: u64) -> Option<AnyElement> {
    let user_id = app
        .store
        .private_channel(id)
        .filter(|dm| !dm.group)?
        .recipient_ids
        .first()
        .copied()?;
    app.store
        .relationship(user_id)
        .filter(|relationship| relationship.kind == RelationshipKind::Blocked)?;
    let entity = app.entity.clone();
    Some(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(12.))
            .mx_4()
            .mb(rpx(24.))
            .px(rpx(16.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .min_w_0()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child("You cannot send messages to a user you have blocked."),
            )
            .child(
                div()
                    .id("blocked-bar-unblock")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(rpx(32.))
                    .px(rpx(12.))
                    .rounded(rpx(8.))
                    .bg(rgba(theme::control()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .when(app.live, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.bg(rgba(theme::hover())))
                            .on_click(move |_, _, cx| {
                                entity
                                    .update(cx, |app, cx| app.unblock_user(user_id, cx))
                                    .ok();
                            })
                    })
                    .when(!app.live, |this| this.opacity(0.5))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::bright()))
                    .without_text_selection()
                    .child("Unblock"),
            )
            .into_any_element(),
    )
}
