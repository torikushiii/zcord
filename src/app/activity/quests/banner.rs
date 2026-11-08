use super::card::{ButtonKind, button};
use super::*;

const ONYX_BASE: u32 = 0x1A1A1E;
const BANNER_ART: &str = "https://cdn.discordapp.com/assets/content/fff9144c814ffc3126fdcaf5ec5bc68eff6d09e23ac8af5a3fbe61b05970cda0.png";

pub(super) fn control_button(
    id: &'static str,
    label: &'static str,
    chevron: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgba(MOD_SUBTLE))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .opacity(0.5)
        .cursor(CursorStyle::OperationNotAllowed)
        .child(label)
        .when(chevron, |this| {
            this.child(theme::icon("chevron-down", 16., theme::strong()))
        })
        .child(theme::hover_tooltip(
            SharedString::from(format!("{id}-tip")),
            "Sorting and filters aren't available in zcord yet",
        ))
}

pub(super) fn banner(width: f32, cx: &App) -> Div {
    let art = images::resolve(
        &ImageSpec::Media {
            url: BANNER_ART.to_owned(),
            width: 0,
            height: 0,
            animated: false,
        },
        theme::image_px(width),
        cx,
    );
    div()
        .relative()
        .flex()
        .flex_col()
        .justify_center()
        .mb(rpx(64.))
        .min_h(rpx(if width < 725. { 580. } else { 500. }))
        .p(rpx(32.))
        .rounded(rpx(BANNER_RADIUS))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(rgb(ONYX_BASE))
        .children(art.map(|art| {
            img(art)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(BANNER_RADIUS - 1.))
                .object_fit(ObjectFit::Cover)
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(rpx(600.))
                .rounded_l(rpx(BANNER_RADIUS - 1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(ONYX_BASE), 0.32),
                    linear_color_stop(rgba(ONYX_BASE << 8), 1.),
                )),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .max_w(rpx(540.))
                .child(
                    div()
                        .text_size(rpx(if width < 500. { 32. } else { 44. }))
                        .line_height(rpx(if width < 500. { 32. } else { 44. }))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(white())
                        .child("PLAY, WATCH,")
                        .child("EARN REWARDS"),
                )
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .text_color(white())
                        .opacity(0.8)
                        .child(
                            "Win in-game items, Discord avatar decorations, and more through \
                             Quests.",
                        ),
                )
                .child(div().flex().child(button(
                    "quests-learn-more",
                    0,
                    "Learn More",
                    ButtonKind::Overlay,
                ))),
        )
}

fn hero_url(path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.to_owned()
    } else {
        asset_url(path)
    }
}

pub(super) fn sponsored_banner(
    hero: &QuestHomeHero,
    list: &QuestList,
    width: f32,
    cx: &App,
) -> Div {
    let pixels = theme::image_px(width);
    let radius = rpx(BANNER_RADIUS - 1.);
    let clip = hero
        .hero_video
        .as_deref()
        .filter(|path| !cx.reduce_motion() && path.ends_with(".mp4"))
        .and_then(|path| {
            crate::media::video::resolve(
                &ImageSpec::Video {
                    url: hero_url(path),
                    width: 0,
                    height: 0,
                },
                pixels,
                cx,
            )
        })
        .map(|frames| {
            crate::media::animated::image(frames)
                .size_full()
                .rounded(radius)
                .object_fit(ObjectFit::Cover)
                .id("quest-home-hero-clip")
                .into_any_element()
        });
    let art = clip.or_else(|| {
        hero.hero_image
            .as_deref()
            .and_then(|path| {
                images::resolve(
                    &ImageSpec::Media {
                        url: hero_url(path),
                        width: 0,
                        height: 0,
                        animated: false,
                    },
                    pixels,
                    cx,
                )
            })
            .map(|still| {
                img(still)
                    .size_full()
                    .rounded(radius)
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
    });
    let logo = hero.sponsor_image.as_deref().and_then(|path| {
        images::resolve(
            &ImageSpec::Media {
                url: hero_url(path),
                width: 0,
                height: 0,
                animated: false,
            },
            theme::image_px(132.),
            cx,
        )
    });
    let cta = hero
        .cta
        .clone()
        .filter(|cta| !cta.button_label.is_empty() && !cta.url.is_empty());
    let quest = hero
        .quest_ids
        .iter()
        .find(|id| list.quests.iter().any(|quest| quest.id == **id))
        .copied();
    div()
        .relative()
        .flex()
        .flex_col()
        .mb(rpx(64.))
        .min_h(rpx(if width < 725. { 580. } else { 500. }))
        .p(rpx(32.))
        .rounded(rpx(BANNER_RADIUS))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(rgb(ONYX_BASE))
        .children(art.map(|art| div().absolute().inset_0().child(art)))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(rpx(600.))
                .max_w_full()
                .rounded_l(radius)
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(theme::base_lower()), 0.32),
                    linear_color_stop(rgba(theme::base_lower() << 8), 1.),
                )),
        )
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .justify_between()
                .children(logo.map(|logo| {
                    img(logo)
                        .flex_none()
                        .max_h(rpx(56.))
                        .max_w(rpx(132.))
                        .object_fit(ObjectFit::Contain)
                }))
                .child(
                    div()
                        .ml_auto()
                        .mt(rpx(-8.))
                        .mr(rpx(-8.))
                        .px(rpx(8.))
                        .py(rpx(4.))
                        .rounded(rpx(8.))
                        .bg(rgba(0x0000_0080))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::strong()))
                        .child("Sponsored"),
                ),
        )
        .child(div().flex_1().min_h(rpx(32.)))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .max_w(rpx(540.))
                .child(
                    div()
                        .text_size(rpx(if width < 500. { 32. } else { 44. }))
                        .line_height(rpx(if width < 500. { 32. } else { 44. }))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(white())
                        .child(hero.label_title.to_uppercase()),
                )
                .when(!hero.label_subtitle.is_empty(), |text| {
                    text.child(
                        div()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .text_color(white())
                            .opacity(0.8)
                            .child(hero.label_subtitle.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .children(cta.map(|cta| {
                            let url = cta.url;
                            div()
                                .id("quest-home-hero-cta")
                                .flex()
                                .items_center()
                                .h(rpx(38.))
                                .px(rpx(16.))
                                .rounded(rpx(8.))
                                .bg(white())
                                .hover(|style| style.opacity(0.9))
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(black())
                                .cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                                .child(cta.button_label)
                        }))
                        .children(quest.map(|quest| {
                            div()
                                .id(("quest-home-hero-view", quest))
                                .relative()
                                .flex()
                                .items_center()
                                .h(rpx(38.))
                                .px(rpx(16.))
                                .rounded(rpx(8.))
                                .hover(|style| style.bg(rgba(0xFFFF_FF1F)))
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white())
                                .child("View Quest")
                                .child(theme::hover_tooltip(
                                    ("quest-home-hero-view-tip", quest),
                                    UNAVAILABLE,
                                ))
                        })),
                ),
        )
}
