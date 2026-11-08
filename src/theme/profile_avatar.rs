use std::{
    path::Path as FilePath,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Bounds, ElementId, FillOptions, FillRule, Hsla, ImageAlphaMask, ImgResourceLoader,
    ObjectFit, PathBuilder, PathStyle, Pixels, Point, RenderImage, Resource, canvas, div, point,
    prelude::*, px, rgb, size,
};

use super::{DECORATION_SCALE, rpx, status_color};
use crate::store::Status;

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    avatar: f32,
    icon: f32,
    backing: f32,
    center: f32,
}

impl Geometry {
    pub(super) fn for_avatar(avatar: f32, with_status: bool) -> Option<Self> {
        (with_status && avatar >= 80.).then_some(Self {
            avatar,
            icon: avatar / 5.,
            backing: avatar / 3.,
            center: avatar * 5. / 6.,
        })
    }

    fn mask(self, bounds: Bounds<Pixels>, decoration: bool, scale_factor: f32) -> ImageAlphaMask {
        let width = self.avatar * if decoration { DECORATION_SCALE } else { 1. };
        let offset = (self.avatar - width) / 2.;
        let scale = bounds.size.width / width;
        let radius = scale * (self.backing / 2.);
        let center = bounds.origin
            + point(
                scale * (self.center - offset),
                scale * (self.center - offset),
            );
        ImageAlphaMask {
            bounds: Bounds::new(
                center - point(radius, radius),
                size(radius * 2., radius * 2.),
            ),
            radius,
            feather: px(1. / scale_factor),
            clearance: px(0.),
            bottom_fade: None,
        }
    }
}

pub(super) enum Source {
    Decoded(Arc<RenderImage>),
    File(Arc<FilePath>),
}

struct Playback {
    image: Arc<RenderImage>,
    frame_ends: Vec<Duration>,
    elapsed: Duration,
    last_frame: Option<Instant>,
    frame: usize,
}

impl Playback {
    fn new(image: Arc<RenderImage>) -> Self {
        let mut end = Duration::ZERO;
        let frame_ends = (0..image.frame_count())
            .map(|index| {
                end += Duration::from(image.delay(index)).max(Duration::from_millis(1));
                end
            })
            .collect();
        Self {
            image,
            frame_ends,
            elapsed: Duration::ZERO,
            last_frame: None,
            frame: 0,
        }
    }

    fn advance(&mut self, now: Instant, playing: bool) -> usize {
        if !playing || self.frame_ends.len() <= 1 {
            self.last_frame = None;
            return self.frame;
        }
        if let Some(last) = self.last_frame {
            let duration = self.frame_ends.last().copied().unwrap_or_default();
            let nanos = (self.elapsed + now.saturating_duration_since(last)).as_nanos()
                % duration.as_nanos();
            self.elapsed = Duration::new(
                (nanos / 1_000_000_000) as u64,
                (nanos % 1_000_000_000) as u32,
            );
            self.frame = self.frame_ends.partition_point(|end| *end <= self.elapsed);
        }
        self.last_frame = Some(now);
        self.frame
    }
}

pub(super) fn image(
    id: impl Into<ElementId>,
    source: Source,
    geometry: Geometry,
    decoration: bool,
) -> AnyElement {
    div()
        .id(id)
        .size_full()
        .child(
            canvas(
                move |_, window, cx| {
                    let image = match source {
                        Source::Decoded(image) => Some(image),
                        Source::File(path) => window
                            .use_asset::<ImgResourceLoader>(&Resource::Path(path), cx)
                            .and_then(|result| result.ok()),
                    }?;
                    if image.frame_count() == 0 {
                        return None;
                    }
                    window.with_global_id("profile-image-playback".into(), |id, window| {
                        window.with_element_state(id, |state: Option<Playback>, window| {
                            let mut state = state
                                .filter(|state| state.image.id == image.id)
                                .unwrap_or_else(|| Playback::new(image));
                            let playing = window.is_window_active() && !cx.reduce_motion();
                            let frame = state.advance(Instant::now(), playing);
                            if playing && state.frame_ends.len() > 1 {
                                window.request_animation_frame();
                            }
                            (Some((state.image.clone(), frame)), state)
                        })
                    })
                },
                move |bounds, frame, window, _| {
                    let Some((image, frame)) = frame else { return };
                    if bounds.is_empty() {
                        return;
                    }
                    let fitted = ObjectFit::Cover.get_bounds(bounds, image.size(frame));
                    let radius = if decoration {
                        px(0.)
                    } else {
                        bounds.size.width / 2.
                    };
                    let mask = geometry.mask(bounds, decoration, window.scale_factor());
                    if let Err(error) = window.paint_image_fitted_masked(
                        bounds,
                        fitted,
                        radius.into(),
                        image,
                        frame,
                        false,
                        Some(mask),
                    ) {
                        log::warn!("Could not paint profile avatar: {error}");
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

fn status_backdrop(status: Status, surface: u32) -> Hsla {
    let surface: Hsla = rgb(surface).into();
    let light = surface.l >= 0.5;
    let opacity = if light && status == Status::Idle {
        0.7
    } else {
        0.45
    };
    gpui::hsla(0., 0., if light { 1. } else { 0. }, opacity)
}

pub(super) fn indicator(geometry: Geometry, status: Status, surface: u32) -> AnyElement {
    let edge = geometry.center - geometry.backing / 2.;
    div()
        .absolute()
        .left(rpx(edge))
        .top(rpx(edge))
        .size(rpx(geometry.backing))
        .rounded_full()
        .bg(status_backdrop(status, surface))
        .flex()
        .items_center()
        .justify_center()
        .child(status_icon(status, geometry.icon))
        .into_any_element()
}

/// Grows upward from the status notch, since the phone is taller than the dot it replaces.
pub(super) fn mobile_indicator(geometry: Geometry, surface: u32) -> AnyElement {
    let edge = geometry.center - geometry.backing / 2.;
    let inset = (geometry.backing - geometry.icon) / 2.;
    let height = geometry.icon * 1.5 + inset * 2.;
    div()
        .absolute()
        .left(rpx(edge))
        .top(rpx(edge + geometry.backing - height))
        .p(rpx(inset))
        .rounded(rpx(geometry.icon * 0.3))
        .bg(rgb(surface))
        .child(super::mobile_status(geometry.icon, surface))
        .into_any_element()
}

/// Paint the status shape with transparent cutouts so it works over any row surface.
pub fn status_icon(status: Status, size: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if let Ok(path) = status_path(bounds, status).build() {
                window.paint_path(path, rgb(status_color(status)));
            }
        },
    )
    .size(rpx(size))
    .flex_none()
}

fn circle(path: &mut PathBuilder, center: Point<Pixels>, radius: Pixels) {
    path.move_to(center + point(radius, px(0.)));
    path.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        center - point(radius, px(0.)),
    );
    path.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        center + point(radius, px(0.)),
    );
    path.close();
}

fn status_path(bounds: Bounds<Pixels>, status: Status) -> PathBuilder {
    let p = |x, y| bounds.origin + point(bounds.size.width * x, bounds.size.height * y);
    let radius = bounds.size.width / 2.;
    let mut path = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::EvenOdd),
    ));
    if status == Status::Idle {
        return notched_circle(bounds, p(0.25, 0.25), bounds.size.width * 0.375);
    }
    circle(&mut path, p(0.5, 0.5), radius);
    match status {
        Status::Offline => circle(&mut path, p(0.5, 0.5), radius / 2.),
        Status::DoNotDisturb => {
            let cap = radius / 4.;
            path.move_to(p(0.25, 0.375));
            path.line_to(p(0.75, 0.375));
            path.arc_to(point(cap, cap), px(0.), false, true, p(0.75, 0.625));
            path.line_to(p(0.25, 0.625));
            path.arc_to(point(cap, cap), px(0.), false, true, p(0.25, 0.375));
            path.close();
        }
        _ => {}
    }
    path
}

fn notched_circle(
    bounds: Bounds<Pixels>,
    notch: Point<Pixels>,
    notch_radius: Pixels,
) -> PathBuilder {
    let center = bounds.center();
    let radius = bounds.size.width / 2.;
    let dx = f32::from(notch.x - center.x);
    let dy = f32::from(notch.y - center.y);
    let outer_radius = f32::from(radius);
    let inner_radius = f32::from(notch_radius);
    let distance = (dx * dx + dy * dy).sqrt();
    let along = (outer_radius * outer_radius - inner_radius * inner_radius + distance * distance)
        / (2. * distance);
    let across = (outer_radius * outer_radius - along * along).max(0.).sqrt();
    let midpoint = center + point(px(dx * (along / distance)), px(dy * (along / distance)));
    let normal = point(px(-dy * (across / distance)), px(dx * (across / distance)));
    let mut path = PathBuilder::fill();
    path.move_to(midpoint + normal);
    path.arc_to(point(radius, radius), px(0.), true, true, midpoint - normal);
    path.arc_to(
        point(notch_radius, notch_radius),
        px(0.),
        false,
        false,
        midpoint + normal,
    );
    path.close();
    path
}

pub(super) fn background(geometry: Geometry, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if bounds.is_empty() {
                return;
            }
            let mask = geometry.mask(bounds, false, window.scale_factor());
            if let Ok(path) = notched_circle(bounds, mask.bounds.center(), mask.radius).build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{Delay, Frame, RgbaImage};

    #[test]
    fn profile_status_matches_discord_geometry() {
        let geometry = Geometry::for_avatar(120., true).unwrap();
        assert_eq!(
            (geometry.icon, geometry.backing, geometry.center),
            (24., 40., 100.)
        );
        let popout = Geometry::for_avatar(80., true).unwrap();
        assert_eq!(popout.icon, 16.);
        assert!(Geometry::for_avatar(32., true).is_none());
        assert!(Geometry::for_avatar(120., false).is_none());
    }

    #[test]
    fn all_status_shapes_build_at_menu_and_profile_sizes() {
        for diameter in [10., 12.5, 15., 16., 24.] {
            let bounds = Bounds::new(point(px(80.), px(120.)), size(px(diameter), px(diameter)));
            for status in [
                Status::Online,
                Status::Idle,
                Status::DoNotDisturb,
                Status::Offline,
            ] {
                let path = status_path(bounds, status).build().unwrap();
                assert!(!path.bounds.is_empty());
            }
        }
    }

    #[test]
    fn status_backdrop_follows_the_surface_instead_of_always_being_white() {
        for status in [
            Status::Online,
            Status::Idle,
            Status::DoNotDisturb,
            Status::Offline,
        ] {
            assert_eq!(
                status_backdrop(status, 0x30205E),
                gpui::hsla(0., 0., 0., 0.45)
            );
            let light = status_backdrop(status, 0xDBD7E4);
            assert_eq!(light.l, 1.);
            assert_eq!(light.a, if status == Status::Idle { 0.7 } else { 0.45 });
        }
    }

    fn playback() -> Playback {
        let frames: Vec<_> = [50, 100, 150]
            .into_iter()
            .map(|delay| {
                Frame::from_parts(
                    RgbaImage::new(1, 1),
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                )
            })
            .collect();
        Playback::new(Arc::new(RenderImage::new(frames)))
    }

    #[test]
    fn playback_keeps_frame_timing_and_wraps_after_the_full_loop() {
        let mut playback = playback();
        let start = Instant::now();
        for (millis, frame) in [
            (0, 0),
            (49, 0),
            (50, 1),
            (150, 2),
            (299, 2),
            (300, 0),
            (1250, 1),
        ] {
            assert_eq!(
                playback.advance(start + Duration::from_millis(millis), true),
                frame
            );
        }
    }

    #[test]
    fn playback_pauses_without_jumping_when_resumed() {
        let mut playback = playback();
        let start = Instant::now();
        playback.advance(start, true);
        assert_eq!(playback.advance(start + Duration::from_millis(75), true), 1);
        assert_eq!(
            playback.advance(start + Duration::from_millis(100), false),
            1
        );
        assert_eq!(playback.advance(start + Duration::from_secs(10), true), 1);
        assert_eq!(
            playback.advance(start + Duration::from_millis(10075), true),
            2
        );
    }
}
