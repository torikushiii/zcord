mod active_now;
mod dm_intro;
mod dm_list;
mod people;
mod requests;

pub(crate) use active_now::needs_active_now_clock;
pub(crate) use dm_intro::{dm_header, dm_intro, dm_welcome};
pub(crate) use dm_list::sidebar;
pub(crate) use people::{friends_header, friends_page};
pub(crate) use requests::{blocked_bar, request_bar, requests_header, requests_page};

use super::super::direct::dm_menu;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{
    app::{
        ComposerDraft, DiscordApp,
        activity::quests,
        calls::voice,
        chat,
        direct::friends,
        display_name::{self, Surface},
        members::member_list,
        messages::message_body::label,
        profile::presence_art,
        search, set_text,
    },
    discord::{Error, Event, ProfileKind, model::PublicApplication},
    media::images::{self, ImageSpec},
    store::{
        ActiveVoice, GameActivity, LiveStream, PrivateChannel, RelationshipKind, Status, Store,
        User,
    },
    theme::{self, WithoutTextSelection, rpx},
};

const DM_AVATAR: f32 = 32.;
const AFFINITY_REFRESH: Duration = Duration::from_secs(8 * 60 * 60);

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
    hovered_row: Option<u64>,
    left_row: Option<u64>,
    pub(crate) row_motion: RefCell<people::RowMotion>,
    online_stack: Cell<people::OnlineStack>,
    mutuals: HashMap<u64, Vec<u64>>,
    mutuals_requested: HashSet<u64>,
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
        if self.hover.rail != Some(self.selected_guild_id) {
            self.rail_morphs.start(self.selected_guild_id, false);
        }
        self.member_rows.reset(0);
        self.member_layout = member_list::Layout::default();
        self.member_top = 0;
        self.dismiss_stream_card();
        self.selected_guild_id = 0;
    }

    pub(crate) fn open_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.favorites_view = false;
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
        let draft = self.compose.input.read(cx).value().to_string();
        if draft.is_empty() {
            self.compose.drafts.remove(&previous);
        } else {
            self.compose.drafts.insert(
                previous,
                ComposerDraft {
                    content: draft,
                    mentions: self.suggest.mention_names.clone(),
                },
            );
        }
        self.compose
            .input
            .update(cx, |input, cx| set_text(input, String::new(), window, cx));
        self.profile = None;
        self.compose.reply = None;
        self.compose.editing = None;
        self.compose.confirm_delete = None;
        self.stop_player(Some(window), cx);
        self.close_lightbox(cx);
        self.close_reactions_dialog(cx);
        self.dismiss_reaction_tooltip();
        self.selected_channel_id = 0;
        self.reset_mention_draft();
        self.unread.since = None;
        self.unread.bar = false;
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

    pub(crate) fn load_user_affinities(&mut self, cx: &mut Context<Self>) {
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
