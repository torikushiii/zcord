use super::tooltip::tooltip_above;
use super::*;

pub fn user_avatar(
    user: Option<&User>,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = user.and_then(|user| avatar_image(&user.avatar_spec(), size, animate, notch, cx));
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        user.and_then(|user| user.decoration.as_ref())
            .and_then(|data| decoration(user_id, data, size, animate, notch, cx)),
    )
}

pub fn default_avatar(
    index: u8,
    size: f32,
    status: Option<(Status, u32)>,
    cx: &App,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let id = u64::from(index);
    avatar(
        "",
        user_color(id),
        size,
        status,
        false,
        id,
        avatar_image(&ImageSpec::DefaultAvatar { index }, size, false, notch, cx),
        None,
    )
}

pub fn spec_avatar(
    spec: &ImageSpec,
    user_id: u64,
    name: &str,
    size: f32,
    cx: &App,
) -> impl IntoElement + use<> {
    let initials: String = name.chars().take(2).collect();
    avatar(
        &initials,
        user_color(user_id),
        size,
        None,
        false,
        user_id,
        avatar_image(spec, size, false, None, cx),
        None,
    )
}

pub fn member_avatar(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    member_avatar_with(store, guild_id, user_id, size, status, animate, true, cx)
}

pub fn webhook_avatar(
    author_id: u64,
    author: &WebhookAuthor,
    size: f32,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let spec = crate::store::avatar_spec(author_id, author.avatar.as_deref(), Some("0000"));
    let image = avatar_image(&spec, size, animate, None, cx);
    let initials: String = author.name.chars().take(2).collect();
    avatar(
        &initials,
        user_color(author_id),
        size,
        None,
        false,
        author_id,
        image,
        None,
    )
}

pub fn preview_avatar(
    user: Option<&User>,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    spec: Option<ImageSpec>,
    file: Option<std::path::PathBuf>,
    decorated_with: Option<&Decoration>,
    cx: &App,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = match file {
        Some(path) => Some(img(path).size_full().rounded_full().into_any_element()),
        None => spec
            .or_else(|| user.map(User::avatar_spec))
            .and_then(|spec| avatar_image(&spec, size, true, notch, cx)),
    };
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        decorated_with.and_then(|data| decoration(user_id, data, size, true, notch, cx)),
    )
}

pub fn placeholder_avatar(
    size: f32,
    decorated_with: Option<&Decoration>,
    cx: &App,
) -> impl IntoElement + use<> {
    let image = div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(0x5C5E66))
        .child(icon("discord", size * 0.6, 0x3B3D44))
        .into_any_element();
    avatar(
        "",
        hsla(0., 0., 0., 0.),
        size,
        None,
        false,
        0,
        Some(image),
        decorated_with.and_then(|data| decoration(0, data, size, true, None, cx)),
    )
}

pub fn undecorated_member_avatar(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    cx: &App,
) -> impl IntoElement + use<> {
    member_avatar_with(store, guild_id, user_id, size, None, false, false, cx)
}

#[allow(clippy::too_many_arguments)]
fn member_avatar_with(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    decorated: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let user = store.user(user_id);
    let spec = store
        .member_avatar_spec(guild_id, user_id)
        .or_else(|| user.map(User::avatar_spec));
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = spec.and_then(|spec| avatar_image(&spec, size, animate, notch, cx));
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        store
            .member_decoration(guild_id, user_id)
            .filter(|_| decorated)
            .and_then(|data| decoration(user_id, data, size, animate, notch, cx)),
    )
}

fn avatar_image(
    spec: &ImageSpec,
    size: f32,
    animate: bool,
    notch: Option<profile_avatar::Geometry>,
    cx: &App,
) -> Option<AnyElement> {
    let pixels = image_px(size);
    let animated = (animate || crate::settings::always_animate())
        .then(|| spec.animated())
        .flatten()
        .and_then(|animated| animated::resolve(&animated, pixels, cx));
    if let Some(notch) = notch {
        let source = match animated {
            Some(image) => profile_avatar::Source::Decoded(image),
            None => profile_avatar::Source::File(images::resolve(spec, pixels, cx)?),
        };
        return Some(profile_avatar::image(
            "profile-avatar-image",
            source,
            notch,
            false,
        ));
    }
    Some(match animated {
        Some(image) => animated::image(image)
            .size_full()
            .rounded_full()
            .id("avatar-animated")
            .into_any_element(),
        None => img(images::resolve(spec, pixels, cx)?)
            .size_full()
            .rounded_full()
            .into_any_element(),
    })
}

pub(super) const DECORATION_SCALE: f32 = 1.2;

pub(super) fn decoration(
    user_id: u64,
    decoration: &Decoration,
    size: f32,
    animate: bool,
    notch: Option<profile_avatar::Geometry>,
    cx: &App,
) -> Option<AnyElement> {
    if size < 24. {
        return None;
    }
    let frame = size * DECORATION_SCALE;
    let pixels = image_px(frame);
    let animates =
        (animate || crate::settings::always_animate()) && decoration.asset.starts_with("a_");
    let animated = animates
        .then(|| animated::resolve(&decoration.image_spec(true), pixels, cx))
        .flatten();
    let still = if animated.is_none() {
        images::resolve(&decoration.image_spec(false), pixels, cx)
    } else {
        None
    };
    let image = if let Some(notch) = notch {
        let source = match animated {
            Some(image) => profile_avatar::Source::Decoded(image),
            None => profile_avatar::Source::File(still?),
        };
        profile_avatar::image(("decoration", user_id), source, notch, true)
    } else {
        match animated {
            Some(image) => animated::image(image)
                .id(("decoration", user_id))
                .size_full()
                .into_any_element(),
            None => img(still?).size_full().into_any_element(),
        }
    };
    Some(
        div()
            .absolute()
            .left(rpx((size - frame) / 2.))
            .top(rpx((size - frame) / 2.))
            .size(rpx(frame))
            .child(image)
            .into_any_element(),
    )
}

pub fn activity_icon(user: &User) -> Option<&'static str> {
    if user.playing {
        Some("gamepad-2")
    } else if user.activity_kind == Some(2) {
        Some("music")
    } else {
        None
    }
}

pub fn avatar(
    initials: &str,
    color: Hsla,
    size: f32,
    status: Option<(Status, u32)>,
    on_mobile: bool,
    user_id: u64,
    image: Option<AnyElement>,
    decoration: Option<AnyElement>,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let dot = (size * 0.3).round();
    div()
        .debug_selector(|| "user-avatar".into())
        .relative()
        .flex_none()
        .size(rpx(size))
        .child(match image {
            Some(image) => image,
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(notch.is_none(), |this| this.bg(color))
                .children(notch.map(|notch| profile_avatar::background(notch, color)))
                .text_color(rgb(bg()))
                .text_size(rpx(size * 0.36))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials.to_owned())
                .into_any_element(),
        })
        .children(decoration)
        .children(status.map(|(status, ring)| {
            let phone = on_mobile && status == Status::Online;
            if let Some(notch) = notch {
                return if phone {
                    profile_avatar::mobile_indicator(notch, ring)
                } else {
                    profile_avatar::indicator(notch, status, ring)
                };
            }
            if phone {
                return div()
                    .debug_selector(|| "avatar-status".into())
                    .absolute()
                    .right(rpx(-2.))
                    .bottom(rpx(-2.))
                    .p(rpx(2.))
                    .rounded(rpx(dot * 0.3))
                    .bg(rgb(ring))
                    .child(mobile_status(dot, ring))
                    .into_any_element();
            }
            div()
                .debug_selector(|| "avatar-status".into())
                .absolute()
                .right(rpx(-2.))
                .bottom(rpx(-2.))
                .size(rpx(dot + 4.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(rgb(ring))
                .bg(rgb(status_color(status)))
                .children(status_cutout(status, dot, ring))
                .into_any_element()
        }))
        .children(status.map(|(status, _)| StatusTooltip {
            user_id,
            target: (dot + 4.).min(size),
            label: status.label(),
        }))
}

#[derive(IntoElement)]
struct StatusTooltip {
    user_id: u64,
    target: f32,
    label: &'static str,
}

impl RenderOnce for StatusTooltip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ("status-tooltip", self.user_id);
        let hovered = window.use_keyed_state(key, cx, |_, _| false);
        let shown = *hovered.read(cx);
        div()
            .id(key)
            .absolute()
            .right(rpx(-2.))
            .bottom(rpx(-2.))
            .size(rpx(self.target))
            .on_hover(move |over, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    *hovered = *over;
                    cx.notify();
                });
            })
            .when(shown, |this| this.child(tooltip_above(self.label)))
    }
}

pub(super) fn mobile_status(width: f32, surface: u32) -> Div {
    let height = width * 1.5;
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(width * 0.1875))
        .bg(rgb(STATUS_ONLINE))
        .child(
            div()
                .absolute()
                .left(rpx(width * 0.125))
                .top(rpx(height / 6.))
                .w(rpx(width * 0.75))
                .h(rpx(height * 0.5))
                .bg(rgb(surface)),
        )
        .child(
            div()
                .absolute()
                .left(rpx(width * 0.375))
                .top(rpx(height * 0.75))
                .w(rpx(width * 0.25))
                .h(rpx(height / 6.))
                .rounded_full()
                .bg(rgb(surface)),
        )
}

fn status_cutout(status: Status, dot: f32, ring: u32) -> Option<Div> {
    let cutout = div().flex_none().bg(rgb(ring));
    match status {
        Status::Online => None,
        Status::DoNotDisturb => Some(
            cutout
                .w(rpx(dot * 0.75))
                .h(rpx((dot * 0.25).max(2.)))
                .rounded(rpx(dot)),
        ),
        Status::Idle => Some(
            cutout
                .absolute()
                .left(rpx(-dot * 0.125))
                .top(rpx(-dot * 0.125))
                .size(rpx(dot * 0.75))
                .rounded_full(),
        ),
        Status::Offline => Some(cutout.size(rpx(dot * 0.5)).rounded_full()),
    }
}
