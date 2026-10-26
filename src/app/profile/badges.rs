use super::*;

#[derive(Clone)]
struct BadgeTip {
    art: Option<ImageSpec>,
    category: Option<String>,
    title: String,
    detail: Option<String>,
}

impl BadgeTip {
    fn of(badge: &model::Badge, detail: &model::BadgeDetail) -> Self {
        let tier = detail.tier();
        let tier_name = tier
            .and_then(|tier| tier.name.clone())
            .filter(|name| !name.is_empty());
        let name = Some(detail.name.clone()).filter(|name| !name.is_empty());
        // Discord titles Nitro tiers "Nitro Opal"; other levelled badges put the badge name above the tier.
        let (category, title) = match tier_name {
            Some(tier) if detail.badge_id == 1 => (
                None,
                name.map_or(tier.clone(), |name| format!("{name} {tier}")),
            ),
            Some(tier) => (name, tier),
            None => (None, name.unwrap_or_else(|| badge.description.clone())),
        };
        let art = [
            tier.and_then(|tier| tier.complex_icon_animated_url.as_deref()),
            detail.complex_icon_animated_url.as_deref(),
            tier.and_then(|tier| tier.simple_icon_raster_url.as_deref()),
            detail.simple_icon_raster_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(badge_art)
        .next()
        .or_else(|| {
            Some(ImageSpec::Badge {
                icon: badge.icon.clone(),
            })
        });
        let detail = detail
            .info_label
            .clone()
            .or_else(|| Some(badge.description.clone()))
            .filter(|text| !text.is_empty() && !text.eq_ignore_ascii_case(&title));
        Self {
            art,
            category,
            title,
            detail,
        }
    }
}

// The SVG variants of badge art can't be decoded, so only the PNG URLs reach here.
fn badge_art(url: &str) -> ImageSpec {
    match url
        .rsplit_once("/badge-icons/")
        .and_then(|(_, file)| file.strip_suffix(".png"))
    {
        Some(icon) => ImageSpec::Badge {
            icon: icon.to_owned(),
        },
        None => ImageSpec::Collectible {
            url: url.to_owned(),
        },
    }
}

fn badge_tooltip(tip: &BadgeTip, cx: &App) -> impl IntoElement {
    let art = tip
        .art
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(BADGE_ART), cx));
    let body = div()
        .flex()
        .flex_col()
        .items_center()
        .min_w(rpx(192.))
        .max_w(rpx(256.))
        .px(rpx(16.))
        .py(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .shadow_lg()
        .text_center()
        .children(art.map(|path| {
            img(path)
                .size(rpx(BADGE_ART))
                .mb(rpx(8.))
                .object_fit(ObjectFit::Contain)
        }))
        .children(tip.category.clone().map(|category| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::MUTED))
                .child(category)
        }))
        .child(
            div()
                .text_size(rpx(18.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .italic()
                .text_color(rgb(theme::STRONG))
                .child(tip.title.to_uppercase()),
        )
        .children(tip.detail.clone().map(|detail| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::MUTED))
                .child(detail)
        }));
    div()
        .absolute()
        .top(rpx(-4.))
        .left(relative(0.5))
        .size_0()
        .child(
            deferred(
                anchored().anchor(Anchor::BottomCenter).child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(body)
                        // Overlaps the border by 1px so the arrow reads as part of the card.
                        .child(
                            theme::icon("tooltip-arrow-down", 12., theme::ELEVATED)
                                .h(rpx(6.))
                                .mt(rpx(-1.)),
                        ),
                ),
            )
            // Above the popout, which is itself deferred at priority 1.
            .with_priority(2),
        )
}

pub(super) fn badges(
    app: &DiscordApp,
    card: &ProfileCard,
    profile: &model::Profile,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let details = card.badge_details.as_deref();
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
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
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
                        let rich = details
                            .and_then(|details| details.detail_for(badge))
                            .map(|detail| BadgeTip::of(badge, detail));
                        // Warm the art before hover; the tooltip only renders what is cached.
                        if let Some(spec) = rich.as_ref().and_then(|tip| tip.art.as_ref()) {
                            images::resolve(spec, theme::image_px(BADGE_ART), cx);
                        }
                        let hovered = card.hovered_badge == Some(index);
                        let link = badge.link.clone();
                        let entity = app.entity.clone();
                        div()
                            .id(("profile-badge", index))
                            .relative()
                            .size(rpx(20.))
                            .when(link.is_some(), |this| this.cursor_pointer())
                            .on_hover(move |hovered, _, cx| {
                                entity
                                    .update(cx, |app, cx| {
                                        app.hover_profile_badge(index, *hovered, cx)
                                    })
                                    .ok();
                            })
                            .when(hovered, |this| match &rich {
                                Some(rich) => this.child(badge_tooltip(rich, cx)),
                                None => this.child(theme::tooltip_above(badge.description.clone())),
                            })
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
                        .text_color(rgb(palette.text))
                        .child(format!("+{hidden}")),
                )
            })
            .into_any_element(),
    )
}
