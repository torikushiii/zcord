use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;
use gpui_tokio::Tokio;

use crate::{
    animated,
    app::{
        DISCORD_EPOCH_MS, DiscordApp,
        message_body::{BodyContext, label, render_body, wrapped_label},
    },
    discord::{ProfileKind, model},
    images::{self, ImageSpec, profile_frame_layer_url},
    store::User,
    theme::{self, rpx},
};

const POPOUT_WIDTH: f32 = 300.;
const MODAL_WIDTH: f32 = 962.;
const MODAL_HEIGHT: f32 = 800.;
const MODAL_MARGIN: f32 = 48.;
const MODAL_RADIUS: f32 = 16.;
const MODAL_CARD_RADIUS: f32 = 16.;
const MODAL_MIN_VIEWPORT_WIDTH: f32 = 929.;
const MODAL_MIN_VIEWPORT_HEIGHT: f32 = 785.;
const BACKDROP_SOURCE: u32 = 512;
const BANNER_SOURCE: u32 = 480;
const BACKDROP_BLUR: f32 = 4.;
const BACKDROP_SATURATION: f32 = 0.5;
const BACKDROP_OPACITY: f32 = 0.3;
const BACKDROP_HEIGHT: f32 = 280.;
const EFFECT_TICK: Duration = Duration::from_millis(100);
const ENTER: Duration = Duration::from_millis(180);
const EFFECT_HEIGHT_RATIO: f32 = 880. / 450.;
const THEME_OVERLAY: f32 = 0.6;
const SURFACE_HIGHEST: u32 = 0x26262C;
const MAX_VISIBLE_BADGES: usize = 6;
const STATUS_OVERLAP: f32 = 10.;
const THEME_BORDER: f32 = 2.;
const BIO_LINES: usize = 3;

pub(crate) struct ProfileCard {
    user_id: u64,
    guild_id: Option<u64>,
    position: Point<Pixels>,
    anchor: Anchor,
    modal: bool,
    profile: Option<Arc<model::Profile>>,
    effect: Option<Arc<model::CollectibleProduct>>,
    frame: Option<Arc<model::CollectibleProduct>>,
    opened: Instant,
    tab: Tab,
    _tasks: Vec<Task<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Activity,
    Servers,
}

struct Layout {
    width: f32,
    banner: f32,
    avatar: f32,
    avatar_left: f32,
    ring: f32,
    padding: f32,
    status_left: f32,
    header_min: f32,
    corner: f32,
}

impl Layout {
    fn popout(themed: bool) -> Self {
        let inset = if themed { THEME_BORDER } else { 0. };
        Self {
            width: POPOUT_WIDTH,
            banner: 105.,
            avatar: 80.,
            avatar_left: 16. - inset,
            ring: 6.,
            padding: 16.,
            status_left: 105.,
            header_min: 105. + 35. - inset,
            corner: if themed { 6. } else { 7. },
        }
    }

    fn modal(width: f32) -> Self {
        Self {
            width,
            banner: 140.,
            avatar: 120.,
            avatar_left: 32.,
            ring: 8.,
            padding: 32.,
            status_left: 32. + 120. + 20.,
            header_min: 140. + 60. + 20.,
            corner: MODAL_CARD_RADIUS - 1.,
        }
    }
}

impl DiscordApp {
    pub(crate) fn open_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        cx: &mut Context<Self>,
    ) {
        let guild_id = (self.selected_guild_id != 0).then_some(self.selected_guild_id);
        self.profile = Some(ProfileCard {
            user_id,
            guild_id,
            position,
            anchor,
            modal: false,
            profile: None,
            effect: None,
            frame: None,
            opened: Instant::now(),
            tab: Tab::Activity,
            _tasks: Vec::new(),
        });
        self.load_profile(ProfileKind::Popout, cx);
        cx.notify();
    }

    pub(crate) fn open_full_profile(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        card.modal = true;
        card.opened = Instant::now();
        card.tab = Tab::Activity;
        self.load_profile(ProfileKind::Modal, cx);
        cx.notify();
    }

    fn select_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile {
            card.tab = tab;
            cx.notify();
        }
    }

    pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.profile.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn load_profile(&mut self, kind: ProfileKind, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(card) = &mut self.profile else {
            return;
        };
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let request = Tokio::spawn(cx, async move {
            session
                .profile(user_id, guild_id, Some(channel_id), kind)
                .await
        });
        card._tasks.push(cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(profile)) => app.profile_loaded(user_id, profile, cx),
                Ok(Err(error)) => log::warn!("Could not load profile: {error}"),
                Err(_) => {}
            })
            .ok();
        }));
    }

    fn profile_loaded(
        &mut self,
        user_id: u64,
        profile: Arc<model::Profile>,
        cx: &mut Context<Self>,
    ) {
        self.store.absorb_user(&profile.user.user);
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(card) = self.profile.as_mut().filter(|card| card.user_id == user_id) else {
            return;
        };
        let effect = guild_profile(&profile)
            .and_then(|p| p.profile_effect.as_ref())
            .or_else(|| profile.user_profile.as_ref()?.profile_effect.as_ref())
            .map(|effect| effect.sku_id);
        let frame_of = |p: &model::UserProfile| {
            p.collectibles
                .iter()
                .find(|c| c.kind == 3)
                .map(|c| c.sku_id)
        };
        let frame = guild_profile(&profile)
            .and_then(frame_of)
            .or_else(|| profile.user_profile.as_ref().and_then(frame_of));
        card.profile = Some(profile);
        for (sku_id, is_effect) in effect
            .map(|sku| (sku, true))
            .into_iter()
            .chain(frame.map(|sku| (sku, false)))
        {
            let session = session.clone();
            let request =
                Tokio::spawn(cx, async move { session.collectible_product(sku_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let Ok(Ok(product)) = request.await else {
                    return;
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app.profile.as_mut().filter(|c| c.user_id == user_id) else {
                        return;
                    };
                    if is_effect {
                        card.effect = Some(product);
                    } else {
                        card.frame = Some(product);
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        if effect.is_some() {
            card._tasks.push(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(EFFECT_TICK).await;
                    let open = this
                        .update(cx, |app, cx| {
                            let open = app.profile.as_ref().is_some_and(|c| c.user_id == user_id);
                            if open {
                                cx.notify();
                            }
                            open
                        })
                        .unwrap_or(false);
                    if !open {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }
}

fn guild_profile(profile: &model::Profile) -> Option<&model::UserProfile> {
    profile.guild_member_profile.as_ref()
}

pub fn opener(
    app: &DiscordApp,
    user_id: u64,
    anchor: Anchor,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |event, _, cx| {
        cx.stop_propagation();
        let gap = match anchor {
            Anchor::BottomLeft | Anchor::BottomRight => point(px(0.), px(-16.)),
            Anchor::TopRight => point(px(-16.), px(-40.)),
            _ => point(px(16.), px(-40.)),
        };
        let position = event.position() + gap;
        entity
            .update(cx, |app, cx| {
                app.open_profile(user_id, position, anchor, cx)
            })
            .ok();
    }
}

pub fn render(
    app: &DiscordApp,
    window: &Window,
    cx: &mut Context<DiscordApp>,
) -> Option<AnyElement> {
    let card = app.profile.as_ref()?;
    if card.modal {
        Some(modal(app, card, window, cx))
    } else {
        Some(popout(app, card, cx))
    }
}

#[derive(Clone, Copy)]
struct ProfileTheme {
    primary: u32,
    secondary: u32,
}

impl ProfileTheme {
    fn of(profile: Option<&model::Profile>) -> Option<Self> {
        let profile = profile?;
        let colors = guild_profile(profile)
            .and_then(|p| p.theme_colors.as_ref())
            .or_else(|| profile.user_profile.as_ref()?.theme_colors.as_ref())?;
        Some(Self {
            primary: *colors.first()?,
            secondary: *colors.get(1)?,
        })
    }

    fn outer(self) -> Background {
        linear_gradient(
            180.,
            linear_color_stop(rgb(self.primary), 0.),
            linear_color_stop(rgb(self.secondary), 1.),
        )
    }

    fn surface(self) -> u32 {
        shade(self.primary, 1. - THEME_OVERLAY)
    }

    fn modal_background(self) -> Hsla {
        let secondary: Hsla = rgb(self.secondary).into();
        hsla(secondary.h, secondary.s * 0.5, 0.1, 1.)
    }
}

fn shade(color: u32, keep: f32) -> u32 {
    let channel = |shift: u32| ((((color >> shift) & 0xFF) as f32) * keep) as u32;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn popout(app: &DiscordApp, card: &ProfileCard, cx: &mut Context<DiscordApp>) -> AnyElement {
    let theme = ProfileTheme::of(card.profile.as_deref());
    let layout = Layout::popout(theme.is_some());
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let entity = cx.entity();
    let open_full = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        entity.update(cx, |app, cx| app.open_full_profile(cx));
    };
    let inner = div()
        .relative()
        .flex()
        .flex_col()
        .rounded(rpx(if theme.is_some() { 6. } else { 8. }))
        .overflow_hidden()
        .pb(rpx(4.))
        .when(theme.is_some(), |this| {
            this.bg(hsla(0., 0., 0., THEME_OVERLAY))
        })
        .child(header(
            app,
            card,
            &layout,
            surface,
            Some(Box::new(open_full)),
            cx,
        ))
        .child(popout_body(app, card, &layout, cx))
        .children(effect_layers(card, layout.width, cx));
    let outer_width = POPOUT_WIDTH + theme.map_or(0., |_| THEME_BORDER * 2.);
    let outer = div()
        .relative()
        .w(rpx(outer_width))
        .rounded(rpx(8.))
        .shadow_lg()
        .map(|this| match theme {
            Some(theme) => this.p(rpx(THEME_BORDER)).bg(theme.outer()),
            None => this
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER)),
        })
        .child(inner)
        .into_any_element();
    let framed = framed(card, outer_width, outer, false, cx);
    let close = cx.entity();
    let margin = if card.frame.is_some() { 80. } else { 12. };
    deferred(
        anchored()
            .position(card.position)
            .anchor(card.anchor)
            .snap_to_window_with_margin(Edges {
                top: px(margin),
                bottom: px(12.),
                left: px(12.),
                right: px(12.),
            })
            .child(
                div()
                    .id("profile-popout")
                    .relative()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_profile(cx));
                    })
                    .child(framed)
                    .with_animation(
                        ("profile-popout-enter", card.user_id),
                        Animation::new(ENTER).with_easing(ease_out_quint()),
                        |this, delta| this.opacity(delta),
                    ),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

fn header(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    surface: u32,
    on_avatar: Option<ClickHandler>,
    cx: &mut Context<DiscordApp>,
) -> AnyElement {
    let user = app.store.user(card.user_id);
    let status = user
        .filter(|user| !user.playing)
        .and_then(|user| user.activity.clone());
    let avatar = div()
        .id("profile-avatar")
        .absolute()
        .left(rpx(layout.avatar_left - layout.ring))
        .top(rpx(layout.banner - layout.avatar / 2. - layout.ring))
        .p(rpx(layout.ring))
        .rounded_full()
        .bg(rgb(surface))
        .child(theme::member_avatar(
            &app.store,
            card.guild_id.unwrap_or(0),
            card.user_id,
            layout.avatar,
            user.map(|user| (user.status, surface)),
            true,
            cx,
        ))
        .map(|this| match on_avatar {
            Some(handler) => this.cursor_pointer().on_click(handler),
            None => this,
        });
    let status_height = status
        .as_ref()
        .map_or(0., |text| if text.chars().count() > 22 { 54. } else { 36. });
    let height = layout
        .header_min
        .max(layout.banner - STATUS_OVERLAP + status_height);
    div()
        .relative()
        .w_full()
        .h(rpx(height))
        .child(banner(card, layout, cx))
        .child(avatar)
        .children(status.map(|text| status_bubble(card.user_id, &text, layout)))
        .into_any_element()
}

fn banner(card: &ProfileCard, layout: &Layout, cx: &App) -> impl IntoElement {
    let profile = card.profile.as_deref();
    let spec = profile.and_then(|p| {
        let member_banner = p.guild_member.as_ref().and_then(|m| m.banner.clone());
        match (member_banner, card.guild_id) {
            (Some(hash), Some(guild_id)) => Some(ImageSpec::GuildBanner {
                guild_id,
                user_id: card.user_id,
                hash,
            }),
            _ => p.user.banner.clone().map(|hash| ImageSpec::Banner {
                user_id: card.user_id,
                hash,
            }),
        }
    });
    let pixels = BANNER_SOURCE;
    let corner = rpx(layout.corner);
    let image = spec.and_then(|spec| {
        let animated = spec
            .animated()
            .and_then(|animated| animated::resolve(&animated, pixels, cx));
        Some(match animated {
            Some(image) => animated::image(image)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .id("profile-banner")
                .into_any_element(),
            None => img(images::resolve(&spec, pixels, cx)?)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
        })
    });
    let color = profile
        .and_then(|p| p.user.banner_color.or(p.user.accent_color))
        .or_else(|| profile?.user_profile.as_ref()?.accent_color)
        .or_else(|| ProfileTheme::of(profile).map(|theme| theme.primary))
        .unwrap_or(theme::SELECTED);
    div()
        .w_full()
        .h(rpx(layout.banner))
        .rounded_t(corner)
        .bg(rgb(color))
        .children(image)
}

fn status_bubble(user_id: u64, text: &str, layout: &Layout) -> impl IntoElement + use<> {
    let bubble = |size: f32| {
        div()
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(SURFACE_HIGHEST))
            .border_1()
            .border_color(rgba(0xFFFFFF1F))
    };
    div()
        .absolute()
        .left(rpx(layout.status_left))
        .top(rpx(layout.banner - 7.))
        .right(rpx(12.))
        .flex()
        .child(
            div()
                .relative()
                .max_w(rpx(181.))
                .min_w(rpx(42.))
                .px(rpx(12.))
                .py(rpx(8.))
                .rounded(rpx(12.))
                .bg(rgb(SURFACE_HIGHEST))
                .border_1()
                .border_color(rgba(0xFFFFFF1F))
                .shadow_sm()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::TEXT))
                .child(
                    div()
                        .line_clamp(2)
                        .child(wrapped_label(("profile-status", user_id), text)),
                )
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.))),
        )
}

fn display_names(
    app: &DiscordApp,
    card: &ProfileCard,
    large: bool,
    on_name: Option<ClickHandler>,
    cx: &App,
) -> impl IntoElement + use<> {
    let profile = card.profile.as_deref();
    let user = app.store.user(card.user_id);
    let guild_id = card.guild_id.unwrap_or(0);
    let name = app
        .store
        .member_name(guild_id, card.user_id)
        .map(str::to_owned)
        .or_else(|| user.map(|user| user.name.clone()))
        .unwrap_or_else(|| "Unknown User".to_owned());
    let username = profile
        .map(|p| p.user.user.username.clone())
        .filter(|username| !username.is_empty());
    let pronouns = profile
        .and_then(guild_profile)
        .and_then(|p| p.pronouns.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| profile?.user_profile.as_ref()?.pronouns.clone())
        .filter(|p| !p.is_empty());
    let name_style = user.and_then(|user| user.name_style.as_ref());
    let dot = || {
        div()
            .size(rpx(4.))
            .mx(rpx(1.))
            .rounded_full()
            .bg(rgb(theme::TEXT))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .child(
            div()
                .id("profile-display-name")
                .map(|this| match on_name {
                    Some(handler) => this.cursor_pointer().on_click(handler),
                    None => this,
                })
                .text_size(rpx(if large { 24. } else { 20. }))
                .line_height(rpx(if large { 30. } else { 24. }))
                .font_weight(if large {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::BOLD
                })
                .child(theme::display_name(&name, name_style, theme::STRONG)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(rpx(6.))
                .gap_y(rpx(if large { 4. } else { 2. }))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::TEXT))
                .children(username.map(|username| div().child(username)))
                .when(user.is_some_and(|user| user.bot), |this| {
                    this.child(theme::app_tag())
                })
                .children(pronouns.map(|pronouns| {
                    div()
                        .flex()
                        .items_center()
                        .gap_x(rpx(6.))
                        .child(dot())
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::STRONG))
                                .child(pronouns),
                        )
                }))
                .children(
                    user.and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                )
                .children(profile.and_then(|profile| badges(profile, cx))),
        )
        .when(!large, |this| this.children(profile.and_then(mutuals_line)))
}

fn mutuals_line(profile: &model::Profile) -> Option<AnyElement> {
    let friends = profile
        .mutual_friends_count
        .filter(|count| *count > 0)
        .map(|count| plural(count as usize, "Mutual Friend"));
    let servers = profile
        .mutual_guilds
        .as_ref()
        .filter(|guilds| !guilds.is_empty())
        .map(|guilds| plural(guilds.len(), "Mutual Server"));
    let parts: Vec<String> = friends.into_iter().chain(servers).collect();
    if parts.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(rpx(6.))
            .py(rpx(4.))
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::MUTED))
            .children(parts.into_iter().enumerate().flat_map(|(index, part)| {
                let spacer = (index > 0).then(|| {
                    div()
                        .size(rpx(4.))
                        .rounded_full()
                        .bg(rgb(theme::MUTED))
                        .into_any_element()
                });
                spacer
                    .into_iter()
                    .chain(std::iter::once(div().child(part).into_any_element()))
            }))
            .into_any_element(),
    )
}

fn badges(profile: &model::Profile, cx: &App) -> Option<AnyElement> {
    if profile.badges.is_empty() {
        return None;
    }
    let shown = if profile.badges.len() > MAX_VISIBLE_BADGES + 1 {
        MAX_VISIBLE_BADGES
    } else {
        profile.badges.len()
    };
    let hidden = profile.badges.len() - shown;
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rpx(2.))
            .w_auto()
            .max_w_full()
            .px(rpx(3.))
            .py(rpx(1.))
            .rounded(rpx(8.))
            .bg(rgba(0xFFFFFF0A))
            .border_1()
            .border_color(rgba(0xFFFFFF14))
            .children(
                profile
                    .badges
                    .iter()
                    .take(shown)
                    .enumerate()
                    .map(|(index, badge)| {
                        let spec = ImageSpec::Badge {
                            icon: badge.icon.clone(),
                        };
                        let tip = SharedString::from(badge.description.clone());
                        let link = badge.link.clone();
                        div()
                            .id(("profile-badge", index))
                            .size(rpx(20.))
                            .when(link.is_some(), |this| this.cursor_pointer())
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(move |_, _, cx| {
                                if let Some(link) = &link {
                                    cx.open_url(link);
                                }
                            })
                            .children(
                                images::resolve(&spec, theme::image_px(20.), cx)
                                    .map(|path| img(path).size_full()),
                            )
                    }),
            )
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .h(rpx(20.))
                        .min_w(rpx(20.))
                        .px(rpx(2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::TEXT))
                        .child(format!("+{hidden}")),
                )
            })
            .into_any_element(),
    )
}

fn popout_body(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    cx: &mut Context<DiscordApp>,
) -> impl IntoElement {
    let user = app.store.user(card.user_id);
    let entity = cx.entity();
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
                entity.update(cx, |app, cx| app.open_full_profile(cx));
            })),
            cx,
        ))
        .children(bio(app, card, true, cx))
        .children(user.and_then(playing))
        .children(roles(app, card))
}

fn bio(app: &DiscordApp, card: &ProfileCard, clamped: bool, cx: &App) -> Option<AnyElement> {
    let profile = card.profile.as_deref()?;
    let text = guild_profile(profile)
        .and_then(|p| p.bio.clone())
        .filter(|bio| !bio.trim().is_empty())
        .or_else(|| profile.user_profile.as_ref()?.bio.clone())
        .filter(|bio| !bio.trim().is_empty())?;
    let parsed = app.parsed(card.user_id, &text);
    let context = BodyContext {
        store: &app.store,
        guild_id: card.guild_id.unwrap_or(0),
        message_id: card.user_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
    };
    Some(
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(theme::STRONG))
            .when(clamped, |this| {
                this.max_h(rpx(18. * BIO_LINES as f32)).overflow_hidden()
            })
            .child(render_body(&parsed.blocks, false, false, &context, cx))
            .into_any_element(),
    )
}

fn playing(user: &User) -> Option<AnyElement> {
    let activity = user.activity.as_ref().filter(|_| user.playing)?;
    Some(section(
        "Playing",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgba(0xFFFFFF0A))
            .border_1()
            .border_color(rgba(0xFFFFFF14))
            .child(theme::icon("gamepad-2", 20., theme::GREEN))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::STRONG))
                    .child(label(("profile-playing", user.id), activity)),
            )
            .into_any_element(),
    ))
}

fn roles(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    let roles = app.store.member_roles(card.guild_id?, card.user_id);
    if roles.is_empty() {
        return None;
    }
    Some(section(
        "Roles",
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(4.))
            .children(roles.into_iter().map(|role| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .min_h(rpx(24.))
                    .pl(rpx(6.))
                    .pr(rpx(8.))
                    .rounded(rpx(8.))
                    .border_1()
                    .border_color(rgba(0xFFFFFF1F))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::TEXT))
                    .child(
                        div()
                            .size(rpx(12.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(role.color.unwrap_or(theme::MUTED))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(label(("profile-role", role.id), &role.name)),
                    )
            }))
            .into_any_element(),
    ))
}

fn section(title: &'static str, body: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::MUTED))
                .child(title),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::TEXT))
                .child(body),
        )
        .into_any_element()
}

fn member_since(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let format = |date: chrono::NaiveDate| date.format("%b %-d, %Y").to_string();
    let created = (card.user_id >> 22) as i64 + DISCORD_EPOCH_MS as i64;
    let created = chrono::DateTime::from_timestamp_millis(created).map(|date| date.date_naive());
    let joined = card
        .profile
        .as_deref()
        .and_then(|p| p.guild_member.as_ref()?.joined_at.as_deref())
        .and_then(|joined| chrono::DateTime::parse_from_rfc3339(joined).ok())
        .map(|date| date.date_naive());
    let guild = card.guild_id.and_then(|id| app.store.guild_by_id(id));
    section(
        "Member Since",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .children(created.map(|date| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .child(theme::icon("discord", 16., theme::MUTED))
                    .child(format(date))
            }))
            .children(joined.zip(guild).map(|(date, guild)| {
                let icon = guild
                    .icon_spec()
                    .and_then(|spec| images::resolve(&spec, theme::image_px(16.), cx));
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .child(div().size(rpx(4.)).rounded_full().bg(rgb(theme::MUTED)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(4.))
                            .child(match icon {
                                Some(path) => {
                                    img(path).size(rpx(16.)).rounded(rpx(4.)).into_any_element()
                                }
                                None => div()
                                    .size(rpx(16.))
                                    .rounded(rpx(4.))
                                    .bg(rgb(guild.icon_color))
                                    .into_any_element(),
                            })
                            .child(format(date)),
                    )
            }))
            .into_any_element(),
    )
}

fn connections(profile: &model::Profile) -> Option<AnyElement> {
    if profile.connected_accounts.is_empty() {
        return None;
    }
    Some(section(
        "Connections",
        div()
            .flex()
            .flex_col()
            .gap(rpx(16.))
            .children(profile.connected_accounts.iter().map(|account| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .child(
                        div()
                            .size(rpx(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rpx(8.))
                            .bg(rgb(0xFFFFFF))
                            .text_size(rpx(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0x2E3035))
                            .child(connection_initial(&account.kind)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(4.))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(rgb(theme::STRONG))
                                            .child(account.name.clone()),
                                    )
                                    .when(account.verified, |this| {
                                        this.child(theme::icon("badge-check", 14., theme::MUTED))
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(theme::MUTED))
                                    .child(connection_name(&account.kind)),
                            ),
                    )
            }))
            .into_any_element(),
    ))
}

fn connection_initial(kind: &str) -> String {
    connection_name(kind)
        .chars()
        .next()
        .unwrap_or('?')
        .to_string()
}

fn modal(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    cx: &mut Context<DiscordApp>,
) -> AnyElement {
    let theme = ProfileTheme::of(card.profile.as_deref());
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let (viewport, viewport_height) = (css(viewport.width), css(viewport.height));
    let windowed = viewport >= MODAL_MIN_VIEWPORT_WIDTH;
    let modal_width = if windowed {
        MODAL_WIDTH.min(viewport - MODAL_MARGIN * 2.)
    } else {
        viewport
    };
    let modal_height = if viewport_height >= MODAL_MIN_VIEWPORT_HEIGHT {
        MODAL_HEIGHT.min(viewport_height - MODAL_MARGIN * 2.)
    } else {
        viewport_height
    };
    let card_inset = if windowed { MODAL_MARGIN * 0.91 } else { 0. };
    let card_width = (viewport * 0.455 - card_inset).clamp(300., 400.);
    let layout = Layout::modal(card_width);
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let background = theme.map_or(rgb(theme::BG).into(), ProfileTheme::modal_background);
    let profile_card = div()
        .relative()
        .w(rpx(card_width))
        .h_full()
        .flex()
        .flex_col()
        .rounded_t(rpx(MODAL_CARD_RADIUS))
        .overflow_hidden()
        .map(|this| match theme {
            Some(theme) => this.bg(theme.outer()),
            None => this.bg(rgb(theme::ELEVATED)),
        })
        .border_1()
        .border_color(rgba(0xFFFFFF14))
        .child(
            div()
                .id("profile-modal-card")
                .flex()
                .flex_col()
                .size_full()
                .rounded_t(rpx(layout.corner))
                .when(theme.is_some(), |this| {
                    this.bg(hsla(0., 0., 0., THEME_OVERLAY))
                })
                .child(header(app, card, &layout, surface, None, cx))
                .child(
                    div()
                        .id("profile-modal-body")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(rpx(24.))
                        .px(rpx(layout.padding))
                        .pb(rpx(layout.padding))
                        .overflow_y_scroll()
                        .child(display_names(app, card, true, None, cx))
                        .children(bio(app, card, false, cx).map(|bio| section("About Me", bio)))
                        .child(member_since(app, card, cx))
                        .children(roles(app, card))
                        .children(card.profile.as_deref().and_then(connections)),
                ),
        )
        .children(effect_layers(card, card_width, cx))
        .into_any_element();
    let framed = framed(card, card_width, profile_card, true, cx);
    let banner_backdrop = card
        .profile
        .as_deref()
        .and_then(|p| p.user.banner.clone())
        .map(|hash| ImageSpec::Banner {
            user_id: card.user_id,
            hash,
        })
        .and_then(|spec| {
            let sigma = BACKDROP_BLUR * BACKDROP_SOURCE as f32 / modal_width;
            images::blurred_with(&spec, BACKDROP_SOURCE, sigma, BACKDROP_SATURATION, cx)
        })
        .map(|image| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(rpx(BACKDROP_HEIGHT))
                .overflow_hidden()
                .child(
                    img(ImageSource::Render(image))
                        .w(rpx(modal_width))
                        .rounded_t(rpx(MODAL_RADIUS))
                        .opacity(BACKDROP_OPACITY),
                )
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(background.opacity(0.), 0.),
                    linear_color_stop(background, 1.),
                )))
        });
    let close = cx.entity();
    let dismiss = cx.entity();
    let dialog = div()
        .id("profile-modal")
        .relative()
        .w(rpx(modal_width))
        .h(rpx(modal_height))
        .rounded(rpx(MODAL_RADIUS))
        .overflow_hidden()
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .bg(background)
        .children(banner_backdrop)
        .child(
            div().absolute().top(rpx(8.)).right(rpx(8.)).child(
                theme::icon_button("profile-close", "x", 18., theme::TEXT)
                    .bg(rgba(0x00000066))
                    .on_click(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_profile(cx));
                    }),
            ),
        )
        .child(
            div()
                .size_full()
                .flex()
                .pt(rpx(32.))
                .pl(rpx(32.))
                .pr(rpx(16.))
                .child(framed)
                .child(side_panel(app, card, cx)),
        );
    div()
        .id("profile-modal-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            dismiss.update(cx, |app, cx| app.close_profile(cx));
        })
        .with_animation(
            ("profile-modal-backdrop", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.bg(hsla(0., 0., 0., 0.7 * delta)),
        )
        .child(dialog.with_animation(
            ("profile-modal-enter", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.opacity(delta).top(rpx(16. * (1. - delta))),
        ))
        .into_any_element()
}

fn side_panel(app: &DiscordApp, card: &ProfileCard, cx: &mut Context<DiscordApp>) -> AnyElement {
    let profile = card.profile.as_deref();
    let mutual_guilds: Vec<&model::MutualGuild> = profile
        .and_then(|p| p.mutual_guilds.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let friends = profile.and_then(|p| p.mutual_friends_count);
    let tab = |id: Tab, title: String| {
        let entity = cx.entity();
        let selected = card.tab == id;
        div()
            .id(SharedString::from(format!("profile-tab-{}", id as u8)))
            .pb(rpx(12.))
            .border_b_2()
            .border_color(if selected {
                rgb(theme::STRONG)
            } else {
                rgba(0)
            })
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if selected {
                theme::STRONG
            } else {
                theme::MUTED
            }))
            .hover(|style| style.text_color(rgb(theme::STRONG)))
            .on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.select_profile_tab(id, cx));
            })
            .child(title)
    };
    let panel = match card.tab {
        Tab::Activity => activity_panel(app, card),
        Tab::Servers => servers_panel(app, &mutual_guilds, cx),
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .pl(rpx(32.))
        .pt(rpx(20.))
        .child(
            div()
                .flex()
                .items_end()
                .gap(rpx(24.))
                .border_b_1()
                .border_color(rgba(0xFFFFFF14))
                .child(tab(Tab::Activity, "Activity".to_owned()))
                .when(!mutual_guilds.is_empty(), |this| {
                    this.child(tab(
                        Tab::Servers,
                        plural(mutual_guilds.len(), "Mutual Server"),
                    ))
                })
                .children(friends.filter(|count| *count > 0).map(|count| {
                    div()
                        .pb(rpx(12.))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::FAINT))
                        .child(plural(count as usize, "Mutual Friend"))
                })),
        )
        .child(
            div()
                .id("profile-tab-panel")
                .flex_1()
                .pt(rpx(20.))
                .pb(rpx(32.))
                .pr(rpx(16.))
                .overflow_y_scroll()
                .child(panel),
        )
        .into_any_element()
}

fn activity_panel(app: &DiscordApp, card: &ProfileCard) -> AnyElement {
    let user = app.store.user(card.user_id);
    if let Some(playing) = user.and_then(playing) {
        return playing;
    }
    let name = user.map_or("This user", |user| user.name.as_str());
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .p(rpx(24.))
        .rounded(rpx(12.))
        .bg(rgba(0xFFFFFF08))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::STRONG))
                .child(format!("{name} doesn't have any activity to share here")),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child("Looks like this profile is still a work in progress."),
        )
        .into_any_element()
}

fn servers_panel(app: &DiscordApp, mutuals: &[&model::MutualGuild], cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .children(mutuals.iter().filter_map(|mutual| {
            let guild = app.store.guild_by_id(mutual.id)?;
            let icon = guild
                .icon_spec()
                .and_then(|spec| images::resolve(&spec, theme::image_px(40.), cx));
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .p(rpx(8.))
                    .rounded(rpx(8.))
                    .hover(|style| style.bg(rgba(0xFFFFFF0A)))
                    .child(match icon {
                        Some(path) => img(path)
                            .size(rpx(40.))
                            .rounded(rpx(12.))
                            .into_any_element(),
                        None => div()
                            .size(rpx(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rpx(12.))
                            .bg(rgb(guild.icon_color))
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::BG))
                            .child(guild.initials.clone())
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::STRONG))
                                    .child(label(("mutual-guild", guild.id), &guild.name)),
                            )
                            .children(mutual.nick.as_ref().map(|nick| {
                                div()
                                    .truncate()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(theme::MUTED))
                                    .child(label(("mutual-nick", guild.id), nick))
                            })),
                    ),
            )
        }))
        .into_any_element()
}

fn framed(card: &ProfileCard, width: f32, body: AnyElement, fill: bool, cx: &App) -> AnyElement {
    let frame = card.frame.as_ref().and_then(|product| {
        let item = product.items.iter().find(|item| item.kind == 3)?;
        Some((product.sku_id, item))
    });
    let layers = |front: bool| -> Vec<AnyElement> {
        let Some((sku, item)) = frame else {
            return Vec::new();
        };
        let scale = width / item.inner_width.unwrap_or(1200).max(1) as f32;
        let overflow_x = item.overflow_horizontal.unwrap_or(0) as f32 * scale;
        let overflow_top = item.overflow_top.unwrap_or(0) as f32 * scale;
        let overflow_bottom = item.overflow_bottom.unwrap_or(0) as f32 * scale;
        let frame_width = width + overflow_x * 2.;
        item.layers
            .iter()
            .filter(|layer| (layer.order == "front") == front)
            .filter_map(|layer| {
                let spec = ImageSpec::Collectible {
                    url: profile_frame_layer_url(sku, layer.id),
                };
                let path = images::resolve(&spec, theme::image_px(frame_width), cx)?;
                let image = img(path).w(rpx(frame_width)).flex_none();
                let base = div().absolute().left(rpx(-overflow_x)).w(rpx(frame_width));
                let base = match (layer.kind.as_str(), layer.anchor.as_str()) {
                    ("rail", anchor) => base
                        .top_0()
                        .bottom_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .map(|rail| match anchor {
                            "top" => rail.justify_start(),
                            "bottom" => rail.justify_end(),
                            _ => rail.justify_center(),
                        }),
                    (_, "bottom") => base.bottom(rpx(-overflow_bottom)),
                    _ => base.top(rpx(-overflow_top)),
                };
                Some(base.child(image).into_any_element())
            })
            .collect()
    };
    div()
        .relative()
        .when(fill, |this| this.h_full())
        .children(layers(false))
        .child(body)
        .children(layers(true))
        .into_any_element()
}

fn connection_name(kind: &str) -> String {
    match kind {
        "github" => "GitHub".into(),
        "twitch" => "Twitch".into(),
        "youtube" => "YouTube".into(),
        "steam" => "Steam".into(),
        "spotify" => "Spotify".into(),
        "xbox" => "Xbox".into(),
        "playstation" => "PlayStation".into(),
        "reddit" => "Reddit".into(),
        "twitter" => "X".into(),
        "domain" => "Domain".into(),
        "battlenet" => "Battle.net".into(),
        "leagueoflegends" => "League".into(),
        "tiktok" => "TikTok".into(),
        "bluesky" => "Bluesky".into(),
        "instagram" => "Instagram".into(),
        "facebook" => "Facebook".into(),
        "epicgames" => "Epic Games".into(),
        "riotgames" => "Riot".into(),
        "roblox" => "Roblox".into(),
        "paypal" => "PayPal".into(),
        "ebay" => "eBay".into(),
        "crunchyroll" => "Crunchyroll".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn effect_layers(card: &ProfileCard, width: f32, cx: &App) -> Vec<AnyElement> {
    let Some(item) = card
        .effect
        .as_ref()
        .and_then(|product| product.items.iter().find(|item| item.kind == 1))
    else {
        return Vec::new();
    };
    let elapsed = card.opened.elapsed().as_millis() as u64;
    let mut layers: Vec<_> = item
        .effects
        .iter()
        .enumerate()
        .filter_map(|(index, layer)| {
            let start = u64::from(layer.start);
            let duration = u64::from(layer.duration).max(1);
            let since = elapsed.checked_sub(start)?;
            let replay = if layer.looped {
                let period = duration + u64::from(layer.loop_delay);
                (since % period < duration).then_some(since / period)?
            } else {
                (since < duration).then_some(0)?
            };
            Some((layer.z_index, index, replay, &layer.src))
        })
        .collect();
    layers.sort_by_key(|(z, ..)| *z);
    let height = width * EFFECT_HEIGHT_RATIO;
    layers
        .into_iter()
        .filter_map(|(_, index, replay, src)| {
            let spec = ImageSpec::Collectible { url: src.clone() };
            let image = animated::resolve(&spec, theme::image_px(height), cx)?;
            Some(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .child(
                        animated::image(image)
                            .id(SharedString::from(format!(
                                "profile-effect-{index}-{replay}"
                            )))
                            .w(rpx(width)),
                    )
                    .into_any_element(),
            )
        })
        .collect()
}
