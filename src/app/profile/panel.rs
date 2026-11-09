use gpui_component::scroll::ScrollableElement as _;

use super::frame::framed;
use super::loading::cosmetic_skus;
use super::popout::{ClickHandler, board_sections, display_names, header, member_since, skeleton};
use super::*;

pub(in crate::app) const CARD_WIDTH: f32 = 340.;
pub(in crate::app) const PANEL_INSET: f32 = 8.;
const BUTTON_HEIGHT: f32 = 40.;

impl ProfileCard {
    pub(in crate::app) fn panel(user_id: u64) -> Self {
        let mut card = Self::new(
            user_id,
            None,
            point(px(0.), px(0.)),
            Anchor::TopRight,
            ProfileOrigin::Member,
        );
        card.panel = true;
        card
    }

    pub(in crate::app) fn user_id(&self) -> u64 {
        self.user_id
    }
}

impl DiscordApp {
    pub(in crate::app) fn dm_profile_loaded(
        &mut self,
        user_id: u64,
        profile: Arc<model::Profile>,
        cx: &mut Context<Self>,
    ) {
        self.store.absorb_profile(None, &profile);
        cx.notify();
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(card) = self
            .dm_profile
            .as_mut()
            .filter(|card| card.user_id == user_id)
        else {
            return;
        };
        let (effect, frame) = cosmetic_skus(&profile);
        let games = widget_game_ids(&profile);
        let applications = widget_application_ids(&profile);
        card.profile = Some(profile);
        card.load_applications(session.clone(), applications.clone(), cx);
        card.load_widget_configs(session.clone(), applications, cx);
        card.load_games(session.clone(), games, cx);
        card.load_linked_metadata(session, cx);
        if effect.is_some() {
            card._tasks.push(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(EFFECT_TICK).await;
                    let tick = this.update(cx, |app, cx| {
                        app.views.members.update(cx, |_, cx| cx.notify());
                    });
                    if tick.is_err() {
                        break;
                    }
                }
            }));
        }
        self.load_profile_cosmetics(true, effect, frame, cx);
        self.prefetch_profile_details(true, cx);
        self.load_avatar_theme(true, cx);
    }
}

pub(in crate::app) fn render(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let layout = Layout::panel(theme.is_some());
    let surface = theme.map_or(theme::elevated(), ProfileTheme::surface);
    let open_full = |app: &DiscordApp| -> ClickHandler {
        let entity = app.entity.clone();
        Box::new(move |_, _, cx| {
            entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
        })
    };
    let body = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(rpx(12.))
        .px(rpx(layout.padding))
        .pt(rpx(4.))
        .pb(rpx(16.))
        .child(display_names(app, card, false, Some(open_full(app)), cx))
        .map(|this| {
            if card.profile.is_none() {
                return this.child(skeleton(card.user_id));
            }
            this.children(popout::bio(app, card, false, cx))
                .child(member_since(app, card, cx))
                .children(board_sections(app, card, cx))
        });
    let open_button = open_full(app);
    let footer = div()
        .flex_none()
        .px(rpx(layout.padding))
        .pt(rpx(4.))
        .pb(rpx(layout.padding))
        .child(
            div()
                .id("dm-panel-full-profile")
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(BUTTON_HEIGHT))
                .rounded(rpx(8.))
                .bg(palette.panel)
                .border_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(move |style| style.bg(palette.raised))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.strong))
                .without_text_selection()
                .on_click(move |event, window, cx| open_button(event, window, cx))
                .child("View Full Profile"),
        );
    let inner = div()
        .id("dm-panel-card")
        .on_hover(effect_hover(app, true))
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .rounded(rpx(layout.corner))
        .overflow_hidden()
        .text_color(rgb(palette.text))
        .map(|this| match theme {
            Some(theme) => this.bg(theme.overlay()),
            None => this,
        })
        .child(
            div()
                .id("dm-panel-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(header(
                    app,
                    card,
                    &layout,
                    surface,
                    Some(open_full(app)),
                    true,
                    cx,
                ))
                .child(body),
        )
        .child(footer)
        .children(effect_layers(card, layout.width, layout.corner, false, cx))
        .when(app.profile_gated(card.user_id), |this| {
            this.child(gate::gate(app, card.user_id, layout.corner, cx))
        });
    let outer = div()
        .relative()
        .size_full()
        .rounded(rpx(8.))
        .map(|this| match theme {
            Some(theme) => this.p(rpx(THEME_BORDER)).bg(theme.outer()),
            None => this
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border())),
        })
        .child(inner)
        .into_any_element();
    framed(card, CARD_WIDTH, outer, true, cx)
}
