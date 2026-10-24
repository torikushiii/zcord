//! Cached child views for the chat shell.
//!
//! `DiscordApp` only builds the layout skeleton and the overlays; each hot panel
//! is a separate view entity so a notify from one panel does not dirty the rest.
//! Every view observes `DiscordApp` with a strong entity handle but keeps only a
//! weak handle for rendering, so the ownership graph is acyclic.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
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
        chat::messages(app.read(cx), cx)
    }
}

pub(crate) struct ComposerView {
    app: WeakEntity<DiscordApp>,
    height: Rc<Cell<Option<Pixels>>>,
    _observe: Subscription,
}

impl ComposerView {
    fn new(app: &Entity<DiscordApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            height: Rc::new(Cell::new(None)),
            _observe: observe_app(app, cx),
        }
    }

    pub(crate) fn height(&self) -> Option<Pixels> {
        self.height.get()
    }
}

impl Render for ComposerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        count_render(ViewKind::Composer);
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app.read(cx);
        if app.selected_forum().is_some() {
            return div().into_any_element();
        }
        let measured = self.height.clone();
        let view = cx.entity().entity_id();
        div()
            .w_full()
            .child(chat::composer(app, cx))
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
        overlay_root()
            .children(chat::delete_dialog(app, cx))
            .children(profile::render(app, window, cx))
            .children(lightbox::render(app, window, cx))
            .children(emoji_picker::render(app, cx))
            .children(channel_list::account_menu(app))
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
    use super::{composer_style, overlay_root, overlays_style};
    use gpui_component::{
        ElementExt,
        input::{Textarea, TextareaState},
    };
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        Anchor, AppContext, Bounds, Context, Entity, Modifiers, Pixels, Render, StyleRefinement,
        Subscription, TestAppContext, WeakEntity, Window, anchored, canvas, deferred, div, point,
        prelude::*, px,
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

    struct CenteredOverlay {
        root: Rc<Cell<Option<Bounds<Pixels>>>>,
        dialog: Rc<Cell<Option<Bounds<Pixels>>>>,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for CenteredOverlay {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let root = self.root.clone();
            let dialog = self.dialog.clone();
            let clicks = self.clicks.clone();
            overlay_root()
                .on_prepaint(move |bounds, _, _| root.set(Some(bounds)))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("centered-dialog")
                                .w(crate::theme::rpx(200.))
                                .h(crate::theme::rpx(100.))
                                .on_prepaint(move |bounds, _, _| dialog.set(Some(bounds)))
                                .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                        ),
                )
        }
    }

    struct CenteredOverlayHost {
        overlay: Entity<CenteredOverlay>,
    }

    impl Render for CenteredOverlayHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .debug_selector(|| "overlay-viewport".into())
                .child(self.overlay.clone().cached(overlays_style()))
        }
    }

    #[gpui::test]
    fn cached_overlays_center_dialogs_in_the_full_viewport(cx: &mut TestAppContext) {
        let root = Rc::new(Cell::new(None));
        let dialog = Rc::new(Cell::new(None));
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, cx| CenteredOverlayHost {
            overlay: cx.new(|_| CenteredOverlay {
                root: root.clone(),
                dialog: dialog.clone(),
                clicks: clicks.clone(),
            }),
        });
        for zoom in [1., 1.25, 1.5] {
            cx.update(|window, cx| {
                window.set_rem_size(px(16. * zoom));
                let _ = window.draw(cx);
            });
            // Check both newly laid-out content and its cached replay.
            for _ in 0..2 {
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                let viewport = cx.debug_bounds("overlay-viewport").unwrap();
                let root_bounds = root.get().unwrap();
                let dialog_bounds = dialog.get().unwrap();
                assert_eq!(root_bounds, viewport);
                assert!((dialog_bounds.center().x - viewport.center().x).abs() <= px(1.));
                assert!((dialog_bounds.center().y - viewport.center().y).abs() <= px(1.));
                let before = clicks.get();
                cx.simulate_click(viewport.center(), Modifiers::none());
                assert_eq!(clicks.get(), before + 1);
            }
        }
    }

    struct MeasuredContent {
        measured: Rc<Cell<Option<Pixels>>>,
        height: Rc<Cell<Pixels>>,
    }

    impl Render for MeasuredContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let measured = self.measured.clone();
            div()
                .w_full()
                .child(div().w_full().h(self.height.get()))
                .child(
                    canvas(
                        move |bounds, _, _| measured.set(Some(bounds.size.height)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
        }
    }

    struct MeasuredHost {
        content: Entity<MeasuredContent>,
        measured: Rc<Cell<Option<Pixels>>>,
    }

    impl Render for MeasuredHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let height = self.measured.get().unwrap_or(px(64.));
            div().size_full().child(
                self.content
                    .clone()
                    .cached(StyleRefinement::default().w_full().h(height)),
            )
        }
    }

    #[gpui::test]
    fn a_cached_view_measures_its_intrinsic_height_for_the_next_frame(cx: &mut TestAppContext) {
        let measured = Rc::new(Cell::new(None));
        let content = measured.clone();
        let host = measured.clone();
        let height = Rc::new(Cell::new(px(100.)));
        let content_height = height.clone();
        let (host, cx) = cx.add_window_view(move |_, cx| MeasuredHost {
            content: cx.new(|_| MeasuredContent {
                measured: content,
                height: content_height,
            }),
            measured: host,
        });
        let content = host.read_with(cx, |host, _| host.content.clone());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(measured.get(), Some(px(100.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(measured.get(), Some(px(100.)));

        content.update(cx, |_, cx| {
            height.set(px(40.));
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(measured.get(), Some(px(40.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(measured.get(), Some(px(40.)));
    }

    struct DeferredOverlay {
        renders: Rc<Cell<usize>>,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for DeferredOverlay {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            let clicks = self.clicks.clone();
            div().absolute().inset_0().child(deferred(
                anchored()
                    .position(point(px(100.), px(80.)))
                    .anchor(Anchor::TopLeft)
                    .child(
                        div()
                            .id("cached-popup")
                            .occlude()
                            .w(px(120.))
                            .h(px(60.))
                            .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                    ),
            ))
        }
    }

    struct DeferredHost {
        overlay: Entity<DeferredOverlay>,
    }

    impl Render for DeferredHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.overlay.clone().cached(overlays_style()))
        }
    }

    #[gpui::test]
    fn cached_views_replay_deferred_popups(cx: &mut TestAppContext) {
        let renders = Rc::new(Cell::new(0));
        let clicks = Rc::new(Cell::new(0));
        let (made_renders, made_clicks) = (renders.clone(), clicks.clone());
        let (_host, cx) = cx.add_window_view(move |_, cx| DeferredHost {
            overlay: cx.new(|_| DeferredOverlay {
                renders: made_renders,
                clicks: made_clicks,
            }),
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        };
        draw(cx);
        let initial = renders.get();
        assert!(initial > 0);

        // Only the root notified/drew: the cached overlay is reused, deferred draw and
        // all. The click is dispatched against this reused frame's replayed hitboxes.
        draw(cx);
        assert_eq!(renders.get(), initial);
        cx.simulate_click(point(px(160.), px(110.)), Modifiers::none());
        assert_eq!(clicks.get(), 1);
    }
    struct OverflowPanel {
        clicks: Rc<Cell<usize>>,
    }

    impl Render for OverflowPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div().relative().w_full().h_full().child(
                div()
                    .id("overflow-popup")
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .w(px(100.))
                    .h(px(80.))
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
            )
        }
    }

    struct OverflowHost {
        panel: Entity<OverflowPanel>,
    }

    impl Render for OverflowHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().pt(px(100.)).child(
                self.panel
                    .clone()
                    .cached(composer_style(Some(px(50.)), false)),
            )
        }
    }

    #[gpui::test]
    fn a_cached_composer_does_not_clip_popups_that_overflow_upward(cx: &mut TestAppContext) {
        let clicks = Rc::new(Cell::new(0));
        let made_clicks = clicks.clone();
        let (_host, cx) = cx.add_window_view(move |_, cx| OverflowHost {
            panel: cx.new(|_| OverflowPanel {
                clicks: made_clicks,
            }),
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        // The 50px-tall view starts at y=100; its 80px popup is pinned to the bottom
        // and must remain clickable above the view's own bounds.
        cx.simulate_click(point(px(50.), px(110.)), Modifiers::none());
        assert_eq!(clicks.get(), 1);
    }
    struct TypedPanel {
        input: Entity<TextareaState>,
        renders: Rc<Cell<usize>>,
    }

    impl Render for TypedPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            div().w_full().h(px(50.)).child(Textarea::new(&self.input))
        }
    }

    struct TypedHost {
        panel: Entity<TypedPanel>,
    }

    impl Render for TypedHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                self.panel
                    .clone()
                    .cached(composer_style(Some(px(64.)), false)),
            )
        }
    }

    #[gpui::test]
    fn a_cached_composer_rerenders_when_the_textarea_changes(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let renders = Rc::new(Cell::new(0));
        let counted = renders.clone();
        let (host, cx) = cx.add_window_view(move |window, cx| {
            let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 15));
            input.update(cx, |input, cx| input.focus(window, cx));
            TypedHost {
                panel: cx.new(|_| TypedPanel {
                    input,
                    renders: counted,
                }),
            }
        });
        let _panel = host.read_with(cx, |host, _| host.panel.clone());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let initial = renders.get();
        cx.simulate_input("hello");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(
            renders.get() > initial,
            "typing in a cached composer must re-render its textarea"
        );
    }
}
