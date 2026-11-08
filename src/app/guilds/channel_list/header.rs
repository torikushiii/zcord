use super::*;

const BANNER_BLUR_SOURCE: u32 = 128;
const BANNER_BLUR_RADIUS: f32 = 30.;

fn banner_hover_listener(app: &DiscordApp) -> impl Fn(&bool, &mut Window, &mut App) + use<> {
    let entity = app.entity.clone();
    let guild_id = app.selected_guild_id;
    move |hovered, _, cx| {
        entity
            .update(cx, |app, cx| {
                if app.selected_guild_id != guild_id {
                    return;
                }
                if app.channel_rows.banner_hover.set(*hovered, Instant::now()) {
                    cx.notify();
                }
            })
            .ok();
    }
}

pub(super) fn guild_header(
    app: &DiscordApp,
    solidity: f32,
    window: &Window,
    cx: &App,
) -> impl IntoElement + use<> {
    let guild = app.guild();
    let solid = solidity >= 1.;
    let now = Instant::now();
    let glass = app.channel_rows.banner_hover.opacity(now);
    if !solid && app.channel_rows.banner_hover.animating(now) {
        window.request_animation_frame();
    }
    let hover: Hsla = if solid {
        rgba(theme::hover()).into()
    } else {
        rgba(0xFFFFFF14).into()
    };
    div()
        .id(("guild-header", guild.map_or(0, |guild| guild.id)))
        .relative()
        .rounded_tl(rpx(8.))
        .flex()
        .items_center()
        .gap_1()
        .h(rpx(HEADER_HEIGHT))
        .flex_none()
        .px_2()
        .bg(theme::surface(
            theme::Layer::Lower,
            Some(theme::Layer::High),
            theme::base_lower(),
        )
        .opacity(solidity))
        .border_b_1()
        .border_color(rgba(theme::border()).opacity(solidity))
        .when(app.channel_rows.banner.is_some(), |this| {
            this.on_hover(banner_hover_listener(app))
        })
        .when(!solid, |this| {
            this.children(
                app.channel_rows
                    .banner
                    .as_ref()
                    .map(|spec| banner_glass(spec, solidity, glass, cx)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_tl(rpx(8.))
                    .opacity(1. - solidity)
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0x00000099), 0.),
                        linear_color_stop(rgba(0x00000000), 1.),
                    )),
            )
        })
        .child(
            div()
                .id("guild-dropdown")
                .relative()
                .flex()
                .items_center()
                .gap_1()
                .h(rpx(32.))
                .min_w_0()
                .px_2()
                .rounded(rpx(8.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(label(
                            ("guild-header", guild.map_or(0, |guild| guild.id)),
                            guild.map_or("No server", |guild| &guild.name),
                        )),
                )
                .child(theme::icon("chevron-down", 16., theme::strong())),
        )
        .child(div().flex_1())
        .child(
            theme::icon_button_with_hover(
                "invite",
                "user-plus",
                18.,
                if solid {
                    theme::muted()
                } else {
                    theme::strong()
                },
                hover,
            )
            .size(rpx(32.))
            .rounded(rpx(8.)),
        )
}

/// Content masks are rectangular, so rows scrolled under the header would show through the
/// panel's rounded corner. This repaints the corner as the panel draws it: window background,
/// then the 1px border, then the panel background down to the header's own curve.
pub(super) fn corner_cover() -> impl IntoElement {
    const RADIUS: f32 = 8.;
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let radius = rpx(RADIUS).to_pixels(window.rem_size());
            let border = px(1.);
            let outside = bounds.origin;
            let inside = outside + point(border, border);
            let outer = outside + point(radius, radius);
            let layers = [
                (inside + point(radius, radius), radius, theme::base_lower()),
                (
                    outer,
                    radius - border,
                    theme::composite(theme::border(), theme::base_lower()),
                ),
                (outer, radius, theme::base_lowest()),
            ];
            for (center, radius, color) in layers {
                let mut path = PathBuilder::fill();
                path.move_to(outside);
                path.line_to(point(center.x, outside.y));
                for step in 0..=12 {
                    let angle = (3. - step as f32 / 12.) * std::f32::consts::FRAC_PI_2;
                    path.line_to(center + point(radius * angle.cos(), radius * angle.sin()));
                }
                path.line_to(point(outside.x, center.y));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .absolute()
    // Starts at the outer edge of the panel's 1px border.
    .top(px(-1.))
    .left(px(-1.))
    .size(rpx(RADIUS + 1.))
}

fn banner_glass(spec: &ImageSpec, solidity: f32, opacity: f32, cx: &App) -> Div {
    let sigma = BANNER_BLUR_RADIUS * BANNER_BLUR_SOURCE as f32 / WIDTH;
    let image = images::blurred_with(spec, BANNER_BLUR_SOURCE, sigma, 1., cx);
    div()
        .absolute()
        .inset_0()
        .opacity(opacity * (1. - solidity))
        .children(image.map(|image| {
            canvas(
                move |_, _, _| image,
                move |bounds, image, window, _| {
                    if bounds.is_empty() {
                        return;
                    }
                    // Fit the full banner before cropping to the fixed, rounded header.
                    let mut banner = bounds;
                    banner.origin.y -= rpx(solidity * (BANNER_HEIGHT - HEADER_HEIGHT))
                        .to_pixels(window.rem_size());
                    banner.size.height = rpx(BANNER_HEIGHT).to_pixels(window.rem_size());
                    let fitted = ObjectFit::Cover.get_bounds(banner, image.size(0));
                    let corners = Corners {
                        top_left: rpx(8.).to_pixels(window.rem_size()),
                        ..Default::default()
                    };
                    if let Err(error) =
                        window.paint_image_fitted(bounds, fitted, corners, image, 0, false)
                    {
                        log::warn!("Could not paint guild banner glass: {error}");
                    }
                },
            )
            .size_full()
        }))
}

pub(super) fn banner_row(spec: Option<&ImageSpec>, cx: &App) -> impl IntoElement + use<> {
    let pixels = theme::image_px(WIDTH);
    let fill = |image: Img| {
        image
            .size_full()
            .rounded_tl(rpx(8.))
            .object_fit(ObjectFit::Cover)
    };
    let image = spec.and_then(|spec| {
        if let Some(image) = spec
            .animated()
            .and_then(|animated| animated::resolve(&animated, pixels, cx))
        {
            return Some(
                animated::image(image)
                    .size_full()
                    .rounded_tl(rpx(8.))
                    .object_fit(ObjectFit::Cover)
                    .id("guild-banner-animated")
                    .into_any_element(),
            );
        }
        images::resolve(spec, pixels, cx).map(|path| fill(img(path)).into_any_element())
    });
    div()
        .w_full()
        .h(rpx(BANNER_HEIGHT))
        .mb_2()
        .bg(rgb(theme::elevated()))
        .rounded_tl(rpx(8.))
        .children(image)
}
