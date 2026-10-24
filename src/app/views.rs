//! Cached child views for the chat shell.
//!
//! `DiscordApp` only builds the layout skeleton and the overlays; each hot panel
//! is a separate view entity so a notify from one panel does not dirty the rest.
//! Every view observes `DiscordApp` with a strong entity handle but keeps only a
//! weak handle for rendering, so the ownership graph is acyclic.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};

use crate::{
    app::{
        DiscordApp, channel_list, chat, emoji_picker, guild_rail, lightbox, member_list, profile,
    },
    theme::rpx,
};

pub(crate) struct PanelViews {
    pub(crate) guild_rail: Entity<GuildRailView>,
    pub(crate) channel_list: Entity<ChannelListView>,
    pub(crate) chat_header: Entity<ChatHeaderView>,
    pub(crate) messages: Entity<MessagesView>,
    pub(crate) composer: Entity<ComposerView>,
    pub(crate) members: Entity<MemberListView>,
    pub(crate) overlays: Entity<OverlaysView>,
}

impl PanelViews {
    pub(crate) fn new(app: &Entity<DiscordApp>, cx: &mut Context<DiscordApp>) -> Self {
        Self {
            guild_rail: cx.new(|cx| GuildRailView::new(app, cx)),
            channel_list: cx.new(|cx| ChannelListView::new(app, cx)),
            chat_header: cx.new(|cx| ChatHeaderView::new(app, cx)),
            messages: cx.new(|cx| MessagesView::new(app, cx)),
            composer: cx.new(|cx| ComposerView::new(app, cx)),
            members: cx.new(|cx| MemberListView::new(app, cx)),
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

pub(crate) fn member_list_style() -> StyleRefinement {
    StyleRefinement::default()
        .w(rpx(member_list::WIDTH))
        .h_full()
        .flex_none()
}

fn observe_app<T: 'static>(app: &Entity<DiscordApp>, cx: &mut Context<T>) -> Subscription {
    cx.observe(app, |_, _, cx| cx.notify())
}

pub(crate) struct GuildRailView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl GuildRailView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
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
        guild_rail::render(app.read(cx), cx).into_any_element()
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
        chat::header(app.read(cx)).into_any_element()
    }
}

pub(crate) struct MessagesView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl MessagesView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            _observe: observe_app(app, cx),
        }
    }
}

impl Render for MessagesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Messages);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        chat::messages(app.read(cx)).into_any_element()
    }
}

pub(crate) struct ComposerView {
    app: WeakEntity<DiscordApp>,
    _observe: Subscription,
}

impl ComposerView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
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
        chat::composer(app.read(cx), cx).into_any_element()
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
        member_list::render(app.read(cx)).into_any_element()
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
        let app = app.read(cx);
        div()
            .absolute()
            .inset_0()
            .children(chat::delete_dialog(app, cx))
            .children(profile::render(app, window, cx))
            .children(lightbox::render(app, window, cx))
            .children(emoji_picker::render(app, cx))
            .children(crate::player::fullscreen(app, cx))
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
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        AppContext, Context, Entity, Render, StyleRefinement, Subscription, TestAppContext,
        WeakEntity, Window, div, prelude::*,
    };

    struct Panel {
        _host: WeakEntity<Host>,
        renders: Rc<Cell<usize>>,
        _observe: Subscription,
    }

    impl Panel {
        fn new(host: Entity<Host>, renders: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
            Self {
                _observe: cx.observe(&host, |_, _, cx| cx.notify()),
                _host: host.downgrade(),
                renders,
            }
        }
    }

    impl Render for Panel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            div().size_full()
        }
    }

    struct Host {
        left: Entity<Panel>,
        right: Entity<Panel>,
    }

    impl Host {
        fn new(
            left_renders: Rc<Cell<usize>>,
            right_renders: Rc<Cell<usize>>,
            cx: &mut Context<Self>,
        ) -> Self {
            // Deliberately observe the host while it is still being constructed,
            // mirroring `DiscordApp::new` creating its cached panel views.
            let host = cx.entity();
            let left = cx.new(|cx| Panel::new(host.clone(), left_renders, cx));
            let right = cx.new(|cx| Panel::new(host, right_renders, cx));
            Self { left, right }
        }
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .size_full()
                .child(
                    self.left
                        .clone()
                        .cached(StyleRefinement::default().flex_1().h_full()),
                )
                .child(
                    self.right
                        .clone()
                        .cached(StyleRefinement::default().flex_1().h_full()),
                )
        }
    }

    struct Harness {
        host: Entity<Host>,
    }

    impl Harness {
        fn new(
            left_renders: Rc<Cell<usize>>,
            right_renders: Rc<Cell<usize>>,
            cx: &mut Context<Self>,
        ) -> Self {
            Self {
                host: cx.new(|cx| Host::new(left_renders, right_renders, cx)),
            }
        }
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.host.clone().into_any_element()
        }
    }

    #[gpui::test]
    fn cached_siblings_isolate_renders_until_the_observed_model_notifies(cx: &mut TestAppContext) {
        let left_renders = Rc::new(Cell::new(0));
        let right_renders = Rc::new(Cell::new(0));
        let (harness, cx) = cx
            .add_window_view(|_, cx| Harness::new(left_renders.clone(), right_renders.clone(), cx));
        let host = harness.read_with(cx, |harness, _| harness.host.clone());
        let left = host.read_with(cx, |host, _| host.left.clone());

        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let left_initial = left_renders.get();
        let right_initial = right_renders.get();
        assert!(left_initial > 0 && right_initial > 0);

        left.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(left_renders.get(), left_initial + 1);
        assert_eq!(right_renders.get(), right_initial);

        host.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(left_renders.get(), left_initial + 2);
        assert_eq!(right_renders.get(), right_initial + 1);
    }
}
