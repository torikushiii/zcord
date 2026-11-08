use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::ElementExt as _;

use crate::{
    app::{
        DiscordApp, calls::voice, chat, direct::dm_panel, direct::home, expressions::emoji_picker,
        expressions::expression_info, expressions::gif_picker, forward_dialog,
        guilds::channel_list, guilds::guild_rail, lightbox, members::member_list, profile, search,
        settings_page, switcher,
    },
    discord::settings_proto::TextImages,
    theme::rpx,
};

pub(crate) struct PanelViews {
    pub(crate) guild_rail: Entity<GuildRailView>,
    pub(crate) channel_list: Entity<ChannelListView>,
    pub(crate) chat_header: Entity<ChatHeaderView>,
    pub(crate) messages: Entity<MessagesView>,
    pub(crate) composer: Entity<ComposerView>,
    /// The composer's measured height, readable while the composer itself renders, which
    /// reading the view then would not allow.
    pub(crate) composer_height: Rc<Cell<Option<Pixels>>>,
    pub(crate) members: Entity<MemberListView>,
    pub(crate) thread_pane: Entity<ThreadPaneView>,
    pub(crate) overlays: Entity<OverlaysView>,
    pub(crate) user_panel: Entity<UserPanelView>,
    pub(crate) user_panel_height: Rc<Cell<Option<Pixels>>>,
}

impl PanelViews {
    pub(crate) fn new(app: &Entity<DiscordApp>, cx: &mut Context<DiscordApp>) -> Self {
        let composer_height = Rc::new(Cell::new(None));
        let channel_list = cx.new(|cx| ChannelListView::new(app, cx));
        Self {
            guild_rail: cx.new(|cx| GuildRailView::new(app, cx)),
            user_panel: cx.new(|cx| UserPanelView::new(app, &channel_list, cx)),
            user_panel_height: Rc::new(Cell::new(None)),
            channel_list,
            chat_header: cx.new(|cx| ChatHeaderView::new(app, cx)),
            messages: cx.new(|cx| MessagesView::new(app, cx)),
            composer: cx.new(|cx| ComposerView::new(app, composer_height.clone(), cx)),
            composer_height,
            members: cx.new(|cx| MemberListView::new(app, cx)),
            thread_pane: cx.new(|cx| ThreadPaneView::new(app, cx)),
            overlays: cx.new(|cx| OverlaysView::new(app, cx)),
        }
    }
}

pub(crate) fn guild_rail_style() -> StyleRefinement {
    StyleRefinement::default()
        .w(rpx(guild_rail::WIDTH))
        .h_full()
        .flex_none()
}

pub(crate) fn channel_list_style() -> StyleRefinement {
    StyleRefinement::default()
        .w(rpx(channel_list::WIDTH))
        .h_full()
        .flex_none()
}

pub(crate) fn chat_header_style() -> StyleRefinement {
    StyleRefinement::default()
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
}

pub(crate) fn messages_style() -> StyleRefinement {
    StyleRefinement::default()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .w_full()
}

pub(crate) fn member_list_style(width: f32) -> StyleRefinement {
    StyleRefinement::default()
        .w(rpx(width))
        .bg(crate::theme::surface(
            crate::theme::Layer::Low,
            Some(crate::theme::Layer::Chat),
            crate::theme::base_lower(),
        ))
        .h_full()
        .flex_none()
}

pub(crate) fn thread_pane_style(forum: bool, dm: bool) -> StyleRefinement {
    let style = StyleRefinement::default().h_full();
    if forum {
        style.flex_1().min_w_0()
    } else if dm {
        style
            .w(relative(0.37))
            .min_w(rpx(chat::THREAD_PANE_WIDTH))
            .flex_none()
    } else {
        style.w(rpx(chat::THREAD_PANE_WIDTH)).flex_none()
    }
}

pub(crate) fn overlays_style() -> StyleRefinement {
    StyleRefinement::default().absolute().inset_0()
}

fn overlay_root() -> Div {
    // Cached views lay their contents out as a separate root, without a positioned parent.
    div().relative().size_full()
}

pub(crate) fn composer_style(height: Option<Pixels>, hidden: bool) -> StyleRefinement {
    let style = StyleRefinement::default().w_full().flex_none();
    match (hidden, height) {
        (true, _) => style.h(px(0.)),
        (false, Some(height)) => style.h(height),
        (false, None) => style.h(rpx(64.)),
    }
}

fn observe_app<T: 'static>(app: &Entity<DiscordApp>, cx: &mut Context<T>) -> Subscription {
    cx.observe(app, |_, _, cx| cx.notify())
}

pub(crate) struct GuildRailView {
    app: WeakEntity<DiscordApp>,
    dms: guild_rail::RailDms,
    dm_settle: Option<Task<()>>,
    _observe: Subscription,
}

impl GuildRailView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            dms: guild_rail::RailDms::default(),
            dm_settle: None,
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for GuildRailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::GuildRail);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let current = guild_rail::rail_dm_ids(app.read(cx));
        if self.dms.update(current, cx.reduce_motion()) {
            self.dm_settle = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(guild_rail::DM_SLIDE).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }));
        }
        guild_rail::render(app.read(cx), &self.dms, cx).into_any_element()
    }
}

pub(crate) struct ChannelListView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl ChannelListView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for ChannelListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::ChannelList);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        channel_list::render(app.read(cx), window, cx).into_any_element()
    }
}

/// Account and voice state changes often notify only the channel list, so this view observes it.
pub(crate) struct UserPanelView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
    _observe_list: Subscription,
}

impl UserPanelView {
    fn new(
        app: &Entity<DiscordApp>,
        channel_list: &Entity<ChannelListView>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
            _observe_list: cx.observe(channel_list, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for UserPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        channel_list::user_panel(app.read(cx), cx).into_any_element()
    }
}

pub(crate) fn user_panel_overlay(views: &PanelViews) -> impl IntoElement + use<> {
    let measured = views.user_panel_height.clone();
    let sidebars = [views.guild_rail.entity_id(), views.channel_list.entity_id()];
    div()
        .absolute()
        .left_0()
        .bottom_0()
        .w(rpx(guild_rail::WIDTH + channel_list::WIDTH))
        .child(views.user_panel.clone())
        .on_prepaint(move |bounds, window, _| {
            let height = bounds.size.height;
            let changed = measured
                .get()
                .is_none_or(|old| (f32::from(old) - f32::from(height)).abs() > 0.5);
            if changed {
                measured.set(Some(height));
                window.on_next_frame(move |_, cx| {
                    for view in sidebars {
                        cx.notify(view);
                    }
                });
            }
        })
}

pub(crate) fn user_panel_reserve(app: &DiscordApp) -> Option<Length> {
    if !crate::settings::user_panel_extended() {
        return None;
    }
    Some(match app.views.user_panel_height.get() {
        Some(height) => height.into(),
        None => rpx(channel_list::USER_PANEL_HEIGHT + 2. + 16.).into(),
    })
}

pub(crate) struct ChatHeaderView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl ChatHeaderView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for ChatHeaderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Header);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app.read(cx);
        if app.showing_home_page() {
            match app.home.page {
                home::HomePage::Friends => home::friends_header(app).into_any_element(),
                home::HomePage::Requests => home::requests_header(app).into_any_element(),
                home::HomePage::Quests => super::activity::quests::header(app).into_any_element(),
            }
        } else if app.is_home() {
            home::dm_header(app, cx).into_any_element()
        } else {
            chat::header(app, cx).into_any_element()
        }
    }
}

type LayoutPreferences = (f32, TextImages, u32, [bool; 3]);

fn layout_preferences() -> LayoutPreferences {
    let display = crate::settings::display();
    let spacing = crate::settings::message_group_spacing(display.message_display_compact);
    (
        crate::settings::chat_font_size(),
        display,
        spacing,
        [
            crate::settings::display_name_styles(),
            crate::settings::mention_avatars(),
            crate::settings::who_reacted(),
        ],
    )
}

pub(crate) struct MessagesView {
    app: WeakEntity<DiscordApp>,
    layout_preferences: LayoutPreferences,
    quest_cards: super::activity::quests::QuestCards,
    active_now_clock: Option<Task<()>>,
    _observe: Subscription,
}

impl MessagesView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            layout_preferences: layout_preferences(),
            quest_cards: Default::default(),
            active_now_clock: None,
            _observe: observe_app(app, cx),
        }
    }

    fn sync_active_now_clock(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !enabled {
            self.active_now_clock = None;
        } else if self.active_now_clock.is_none() {
            self.active_now_clock = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
    }
}

impl Render for MessagesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Messages);
        let Some(app) = self.app.upgrade() else {
            self.sync_active_now_clock(false, cx);
            return div().into_any_element();
        };
        self.sync_active_now_clock(home::needs_active_now_clock(app.read(cx)), cx);
        let quests = {
            let app = app.read(cx);
            app.showing_home_page() && matches!(app.home.page, home::HomePage::Quests)
        };
        if quests {
            self.quest_cards.sync(&app, cx);
        }
        let app = app.read(cx);
        let preferences = layout_preferences();
        if self.layout_preferences != preferences {
            app.messages.remeasure();
            if let Some(pane) = &app.thread_pane {
                pane.remeasure();
            }
            self.layout_preferences = preferences;
        }
        if app.showing_home_page() {
            return match app.home.page {
                home::HomePage::Friends => home::friends_page(app, cx),
                home::HomePage::Requests => home::requests_page(app, cx),
                home::HomePage::Quests => {
                    super::activity::quests::page(app, &self.quest_cards, window, cx)
                }
            };
        }
        chat::messages(app, cx)
    }
}

pub(crate) struct ComposerView {
    app: WeakEntity<DiscordApp>,
    height: Rc<Cell<Option<Pixels>>>,
    _observe: Subscription,
}

impl ComposerView {
    fn new(
        app: &Entity<DiscordApp>,
        height: Rc<Cell<Option<Pixels>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            app: app.downgrade(),
            height,
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for ComposerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Composer);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app.read(cx);
        if app.selected_forum().is_some() || app.showing_home_page() {
            return div().into_any_element();
        }
        let measured = self.height.clone();
        let view = cx.entity().entity_id();
        div()
            .w_full()
            .child(
                match home::request_bar(app, app.selected_channel_id)
                    .or_else(|| home::blocked_bar(app, app.selected_channel_id))
                    .filter(|_| app.is_home())
                {
                    Some(bar) => bar,
                    None => chat::composer(app, cx).into_any_element(),
                },
            )
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        let height = bounds.size.height;
                        // Sub-pixel churn between the style height and the measured
                        // content must not schedule a frame every frame.
                        let changed = measured
                            .get()
                            .is_none_or(|old| (f32::from(old) - f32::from(height)).abs() > 0.5);
                        if changed {
                            measured.set(Some(height));
                            // The parent supplies next frame's cached height; ask for
                            // that frame even though this one is already drawing.
                            window.on_next_frame(move |_, cx| cx.notify(view));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
}

pub(crate) struct ThreadPaneView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl ThreadPaneView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for ThreadPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        chat::render_thread_pane(app.read(cx), cx)
    }
}

pub(crate) struct MemberListView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl MemberListView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for MemberListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Members);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app.read(cx);
        if let Some(results) = search::render_results(app, cx) {
            return results;
        }
        if app.is_home() {
            return dm_panel::render(app, cx);
        }
        member_list::render(app).into_any_element()
    }
}

pub(crate) struct OverlaysView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl OverlaysView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for OverlaysView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Overlays);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        if app.read(cx).stage_chat_orphaned() {
            let app = app.clone();
            window.defer(cx, move |window, cx| {
                app.update(cx, |app, cx| app.close_stream_chat(window, cx));
            });
        }
        let app = app.read(cx);
        overlay_root()
            .children(super::calls::call_view::render(app, window, cx))
            .children(super::calls::call_view::render_lobby(app))
            .children(super::calls::stream_view::render(app, cx))
            .children(voice::self_preview::render(app, window, cx))
            .children(settings_page::render(app, window, cx))
            .children(chat::channel_popout(app, window, cx))
            .children(super::inbox::inbox_popout(app, window, cx))
            .children(super::messages::saved::custom_reminder_dialog(app))
            .children(super::inbox::schedule_edit_dialog(app))
            .children(search::popover(app, cx))
            .children(chat::create_thread_dialog(app))
            .children(forward_dialog::render(app))
            .children(super::expressions::reactions_dialog::render(app, cx))
            .children(chat::thread_menu(app))
            .children(channel_list::sharing_paused_card(app, window))
            .children(chat::delete_dialog(app, cx))
            .children(profile::render(app, window, cx))
            .children(super::direct::friends::remove_dialog(app))
            .children(expression_info::render(app, window, cx))
            .children(lightbox::render(app, window, cx))
            .children(emoji_picker::render(app, window, cx))
            .children(gif_picker::render(app, window, cx))
            .children(super::expressions::sticker_picker::render(app, window, cx))
            .children(voice::menu(app, cx))
            .children(chat::message_menu(app, cx))
            .children(chat::composer_menu(app, cx))
            .children(super::direct::dm_menu::menu(app))
            .children(super::direct::dm_menu::dialog(app, cx))
            .children(super::guilds::guild_menu::menu(app))
            .children(super::members::member_menu::menu(app))
            .children(super::direct::dm_menu::incoming_call(app, cx))
            .children(member_list::activity::popout(app, window, cx))
            .children(voice::stream_card::render(app, cx))
            .children(voice::share_picker::render(app))
            .children(switcher::render(app, cx))
            .children(crate::media::player::fullscreen(app, cx))
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ViewKind {
    Root,
    GuildRail,
    ChannelList,
    Header,
    Messages,
    Composer,
    Members,
    Overlays,
}

#[derive(Default)]
struct RenderCounts {
    last: Option<Instant>,
    root: u64,
    guild_rail: u64,
    channel_list: u64,
    header: u64,
    messages: u64,
    composer: u64,
    members: u64,
    overlays: u64,
}

thread_local! {
    static RENDER_COUNTS: RefCell<RenderCounts> = RefCell::new(RenderCounts::default());
}

pub(crate) fn count_render(kind: ViewKind) {
    if !log::log_enabled!(target: "zcord::perf", log::Level::Debug) {
        return;
    }
    RENDER_COUNTS.with_borrow_mut(|counts| {
        match kind {
            ViewKind::Root => counts.root += 1,
            ViewKind::GuildRail => counts.guild_rail += 1,
            ViewKind::ChannelList => counts.channel_list += 1,
            ViewKind::Header => counts.header += 1,
            ViewKind::Messages => counts.messages += 1,
            ViewKind::Composer => counts.composer += 1,
            ViewKind::Members => counts.members += 1,
            ViewKind::Overlays => counts.overlays += 1,
        }
        let now = Instant::now();
        let due = counts
            .last
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1));
        if !due {
            return;
        }
        log::debug!(
            target: "zcord::perf",
            "renders/s root={} rail={} channels={} header={} messages={} composer={} members={} overlays={}",
            counts.root,
            counts.guild_rail,
            counts.channel_list,
            counts.header,
            counts.messages,
            counts.composer,
            counts.members,
            counts.overlays,
        );
        *counts = RenderCounts {
            last: Some(now),
            ..RenderCounts::default()
        };
    });
}

#[cfg(test)]
mod tests;
