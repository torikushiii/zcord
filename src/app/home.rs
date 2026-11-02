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
        DiscordApp, chat,
        display_name::{self, Surface},
        friends, member_list,
        message_body::label,
        profile::presence_art,
        search, set_text, voice,
    },
    discord::{Event, model::PublicApplication},
    media::images::{self, ImageSpec},
    store::{GameActivity, LiveStream, PrivateChannel, RelationshipKind, Status, Store, User},
    theme::{self, WithoutTextSelection, rpx},
};

const NAV_HEIGHT: f32 = 42.;
const DM_HEIGHT: f32 = 42.;
const DM_AVATAR: f32 = 32.;
const FRIEND_HEIGHT: f32 = 62.;
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
    Add,
}

impl FriendsTab {
    fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::All => "All",
            Self::Pending => "Pending",
            Self::Add => "Add Friend",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HomePage {
    #[default]
    Friends,
    Requests,
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
    requests_tab: RequestsTab,
    previews_requested: HashSet<u64>,
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
    }

    fn open_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_home_page(HomePage::Requests, window, cx);
        self.load_request_previews(cx);
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
            self.drafts.insert(previous, draft);
        }
        self.composer
            .update(cx, |input, cx| set_text(input, String::new(), window, cx));
        self.profile = None;
        self.reply = None;
        self.editing = None;
        self.confirm_delete = None;
        self.stop_player(Some(window), cx);
        self.close_lightbox(cx);
        self.selected_channel_id = 0;
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

    fn load_request_previews(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let ids: Vec<u64> = self
            .store
            .message_requests()
            .into_iter()
            .chain(self.store.spam_requests())
            .map(|dm| dm.channel.id)
            .filter(|id| self.home.previews_requested.insert(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let request = Tokio::spawn(
            cx,
            async move { session.message_request_previews(ids).await },
        );
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
        let request = Tokio::spawn(cx, async move { session.public_applications(&ids).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(applications)) => {
                    for application in applications {
                        app.home.applications.insert(application.id, application);
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }
                Ok(Err(error)) => log::warn!("Could not load game applications: {error}"),
                Err(error) => log::warn!("Game applications task failed: {error}"),
            })
            .ok();
        })
        .detach();
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
        cx.notify();
    }

    fn select_friends_tab(&mut self, tab: FriendsTab, cx: &mut Context<Self>) {
        self.home.tab = tab;
        cx.notify();
    }

    pub(crate) fn select_dm(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_home();
        self.select_channel(channel_id, window, cx);
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
        .bg(rgba(theme::HOVER))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::MUTED))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::ACTIVE)).text_color(rgb(theme::TEXT)))
        .child("Find or start a conversation")
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.toggle_quick_switcher(window, cx))
                .ok();
        });
    let friends_selected = app.showing_home_page() && app.home.page == HomePage::Friends;
    let requests_selected = app.showing_home_page() && app.home.page == HomePage::Requests;
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
        this.hover(|style| style.bg(rgba(theme::HOVER)).text_color(rgb(theme::TEXT)))
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
                .border_color(rgba(theme::BORDER))
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
                            style.bg(rgba(theme::HOVER)).text_color(rgb(theme::TEXT))
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
                        .bg(rgba(theme::BORDER)),
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
                        .text_color(rgb(theme::MUTED))
                        .child("Direct Messages")
                        .child(theme::icon("plus", 16., theme::MUTED)),
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
        theme::STRONG
    } else {
        theme::MUTED
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
        .when(selected, |this| this.bg(rgba(theme::SELECTED)))
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
            .child(theme::icon("users", 18., theme::STRONG))
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
        theme::STRONG
    } else {
        theme::MUTED
    };
    let ring = if selected {
        theme::composite(theme::SELECTED, theme::BG)
    } else {
        theme::BG
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
            .text_color(rgb(theme::MUTED))
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
        member_list::nameplate_layer(user.nameplate.as_ref()?, user_id, DM_HEIGHT, hovered, cx)
    });
    let plated = nameplate.is_some();
    let dim = rgba(
        if selected {
            ring
        } else if hovered {
            theme::composite(theme::HOVER, theme::BG)
        } else {
            theme::BG
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
        .when(selected, |this| this.bg(rgba(theme::SELECTED)))
        .when(!selected, |this| {
            this.hover(|style| style.bg(rgba(theme::HOVER)).text_color(rgb(theme::TEXT)))
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
        .children(nameplate)
        .when(plated, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(rpx(8. + DM_AVATAR + 12.))
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
        .when(dm.channel.mentions > 0, |this| {
            this.child(count_pill(dm.channel.mentions))
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
                    .bg(rgb(theme::STRONG)),
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
            .gap(rpx(6.))
            .h(rpx(28.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .text_color(rgb(if selected {
                theme::STRONG
            } else {
                theme::MUTED
            }))
            .when(selected, |this| this.bg(rgba(theme::SELECTED)))
            .when(!selected, |this| {
                this.hover(|style| style.bg(rgba(theme::HOVER)).text_color(rgb(theme::TEXT)))
            })
            .child(tab.label())
            .when(badge > 0, |this| this.child(count_pill(badge)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_friends_tab(tab, cx))
                    .ok();
            })
    };
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
        .when(adding, |this| this.bg(rgba(theme::SELECTED)))
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
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("users", 20., theme::FAINT))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child("Friends"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::FAINT)),
        )
        .child(tab(FriendsTab::Online, 0))
        .child(tab(FriendsTab::All, 0))
        .child(tab(FriendsTab::Pending, app.store.pending_count()))
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
            FriendsTab::Add => "",
        },
        ids.len()
    );
    let empty = ids.is_empty();
    let rows: Vec<AnyElement> = ids
        .into_iter()
        .map(|id| friend_row(app, id, cx).into_any_element())
        .collect();
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
                .pt_4()
                .children(app.home.search.as_ref().map(|search| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .mx_6()
                        .h(rpx(36.))
                        .px(rpx(10.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::ELEVATED))
                        .border_1()
                        .border_color(rgba(theme::BORDER))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(search)
                                    .appearance(false)
                                    .px_0()
                                    .text_size(rpx(16.)),
                            ),
                        )
                        .child(theme::icon("search", 18., theme::MUTED))
                }))
                .child(
                    div()
                        .flex_none()
                        .mx_6()
                        .mt(rpx(20.))
                        .mb(rpx(8.))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::MUTED))
                        .child(heading),
                )
                .children(
                    (tab == FriendsTab::Pending)
                        .then(|| app.pending_error())
                        .flatten()
                        .map(|text| {
                            div()
                                .flex_none()
                                .mx_6()
                                .mb(rpx(8.))
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
                        .px(rpx(14.))
                        .pb_4()
                        .children(rows)
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .mt(rpx(40.))
                                    .text_center()
                                    .text_size(rpx(14.))
                                    .text_color(rgb(theme::MUTED))
                                    .child(match tab {
                                        FriendsTab::Pending => {
                                            "There are no pending friend requests."
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

fn round_action(id: impl Into<ElementId>, icon: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(ACTION_SIZE))
        .rounded_full()
        .bg(rgb(theme::ELEVATED))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::SELECTED)))
        .child(theme::icon(icon, 18., theme::MUTED))
}

fn friend_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let kind = app
        .store
        .relationship(user_id)
        .map_or(RelationshipKind::Friend, |relationship| relationship.kind);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| user.username.clone()).unwrap_or_default();
    let status = user.map_or(Status::Offline, |user| user.status);
    let icon = user
        .filter(|user| user.status.is_online())
        .and_then(theme::activity_icon);
    let line = status_text(user, kind);
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx));
    let entity = app.entity.clone();
    let entity_for_accept = app.entity.clone();
    let entity_for_drop = app.entity.clone();
    let pending = matches!(
        kind,
        RelationshipKind::Incoming | RelationshipKind::Outgoing
    );
    div()
        .id(("friend", user_id))
        .group("friend-row")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(FRIEND_HEIGHT))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .border_t_1()
        .border_color(rgba(theme::BORDER))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::HOVER)))
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.open_dm(user_id, window, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            DM_AVATAR,
            Some((status, theme::BG)),
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
                                .text_color(rgb(theme::STRONG))
                                .child(label(("friend-name", user_id), &name)),
                        )
                        .children(tag)
                        .child(
                            div()
                                .invisible()
                                .group_hover("friend-row", |style| style.visible())
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(14.))
                                .text_color(rgb(theme::MUTED))
                                .child(username),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .min_w_0()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .children(icon.map(|icon| theme::icon(icon, 14., theme::GREEN)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(label(("friend-status", user_id), &line)),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .when(kind == RelationshipKind::Incoming, |this| {
                    let accept = entity_for_accept.clone();
                    this.child(
                        round_action(("friend-accept", user_id), "check")
                            .tooltip(|_, cx| theme::text_tooltip("Accept", cx))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                accept
                                    .update(cx, |app, cx| app.add_friend(user_id, cx))
                                    .ok();
                            }),
                    )
                })
                .when(pending, |this| {
                    let tip = if kind == RelationshipKind::Incoming {
                        "Ignore"
                    } else {
                        "Cancel"
                    };
                    this.child(
                        round_action(("friend-drop", user_id), "x")
                            .tooltip(move |_, cx| theme::text_tooltip(tip, cx))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                entity_for_drop
                                    .update(cx, |app, cx| app.drop_friend_request(user_id, cx))
                                    .ok();
                            }),
                    )
                })
                .when(!pending, |this| {
                    this.child(round_action(("friend-message", user_id), "message-circle"))
                }),
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
        .bg(rgba(theme::SELECTED))
        .child(image)
        .children(badge.map(|badge| {
            div()
                .absolute()
                .right(rpx(-4.))
                .bottom(rpx(-4.))
                .size(rpx(ACTIVE_BADGE + 4.))
                .p(rpx(2.))
                .rounded_full()
                .bg(rgb(theme::BG))
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
                .text_color(rgb(theme::STRONG))
                .child(label(("active-detail", id), detail)),
        )
        .children(state.map(|state| {
            div()
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::MUTED))
                .child(label(("active-state", id), state))
        }))
        .children(duration.map(|duration| {
            div()
                .debug_selector(|| "active-elapsed".into())
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::MUTED))
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
        .bg(rgb(theme::ELEVATED))
        .group_hover(ACTIVE_CARD_GROUP, |style| {
            style.bg(rgb(theme::composite(theme::HOVER, theme::ELEVATED)))
        })
        .child(theme::user_avatar(
            Some(user),
            id,
            DM_AVATAR,
            Some((user.status, theme::ELEVATED)),
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
                        .text_color(rgb(theme::STRONG))
                        .child(label(("active-name", id), &app.store.friend_name(id))),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
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
        .border_color(rgba(theme::BORDER))
        .overflow_hidden()
        .hover(|style| style.bg(rgba(theme::HOVER)))
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
        _ => ("monitor-up", theme::MUTED),
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
            .bg(rgba(theme::SELECTED))
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
            .child(theme::icon("gamepad-2", size * 0.55, theme::MUTED))
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
                .child(theme::user_avatar(
                    Some(user),
                    id,
                    ACTIVE_PARTY_AVATAR,
                    None,
                    false,
                    cx,
                )),
        })
        .into_any_element()
}

fn active_now(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let cards: Vec<AnyElement> = app
        .store
        .active_now()
        .into_iter()
        .filter_map(|id| {
            let user = app.store.user(id)?;
            if let Some(stream) = &user.live_stream {
                return Some(live_stream_card(app, id, user, stream, cx));
            }
            let activity = user.game.as_ref().or(user.listening.as_ref())?;
            Some(active_card(app, id, user, activity, cx))
        })
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
        .border_color(rgba(theme::BORDER))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
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
                            .text_color(rgb(theme::STRONG))
                            .child("It's quiet for now..."),
                    )
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::MUTED))
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
        .border_color(rgba(theme::BORDER))
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
                .text_color(rgb(theme::STRONG))
                .child(label(("header-dm", channel_id), &name)),
        )
        .children(game.map(|game| {
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .min_w_0()
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child(theme::icon("gamepad-2", 14., theme::GREEN))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label(("header-game", channel_id), &game)),
                )
        }))
        .child(div().flex_1())
        .child(chat::pins_button(app))
        .child(
            theme::icon_button(
                "dm-members",
                if group { "users" } else { "user" },
                20.,
                if app.show_members {
                    theme::STRONG
                } else {
                    theme::MUTED
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
            .child(theme::icon("users", 14., theme::STRONG))
            .into_any_element();
    }
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    theme::user_avatar(app.store.user(user_id), user_id, 24., None, false, cx).into_any_element()
}

pub(crate) fn dm_welcome(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let channel_id = app.selected_channel_id;
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
                    .text_color(rgb(theme::STRONG))
                    .child(label(("welcome-dm", channel_id), &name)),
            )
            .children(user.map(|user| {
                div()
                    .text_size(rpx(20.))
                    .text_color(rgb(theme::TEXT))
                    .child(user.username.clone())
            }))
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(16.))
                    .text_color(rgb(theme::MUTED))
                    .child(text),
            )
            .into_any_element(),
    )
}

fn header_tab(id: &'static str, text: &'static str, selected: bool, badge: u32) -> Stateful<Div> {
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
            theme::STRONG
        } else {
            theme::MUTED
        }))
        .when(selected, |this| this.bg(rgba(theme::SELECTED)))
        .when(!selected, |this| {
            this.hover(|style| style.bg(rgba(theme::HOVER)).text_color(rgb(theme::TEXT)))
        })
        .child(text)
        .when(badge > 0, |this| this.child(count_pill(badge)))
}

pub(crate) fn requests_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |id: &'static str, tab: RequestsTab, text: &'static str, badge: u32| {
        let entity = app.entity.clone();
        header_tab(id, text, app.home.requests_tab == tab, badge).on_click(move |_, _, cx| {
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
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("mail", 20., theme::FAINT))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child("Message Requests"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::FAINT)),
        )
        .child(tab(
            "requests-tab",
            RequestsTab::Requests,
            "Requests",
            app.store.message_request_count(),
        ))
        .child(tab("spam-tab", RequestsTab::Spam, "Spam", 0))
}

pub(crate) fn requests_page(app: &DiscordApp, cx: &App) -> AnyElement {
    let spam = app.home.requests_tab == RequestsTab::Spam;
    let requests = if spam {
        app.store.spam_requests()
    } else {
        app.store.message_requests()
    };
    let heading = format!(
        "{} — {}",
        if spam { "Spam" } else { "Requests" },
        requests.len()
    );
    let empty = requests.is_empty();
    let rows: Vec<AnyElement> = requests
        .into_iter()
        .map(|dm| request_row(app, dm, cx).into_any_element())
        .collect();
    div()
        .flex()
        .flex_col()
        .size_full()
        .pt_4()
        .child(
            div()
                .flex_none()
                .mx_6()
                .mb(rpx(8.))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::MUTED))
                .child(heading),
        )
        .child(
            div()
                .id("requests-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px(rpx(14.))
                .pb_4()
                .children(rows)
                .when(empty, |this| {
                    this.child(
                        div()
                            .mt(rpx(40.))
                            .text_center()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::MUTED))
                            .child(if spam {
                                "There are no spam requests."
                            } else {
                                "There are no pending message requests."
                            }),
                    )
                }),
        )
        .into_any_element()
}

fn request_row(app: &DiscordApp, dm: &PrivateChannel, cx: &App) -> impl IntoElement + use<> {
    let id = dm.channel.id;
    let name = app.store.private_channel_name(id);
    let preview = app.store.request_preview(id);
    let time = preview.map(|preview| preview.timestamp.full());
    let content = preview
        .map(|preview| preview.content.clone())
        .filter(|content| !content.is_empty())
        .unwrap_or_else(|| "Sent a message".to_owned());
    let entity = app.entity.clone();
    let accept = {
        let entity = app.entity.clone();
        round_action(("request-accept", id), "check").on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.accept_request(id, window, cx))
                .ok();
        })
    };
    let ignore = {
        let entity = app.entity.clone();
        round_action(("request-ignore", id), "x").on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.ignore_request(id, window, cx))
                .ok();
        })
    };
    div()
        .id(("request", id))
        .group("request-row")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(FRIEND_HEIGHT))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .border_t_1()
        .border_color(rgba(theme::BORDER))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::HOVER)))
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.select_dm(id, window, cx))
                .ok();
        })
        .child(dm_avatar(app, dm, theme::BG, false, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(rpx(8.))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(label(("request-name", id), &name)),
                        )
                        .children(time.map(|time| {
                            div()
                                .flex_none()
                                .text_size(rpx(12.))
                                .text_color(rgb(theme::FAINT))
                                .child(time)
                        })),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .child(label(("request-preview", id), &content)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .child(ignore)
                .when(!dm.spam, |this| this.child(accept)),
        )
}

/// Replaces the composer while an unaccepted message request is open.
pub(crate) fn request_bar(app: &DiscordApp) -> Option<AnyElement> {
    let id = app.selected_channel_id;
    let dm = app
        .store
        .private_channel(id)
        .filter(|dm| app.is_home() && dm.message_request)?;
    let name = app.store.private_channel_name(id);
    let button = |key: &'static str, text: &'static str, background: Rgba, hover: Rgba| {
        div()
            .id(key)
            .flex()
            .items_center()
            .justify_center()
            .h(rpx(32.))
            .px(rpx(16.))
            .rounded(rpx(8.))
            .bg(background)
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white())
            .child(text)
    };
    let accept = {
        let entity = app.entity.clone();
        button(
            "request-bar-accept",
            "Accept",
            rgb(theme::BLURPLE),
            rgb(theme::BLURPLE),
        )
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.accept_request(id, window, cx))
                .ok();
        })
    };
    let ignore = {
        let entity = app.entity.clone();
        button(
            "request-bar-ignore",
            "Ignore",
            rgba(theme::CONTROL),
            rgba(theme::HOVER),
        )
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.ignore_request(id, window, cx))
                .ok();
        })
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(12.))
            .mx_4()
            .mb(rpx(24.))
            .p(rpx(16.))
            .rounded(rpx(8.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .child(
                div()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::STRONG))
                    .child(format!("{name} wants to message you")),
            )
            .child(
                div()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::MUTED))
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
