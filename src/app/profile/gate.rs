use super::*;
use crate::store::RelationshipKind;

pub(super) const MIN_HEIGHT: f32 = 440.;
const AVATAR: f32 = 64.;
const BADGE: f32 = 26.;

impl DiscordApp {
    pub(crate) fn profile_gated(&self, user_id: u64) -> bool {
        user_id != self.store.current_user_id
            && !self.revealed_profiles.contains(&user_id)
            && self
                .store
                .relationship(user_id)
                .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
    }

    fn reveal_profile(&mut self, user_id: u64, cx: &mut Context<Self>) {
        if self.revealed_profiles.insert(user_id) {
            cx.notify();
        }
    }
}

fn prompt_text(name: &str) -> StyledText {
    let before = "You blocked ";
    let text = format!(
        "{before}{name}, so just making sure. They won't be notified, and will still be blocked"
    );
    StyledText::new(text).with_highlights([(
        before.len()..before.len() + name.len(),
        HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
    )])
}

fn placeholder() -> Div {
    let bar = |width: DefiniteLength, height: f32| {
        div()
            .w(width)
            .h(rpx(height))
            .rounded(rpx(8.))
            .bg(rgba(theme::selected()))
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .p(rpx(16.))
        .opacity(0.4)
        .child(
            div()
                .size(rpx(80.))
                .rounded_full()
                .bg(rgba(theme::selected())),
        )
        .child(bar(relative(0.5), 16.))
        .child(bar(relative(0.35), 12.))
        .child(bar(relative(1.), 88.))
        .child(bar(relative(1.), 120.))
        .child(bar(relative(0.6), 12.))
}

pub(super) fn gate(app: &DiscordApp, user_id: u64, radius: f32, cx: &App) -> AnyElement {
    let user = app.store.user(user_id);
    let name = user.map(|user| user.name.clone()).unwrap_or_default();
    let entity = app.entity.clone();
    let avatar = div()
        .relative()
        .size(rpx(AVATAR))
        .child(theme::user_avatar(user, user_id, AVATAR, None, false, cx))
        .child(
            div()
                .absolute()
                .right(rpx(-6.))
                .bottom(rpx(-4.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(BADGE))
                .rounded_full()
                .bg(rgb(theme::elevated()))
                .child(theme::icon("ban", 18., theme::muted())),
        );
    div()
        .id(("profile-gate", user_id))
        .absolute()
        .inset_0()
        .rounded(rpx(radius))
        .overflow_hidden()
        .bg(rgb(theme::elevated()))
        .occlude()
        .without_text_selection()
        .child(placeholder())
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(rpx(8.))
                .px(rpx(24.))
                .child(avatar)
                .child(
                    div()
                        .mt(rpx(4.))
                        .text_size(rpx(20.))
                        .line_height(rpx(24.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Show Profile?"),
                )
                .child(
                    div()
                        .text_center()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::text()))
                        .child(prompt_text(&name)),
                )
                .child(
                    div()
                        .id(("profile-gate-show", user_id))
                        .mt(rpx(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(rpx(32.))
                        .px(rpx(14.))
                        .rounded(rpx(8.))
                        .cursor_pointer()
                        .bg(rgb(theme::BLURPLE))
                        .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(white())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            entity
                                .update(cx, |app, cx| app.reveal_profile(user_id, cx))
                                .ok();
                        })
                        .child("Yes, show profile"),
                ),
        )
        .into_any_element()
}
