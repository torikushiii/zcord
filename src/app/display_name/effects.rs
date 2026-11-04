use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

use gpui::{
    Bounds, ContentMask, Hsla, Pixels, Point, Rgba, Size, TransformationMatrix, UnderlineStyle,
    Window, hsla, point, px, size, white,
};

use super::{Glyph, Layout, Line, Metrics};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    Solid,
    Gradient,
    Neon,
    Toon,
    Pop,
    Prism,
    Gummy,
}

impl Effect {
    pub(super) fn from_id(id: Option<u32>) -> Self {
        match id {
            Some(2 | 6) => Self::Gradient,
            Some(3) => Self::Neon,
            Some(4) => Self::Toon,
            Some(5) => Self::Pop,
            Some(7) => Self::Prism,
            Some(8) => Self::Gummy,
            _ => Self::Solid,
        }
    }

    pub(super) fn animated(self) -> bool {
        !matches!(self, Self::Solid | Self::Gradient)
    }
}

#[derive(Default)]
pub(super) struct Clock {
    elapsed: Duration,
    last: Option<Instant>,
}

impl Clock {
    pub(super) fn advance(&mut self, now: Instant, requested: bool, playing: bool) -> f32 {
        if !requested {
            self.elapsed = Duration::ZERO;
            self.last = None;
        } else if playing {
            if let Some(last) = self.last {
                self.elapsed += now.saturating_duration_since(last);
            }
            self.last = Some(now);
        } else {
            self.last = None;
        }
        self.elapsed.as_secs_f32()
    }
}

const EASE: [f32; 4] = [0.44, 0.29, 0.48, 1.];
const FLICKER_EASE: [f32; 4] = [0.24, 0.31, 0.36, 0.93];

fn bezier(progress: f32, curve: [f32; 4]) -> f32 {
    let at = |t: f32, a: f32, b: f32| {
        3. * (1. - t).powi(2) * t * a + 3. * (1. - t) * t * t * b + t.powi(3)
    };
    let mut low = 0.;
    let mut high = 1.;
    for _ in 0..14 {
        let mid = (low + high) / 2.;
        if at(mid, curve[0], curve[2]) < progress {
            low = mid;
        } else {
            high = mid;
        }
    }
    at((low + high) / 2., curve[1], curve[3])
}

fn sample(frames: &[(f32, f32)], phase: f32, curve: [f32; 4]) -> f32 {
    for pair in frames.windows(2) {
        let [(start, from), (end, to)] = pair else {
            unreachable!()
        };
        if phase <= *end {
            let amount = ((phase - start) / (end - start)).clamp(0., 1.);
            return from + (to - from) * bezier(amount, curve);
        }
    }
    frames.last().map_or(0., |(_, value)| *value)
}

fn gummy_scale(elapsed: f32, cluster: usize) -> Size<f32> {
    let delayed = elapsed - cluster as f32 * 0.05;
    if delayed <= 0. {
        return size(1., 1.);
    }
    let phase = delayed.rem_euclid(4.) / 4.;
    let x = sample(
        &[
            (0., 1.),
            (0.07, 1.28),
            (0.14, 0.82),
            (0.21, 1.1),
            (0.28, 0.96),
            (0.35, 1.),
            (1., 1.),
        ],
        phase,
        EASE,
    );
    size(x, 2. - x)
}

fn shade(mut color: Hsla, lightness: f32) -> Hsla {
    color.l = (color.l * lightness).clamp(0., 1.);
    color
}

fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let from: Rgba = from.into();
    let to: Rgba = to.into();
    Rgba {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: from.a + (to.a - from.a) * amount,
    }
    .into()
}

fn gradient(stops: &[(f32, Hsla)], position: f32) -> Hsla {
    let Some(&(first, color)) = stops.first() else {
        return white();
    };
    if position <= first {
        return color;
    }
    for pair in stops.windows(2) {
        if position <= pair[1].0 {
            return mix(
                pair[0].1,
                pair[1].1,
                (position - pair[0].0) / (pair[1].0 - pair[0].0),
            );
        }
    }
    stops.last().unwrap().1
}

fn glyph(
    glyph: &Glyph,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    transform: TransformationMatrix,
    blur: Pixels,
    window: &mut Window,
) {
    let origin = origin + glyph.origin;
    if glyph.emoji {
        let _ = window.paint_emoji(origin, glyph.font, glyph.id, font_size);
    } else if transform == TransformationMatrix::unit() && blur == px(0.) {
        let _ = window.paint_glyph(origin, glyph.font, glyph.id, font_size, color);
    } else {
        let _ = window.paint_glyph_transformed(
            origin,
            glyph.font,
            glyph.id,
            font_size,
            color,
            transform,
            blur.max(TILE_CLAMP_BLUR),
        );
    }
}

/// A scaled glyph samples the atlas with linear filtering, which bleeds the neighboring glyph's
/// edge in as a faint column. gpui only clamps sampling to the glyph's own tile on its blur
/// path, so a blur this small takes that path without visibly softening anything.
const TILE_CLAMP_BLUR: Pixels = px(0.01);

fn stroke(
    line: &Line,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    radius: Pixels,
    window: &mut Window,
) {
    for item in line.glyphs.iter().filter(|glyph| !glyph.emoji) {
        stroke_glyph(item, origin, font_size, color, radius, window);
    }
}

fn stroke_glyph(
    item: &Glyph,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    radius: Pixels,
    window: &mut Window,
) {
    for index in 0..16 {
        let angle = index as f32 * TAU / 16.;
        glyph(
            item,
            origin + point(radius * angle.cos(), radius * angle.sin()),
            font_size,
            color,
            TransformationMatrix::unit(),
            px(0.),
            window,
        );
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
    Diagonal,
}

fn ink_x(advance: Pixels, font_left: Pixels, font_right: Pixels, pad: Pixels) -> (Pixels, Pixels) {
    let left = font_left - pad;
    let right = font_right + pad;
    if advance <= px(0.) {
        (left, right)
    } else {
        (left.max(-pad), right.min(advance + pad))
    }
}

// GPUI's glyph pipeline has no gradient brush. Mask small fill strips over its cached glyph;
// outlines and color emoji are painted separately, without reshaping or rasterizing text per frame.
fn gradient_line(
    line: &Line,
    origin: Point<Pixels>,
    metrics: &Metrics,
    axis: Axis,
    color_at: impl Fn(Point<Pixels>) -> Hsla,
    window: &mut Window,
) {
    let step = match axis {
        Axis::Diagonal => (metrics.size / 8.).clamp(px(1.5), px(2.5)),
        Axis::Horizontal | Axis::Vertical => (metrics.size / 12.).clamp(px(1.), px(2.)),
    };
    let mask = window.content_mask().bounds;
    for item in &line.glyphs {
        if item.emoji {
            glyph(
                item,
                origin,
                metrics.size,
                white(),
                TransformationMatrix::unit(),
                px(0.),
                window,
            );
            continue;
        }
        let font_bounds = window.text_system().bounding_box(item.font, metrics.size);
        // Font boxes use different Y conventions across platform backends. Baseline metrics
        // are normalized, so use those for the fill mask rather than the font box's origin.y.
        let ascent = window.text_system().ascent(item.font, metrics.size);
        let descent = window.text_system().descent(item.font, metrics.size).abs();
        let pad = metrics.size * 0.25;
        let (left, right) = ink_x(item.advance, font_bounds.left(), font_bounds.right(), pad);
        let ink = Bounds::new(
            origin + item.origin + point(left, -ascent - pad),
            size(right - left, ascent + descent + pad * 2.),
        )
        .intersect(&mask);
        if ink.is_empty() {
            continue;
        }
        let dx = if matches!(axis, Axis::Vertical) {
            ink.size.width
        } else {
            step
        };
        let dy = if matches!(axis, Axis::Horizontal) {
            ink.size.height
        } else {
            step
        };
        let mut y = ink.top();
        while y < ink.bottom() {
            let height = dy.min(ink.bottom() - y);
            let mut x = ink.left();
            while x < ink.right() {
                let width = dx.min(ink.right() - x);
                let strip = Bounds::new(point(x, y), size(width, height));
                let color = color_at(strip.center() - origin);
                window.with_content_mask(Some(ContentMask { bounds: strip }), |window| {
                    glyph(
                        item,
                        origin,
                        metrics.size,
                        color,
                        TransformationMatrix::unit(),
                        px(0.),
                        window,
                    );
                });
                x += dx;
            }
            y += dy;
        }
    }
}

fn underline(
    line: &Line,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    style: Option<UnderlineStyle>,
    window: &mut Window,
) {
    if let Some(mut style) = style {
        style.color = Some(color);
        window.paint_underline(
            origin + point(px(0.), line.baseline + font_size * 0.08),
            line.width,
            &style,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    effect: Option<Effect>,
    colors: &[Rgba],
    elapsed: f32,
    bounds: Bounds<Pixels>,
    metrics: &Metrics,
    layout: &Layout,
    inherited: Hsla,
    underline_style: Option<UnderlineStyle>,
    window: &mut Window,
) {
    let main: Hsla = colors.first().copied().map(Into::into).unwrap_or(inherited);
    // Chat's font-only path must not calculate decorative palettes/keyframes for every author.
    if matches!(effect, None | Some(Effect::Solid)) {
        for line in &layout.lines {
            for item in &line.glyphs {
                glyph(
                    item,
                    bounds.origin,
                    metrics.size,
                    main,
                    TransformationMatrix::unit(),
                    px(0.),
                    window,
                );
            }
            underline(
                line,
                bounds.origin,
                metrics.size,
                main,
                underline_style,
                window,
            );
        }
        return;
    }
    let phase = elapsed.rem_euclid(4.) / 4.;
    let unit = crate::theme::rpx(1.).to_pixels(window.rem_size());
    let toon_position = sample(&[(0., 0.), (0.05, 0.), (0.55, 1.), (1., 1.)], phase, EASE);
    let pop_y = sample(
        &[(0., 0.), (0.18, -0.05), (0.35, 0.08), (0.5, 0.), (1., 0.)],
        phase,
        EASE,
    );
    let pop_shadow = sample(
        &[
            (0., 0.08),
            (0.18, 0.13),
            (0.35, 0.),
            (0.5, 0.08),
            (1., 0.08),
        ],
        phase,
        EASE,
    );
    let pop_position = sample(&[(0., 0.), (0.5, 1.), (1., 1.)], phase, EASE);
    let width = layout.size.width.max(px(1.));
    let height = layout.size.height.max(px(1.));
    let mut color_stops: Vec<(f32, Hsla)> = colors
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let t = index as f32 / colors.len().saturating_sub(1).max(1) as f32;
            (
                if effect == Some(Effect::Gradient) {
                    0.1 + t * 0.8
                } else {
                    t
                },
                (*color).into(),
            )
        })
        .collect();
    if effect == Some(Effect::Prism) && colors.len() > 1 && colors.first() != colors.last() {
        let count = colors.len() as f32;
        for (index, stop) in color_stops.iter_mut().enumerate() {
            stop.0 = index as f32 / count;
        }
        color_stops.push((1., main));
    }
    let neon_radius = unit * 4. + metrics.size * 0.12;
    let mut neon_stroke = main;
    neon_stroke.s = 1.;
    neon_stroke.l = (main.l + 0.1).min(0.6);
    let neon_flicker = sample(
        &[
            (0., 0.),
            (0.15, 0.),
            (0.16, 1.),
            (0.18, 0.),
            (0.20, 0.),
            (0.22, 1.),
            (0.23, 0.),
            (0.25, 0.),
            (0.28, 1.),
            (0.50, 0.),
            (0.51, 0.),
            (1., 0.),
        ],
        phase,
        FLICKER_EASE,
    );
    let tint = hsla(main.h, (main.s * 1.1 + 0.1).min(1.), 0.85, 1.);
    for line in &layout.lines {
        match effect {
            Some(Effect::Gradient) => gradient_line(
                line,
                bounds.origin,
                metrics,
                Axis::Diagonal,
                |point| gradient(&color_stops, (point.x / width + point.y / height) / 2.),
                window,
            ),
            Some(Effect::Prism) => {
                let cycle = (layout.ch_width * 5.).max(unit * 38.);
                gradient_line(
                    line,
                    bounds.origin,
                    metrics,
                    Axis::Horizontal,
                    |point| {
                        gradient(
                            &color_stops,
                            (point.x / cycle + elapsed / 2.).rem_euclid(1.),
                        )
                    },
                    window,
                );
            }
            Some(Effect::Toon) => {
                stroke(
                    line,
                    bounds.origin,
                    metrics.size,
                    shade(main, 0.4),
                    (unit * 1.6 + metrics.size * 0.04) / 2.,
                    window,
                );
                let stops = [
                    (0., white()),
                    (0.08, shade(main, 1.6)),
                    (0.15, shade(main, 1.2)),
                    (0.25, main),
                    (0.45, shade(main, 1.6)),
                    (0.55, main),
                    (0.75, white()),
                    (0.83, shade(main, 1.6)),
                    (0.9, shade(main, 1.2)),
                    (1., main),
                ];
                gradient_line(
                    line,
                    bounds.origin,
                    metrics,
                    Axis::Vertical,
                    |point| gradient(&stops, (point.y / height + 3. * toon_position) / 4.),
                    window,
                );
            }
            Some(Effect::Pop) => {
                let shadow_origin =
                    bounds.origin + point(px(0.), metrics.size * (pop_y + pop_shadow));
                let stops = [
                    (0., shade(main, 1.2)),
                    (0.06, shade(main, 1.2)),
                    (0.2, main),
                    (0.5, main),
                    (0.56, shade(main, 1.2)),
                    (0.7, main),
                    (1., main),
                ];
                let shadow_at = |point: Point<Pixels>| {
                    gradient(
                        &stops,
                        (1. - point.x / width + point.y / height + pop_position) / 4.,
                    )
                };
                let stroke_radius = (unit * 1.2 + metrics.size * 0.04) / 2.;
                // Discord gives the shadow a transparent stroke that its gradient still fills,
                // so it's a stroke fatter than the letters and shows beneath their outline.
                // One color per glyph keeps this to a ring of plain glyphs.
                for item in line.glyphs.iter().filter(|item| !item.emoji) {
                    let color = shadow_at(item.center);
                    stroke_glyph(
                        item,
                        shadow_origin,
                        metrics.size,
                        color,
                        stroke_radius,
                        window,
                    );
                }
                gradient_line(
                    line,
                    shadow_origin,
                    metrics,
                    Axis::Diagonal,
                    shadow_at,
                    window,
                );
                let origin = bounds.origin + point(px(0.), metrics.size * pop_y);
                stroke(
                    line,
                    origin,
                    metrics.size,
                    shade(main, 0.2),
                    stroke_radius,
                    window,
                );
                for item in &line.glyphs {
                    glyph(
                        item,
                        origin,
                        metrics.size,
                        white(),
                        TransformationMatrix::unit(),
                        px(0.),
                        window,
                    );
                }
            }
            Some(Effect::Neon) => {
                for item in line.glyphs.iter().filter(|item| !item.emoji) {
                    glyph(
                        item,
                        bounds.origin,
                        metrics.size,
                        main,
                        TransformationMatrix::unit(),
                        neon_radius / 2.,
                        window,
                    );
                }
                stroke(
                    line,
                    bounds.origin,
                    metrics.size,
                    neon_stroke,
                    (unit + metrics.size * 0.04) / 2.,
                    window,
                );
                for item in &line.glyphs {
                    glyph(
                        item,
                        bounds.origin,
                        metrics.size,
                        mix(white(), tint, neon_flicker),
                        TransformationMatrix::unit(),
                        px(0.),
                        window,
                    );
                }
            }
            Some(Effect::Gummy) => {
                for item in &line.glyphs {
                    let scale = gummy_scale(elapsed, item.cluster);
                    let center = bounds.origin + item.center;
                    let center = center.scale(window.scale_factor());
                    let transform = TransformationMatrix {
                        rotation_scale: [[scale.width, 0.], [0., scale.height]],
                        translation: [
                            center.x.0 * (1. - scale.width),
                            center.y.0 * (1. - scale.height),
                        ],
                    };
                    let color = colors
                        .get(item.cluster % colors.len().max(1))
                        .copied()
                        .map(Into::into)
                        .unwrap_or(main);
                    glyph(
                        item,
                        bounds.origin,
                        metrics.size,
                        color,
                        transform,
                        px(0.),
                        window,
                    );
                }
            }
            None | Some(Effect::Solid) => {
                for item in &line.glyphs {
                    glyph(
                        item,
                        bounds.origin,
                        metrics.size,
                        main,
                        TransformationMatrix::unit(),
                        px(0.),
                        window,
                    );
                }
            }
        }
        underline(
            line,
            bounds.origin,
            metrics.size,
            main,
            underline_style,
            window,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    #[test]
    fn gradient_ink_x_clamps_advances_but_preserves_zero_width_ink() {
        assert_eq!(
            ink_x(px(10.), px(-20.), px(30.), px(2.)),
            (px(-2.), px(12.))
        );
        assert_eq!(ink_x(px(10.), px(1.), px(8.), px(2.)), (px(-1.), px(10.)));
        assert_eq!(
            ink_x(px(0.), px(-20.), px(30.), px(2.)),
            (px(-22.), px(32.))
        );
        assert_eq!(
            ink_x(px(-1.), px(-20.), px(30.), px(2.)),
            (px(-22.), px(32.))
        );
    }

    #[test]
    fn effect_protocol_ids_and_safe_fallbacks() {
        assert_eq!(Effect::from_id(Some(1)), Effect::Solid);
        assert_eq!(Effect::from_id(Some(2)), Effect::Gradient);
        assert_eq!(Effect::from_id(Some(3)), Effect::Neon);
        assert_eq!(Effect::from_id(Some(4)), Effect::Toon);
        assert_eq!(Effect::from_id(Some(5)), Effect::Pop);
        assert_eq!(Effect::from_id(Some(6)), Effect::Gradient);
        assert_eq!(Effect::from_id(Some(7)), Effect::Prism);
        assert_eq!(Effect::from_id(Some(8)), Effect::Gummy);
        assert_eq!(Effect::from_id(None), Effect::Solid);
        assert_eq!(Effect::from_id(Some(u32::MAX)), Effect::Solid);
        assert!(!Effect::Solid.animated());
        assert!(!Effect::Gradient.animated());
    }

    #[test]
    fn gummy_uses_discord_keyframes_and_per_letter_delay() {
        let close = |a: f32, b: f32| assert!((a - b).abs() < 0.001, "{a} != {b}");
        close(gummy_scale(0.28, 0).width, 1.28);
        close(gummy_scale(0.56, 0).width, 0.82);
        close(gummy_scale(0.84, 0).width, 1.1);
        close(gummy_scale(1.12, 0).width, 0.96);
        close(gummy_scale(1.4, 0).width, 1.);
        close(gummy_scale(0.33, 1).width, gummy_scale(0.28, 0).width);
        assert_eq!(gummy_scale(0.01, 10), size(1., 1.));
        close(gummy_scale(4.28, 0).width, gummy_scale(0.28, 0).width);
    }

    #[test]
    fn animation_clock_pauses_when_inactive_and_resets_when_disabled() {
        let now = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.advance(now, true, true), 0.);
        assert_eq!(clock.advance(now + Duration::from_secs(1), true, true), 1.);
        assert_eq!(clock.advance(now + Duration::from_secs(2), true, false), 1.);
        assert_eq!(clock.advance(now + Duration::from_secs(30), true, true), 1.);
        assert_eq!(clock.advance(now + Duration::from_secs(31), true, true), 2.);
        assert_eq!(
            clock.advance(now + Duration::from_secs(32), false, false),
            0.
        );
    }

    #[test]
    fn derived_palette_matches_supplied_discord_sample() {
        let main: Hsla = rgb(0x7935ef).into();
        for (factor, expected) in [
            (1.2, 0x9d6bf3),
            (1.6, 0xe5d7fc),
            (0.6, 0x440da2),
            (0.2, 0x170436),
            (0.4, 0x2d096c),
        ] {
            let actual: Rgba = shade(main, factor).into();
            let expected = rgb(expected);
            assert!((actual.r - expected.r).abs() < 1.5 / 255.);
            assert!((actual.g - expected.g).abs() < 1.5 / 255.);
            assert!((actual.b - expected.b).abs() < 1.5 / 255.);
        }
    }
}
