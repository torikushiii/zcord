use super::names::display_names;
use super::sections::{
    add_app_button, app_section, bio, command_chips, is_app, playing, role_chips, roles,
};
use super::*;

pub(super) fn popout_body(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let entity = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .px(rpx(layout.padding))
        .pt(rpx(4.))
        .pb(rpx(8.))
        .child(display_names(
            app,
            card,
            false,
            Some(Box::new(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })),
            cx,
        ))
        .map(|this| {
            if card.loading() {
                return this.child(skeleton(card.user_id));
            }
            this.children(add_app_button(app, card, true))
                .children(bio(app, card, true, cx))
                .children(board_sections(app, card, cx))
                .map(|this| {
                    if is_app(app, card) {
                        let palette = card.palette();
                        this.children(
                            command_chips(app, card)
                                .map(|chips| app_section("Commands", chips, palette)),
                        )
                        .children(
                            role_chips(app, card).map(|chips| app_section("Roles", chips, palette)),
                        )
                    } else {
                        this.when(!card.is_account(), |this| this.children(roles(app, card)))
                    }
                })
        })
        .when(card.is_account(), |this| {
            this.child(account_menu(app, card, window, cx))
        })
}

pub(crate) fn board_sections(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> impl Iterator<Item = AnyElement> {
    let user = app.store.user(card.user_id);
    let playing = match user {
        Some(user) if user.bot => playing(user, card.palette()),
        _ => board::popout_playing(app, card, cx)
            .or_else(|| user.and_then(|user| playing(user, card.palette()))),
    };
    let voice = board::popout_voice(app, card, cx);
    let (activity, voice) = match playing {
        Some(playing) if voice.is_some() => (
            Some(board::card_stack(
                app,
                card,
                Stack::Activity,
                Tab::Activity,
                playing,
                cx,
            )),
            None,
        ),
        playing => (playing, voice),
    };
    [
        board::popout_widget(app, card, cx),
        activity,
        voice,
        board::listening(app, card, false, cx),
    ]
    .into_iter()
    .flatten()
}

fn account_menu(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let palette = card.palette();
    let row = |id: &'static str,
               icon: &'static str,
               icon_size: f32,
               text: &'static str,
               action: AccountAction| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(12.))
            .min_h(rpx(36.))
            .px(rpx(12.))
            .py(rpx(8.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .line_height(rpx(20.))
            .text_color(rgb(palette.text))
            .hover(|style| style.bg(palette.hover).text_color(rgb(palette.strong)))
            .without_text_selection()
            .on_click(account_action(app, action))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(18.))
                    .child(theme::icon(icon, icon_size, palette.muted)),
            )
            .child(div().flex_1().min_w_0().truncate().child(text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .child(
            div()
                .rounded(rpx(8.))
                .bg(palette.panel)
                .child(row(
                    "account-view-profile",
                    "pencil",
                    14.,
                    "Edit Profile",
                    AccountAction::ViewProfile,
                ))
                .child(div().h(rpx(1.)).mx(rpx(12.)).bg(palette.border))
                .child(status::trigger(app, card, window, cx))
                .child(div().h(rpx(1.)).mx(rpx(12.)).bg(palette.border))
                .child(row(
                    "account-settings",
                    "settings",
                    18.,
                    "User Settings",
                    AccountAction::Settings,
                )),
        )
        .child(
            div()
                .rounded(rpx(8.))
                .bg(palette.panel)
                .child(switch_menu::trigger(app, card, window, cx))
                .child(div().h(rpx(1.)).mx(rpx(12.)).bg(palette.border))
                .child(row(
                    "account-copy-user-id",
                    "copy",
                    18.,
                    "Copy User ID",
                    AccountAction::CopyUserId,
                )),
        )
        .into_any_element()
}

pub(crate) fn skeleton(user_id: u64) -> impl IntoElement {
    let bar = |width: DefiniteLength, height: f32| {
        div()
            .w(width)
            .h(rpx(height))
            .rounded(rpx(height / 2.))
            .bg(rgba(theme::selected()))
    };
    let pill = |width: f32| bar(rpx(width).into(), 20.);
    let content = div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(bar(relative(1.), 12.))
        .child(bar(relative(0.85), 12.))
        .child(bar(relative(0.55), 12.))
        .child(div().h(rpx(4.)))
        .child(bar(rpx(48.).into(), 12.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(rpx(4.))
                .child(pill(72.))
                .child(pill(56.))
                .child(pill(88.)),
        );
    pulse(content, "profile-skeleton-pulse", user_id)
}

pub(super) fn pulse<E: IntoElement + Styled + 'static>(
    element: E,
    key: &'static str,
    user_id: u64,
) -> AnimationElement<E> {
    element.with_animation(
        (key, user_id),
        Animation::new(Duration::from_millis(1200))
            .repeat()
            .with_easing(pulsating_between(0.45, 1.)),
        |this, delta| this.opacity(delta),
    )
}
