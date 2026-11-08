use super::wakeups::{Wake, schedule_wakeup, wake};
use super::*;

/// A frame change this close is worth waiting for the compositor instead of a timer.
pub(super) const FRAME_LEAD: Duration = Duration::from_millis(8);

pub struct AnimatedImage {
    image: Arc<RenderImage>,
    animate: bool,
    object_fit: Fit,
    pub(super) started_at: Option<Instant>,
    plays: Option<NonZeroU32>,
    interactivity: Interactivity,
}

#[derive(Clone, Copy)]
enum Fit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl Fit {
    fn of(fit: ObjectFit) -> Self {
        match fit {
            ObjectFit::Fill => Fit::Fill,
            ObjectFit::Contain => Fit::Contain,
            ObjectFit::Cover => Fit::Cover,
            ObjectFit::ScaleDown => Fit::ScaleDown,
            ObjectFit::None => Fit::None,
        }
    }

    fn bounds(
        self,
        bounds: gpui::Bounds<Pixels>,
        size: gpui::Size<gpui::DevicePixels>,
    ) -> gpui::Bounds<Pixels> {
        let fit = match self {
            Fit::Fill => ObjectFit::Fill,
            Fit::Contain => ObjectFit::Contain,
            Fit::Cover => ObjectFit::Cover,
            Fit::ScaleDown => ObjectFit::ScaleDown,
            Fit::None => ObjectFit::None,
        };
        fit.get_bounds(bounds, size)
    }
}

/// An animated [`RenderImage`] that wakes only when its current frame is due.
///
/// `Img` requests an animation frame on every frame while its image has more than one
/// frame, which redraws the whole view at monitor refresh for a 30-100ms GIF. This
/// element picks the frame from elapsed time and schedules one timer per view for the
/// next change.
pub fn image(image: Arc<RenderImage>) -> AnimatedImage {
    AnimatedImage {
        image,
        animate: true,
        object_fit: Fit::Contain,
        started_at: None,
        plays: None,
        interactivity: Interactivity::new(),
    }
}

pub(crate) fn poster_image(frame: Arc<RenderImage>) -> AnimatedImage {
    let mut poster = image(frame);
    poster.animate = false;
    poster
}

impl AnimatedImage {
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = Fit::of(object_fit);
        self
    }

    /// Uses an external timeline; a finite play count holds the final frame when exhausted.
    pub(crate) fn play_from(mut self, started_at: Instant, plays: Option<NonZeroU32>) -> Self {
        self.started_at = Some(started_at);
        self.plays = plays;
        self
    }
}

impl Styled for AnimatedImage {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for AnimatedImage {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for AnimatedImage {}

impl IntoElement for AnimatedImage {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AnimatedImage {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |mut style, window, cx| {
                let size = self.image.size(0);
                let image_size = gpui::size(px(size.width.0 as f32), px(size.height.0 as f32));
                // Match `Img`: intrinsic aspect only when a dimension is auto, otherwise
                // an explicit size wins (Cover crops, it must not resize the element).
                if matches!(style.size.width, Length::Auto)
                    || matches!(style.size.height, Length::Auto)
                {
                    let ratio = if image_size.height > px(0.) {
                        image_size.width / image_size.height
                    } else {
                        1.
                    };
                    style.aspect_ratio = Some(ratio);
                }
                if let Length::Auto = style.size.width {
                    style.size.width = match style.size.height {
                        Length::Definite(DefiniteLength::Absolute(height)) => {
                            let height = height.to_pixels(window.rem_size());
                            Length::Definite(
                                px(f32::from(image_size.width) * f32::from(height)
                                    / f32::from(image_size.height))
                                .into(),
                            )
                        }
                        _ => Length::Definite(image_size.width.into()),
                    };
                }
                if let Length::Auto = style.size.height {
                    style.size.height = match style.size.width {
                        Length::Definite(DefiniteLength::Absolute(width)) => {
                            let width = width.to_pixels(window.rem_size());
                            Length::Definite(
                                px(f32::from(image_size.height) * f32::from(width)
                                    / f32::from(image_size.width))
                                .into(),
                            )
                        }
                        _ => Length::Definite(image_size.height.into()),
                    };
                }
                window.request_layout(style, None::<LayoutId>, cx)
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image = self.image.clone();
        let object_fit = self.object_fit;
        let started_at = self.started_at;
        let plays = self.plays;
        let animate = self.animate;
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                if bounds.intersect(&window.content_mask().bounds).is_empty() {
                    return;
                }
                let clock = started_at.map(|started_at| (started_at, plays));
                let streamed = stream::paint(&image, animate, clock, window, cx);
                let image = streamed.as_ref().unwrap_or(&image);
                let frame = match (started_at, global_id) {
                    _ if !animate => 0,
                    _ if streamed.is_some() => 0,
                    (Some(started_at), _) => frame_index_at(&image, started_at, plays, window, cx),
                    (None, Some(global_id)) => frame_index(&image, global_id, window, cx),
                    // Without an id there is nowhere to keep the clock; frame 0 is the
                    // only stable choice, matching `Img`'s stateless path.
                    (None, None) => 0,
                };
                if image.frame_count() == 0 {
                    return;
                }
                let frame = frame.min(image.frame_count() - 1);
                let fitted = object_fit.bounds(bounds, image.size(frame));
                let visible = bounds.intersect(&fitted);
                let corner_radii = style
                    .corner_radii
                    .to_pixels(window.rem_size())
                    .clamp_radii_for_quad_size(visible.size);
                window
                    .paint_image_fitted(visible, fitted, corner_radii, image.clone(), frame, false)
                    .ok();
            },
        );
    }
}

/// Canvas overlays use the same playback clock and per-view wakeups as animated elements.
pub(crate) fn frame_index(
    image: &Arc<RenderImage>,
    id: &GlobalElementId,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let (frame, wake) = window.with_element_state(id, |state: Option<AnimatedState>, window| {
        let mut state = state
            .filter(|state| state.image.id == image.id)
            .unwrap_or_else(|| AnimatedState {
                image: image.clone(),
                started: cx.background_executor().now(),
                paused_at: None,
                frame: 0,
            });
        let wake = state.advance(image, window, cx);
        ((state.frame, wake), state)
    });
    request_wakeup(wake, window, cx);
    frame
}

fn frame_index_at(
    image: &RenderImage,
    started_at: Instant,
    plays: Option<NonZeroU32>,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let reduced = cx.reduce_motion();
    let elapsed = if reduced {
        Duration::ZERO
    } else {
        cx.background_executor()
            .now()
            .saturating_duration_since(started_at)
    };
    let (frame, remaining) = frame_at(image, elapsed, plays);
    request_wakeup(
        wake(
            remaining,
            image.frame_count() > 1,
            reduced,
            window_active(window),
        ),
        window,
        cx,
    );
    frame
}

pub(super) fn request_wakeup(wake: Wake, window: &mut Window, cx: &mut App) {
    match wake {
        Wake::AnimationFrame => window.request_animation_frame(),
        Wake::Timer(remaining) => {
            let view = window.current_view();
            schedule_wakeup(view, cx, remaining, window);
        }
        Wake::None => {}
    }
}

struct AnimatedState {
    image: Arc<RenderImage>,
    started: Instant,
    paused_at: Option<Instant>,
    frame: usize,
}

// GPUI's test platform always reports its window inactive, so render-count tests
// have to opt in to the active path explicitly.
pub(super) fn window_active(window: &Window) -> bool {
    #[cfg(test)]
    if FORCE_ACTIVE.with(|active| active.get()) {
        return true;
    }
    window.is_window_active()
}

impl AnimatedState {
    fn advance(&mut self, image: &RenderImage, window: &Window, cx: &App) -> Wake {
        let now = cx.background_executor().now();
        let animated = image.frame_count() > 1;
        let reduced = cx.reduce_motion();
        let active = window_active(window);
        let elapsed = if reduced {
            self.started = now;
            self.paused_at = None;
            Duration::ZERO
        } else if !active {
            let paused = *self.paused_at.get_or_insert(now);
            paused.saturating_duration_since(self.started)
        } else {
            if let Some(paused) = self.paused_at.take() {
                self.started += now.saturating_duration_since(paused);
            }
            now.saturating_duration_since(self.started)
        };
        let (frame, remaining) = frame_at(image, elapsed, None);
        self.frame = frame;
        wake(remaining, animated, reduced, active)
    }
}

/// The frame visible at `elapsed` and how long until the next one.
pub(super) fn frame_at(
    image: &RenderImage,
    elapsed: Duration,
    plays: Option<NonZeroU32>,
) -> (usize, Duration) {
    let count = image.frame_count();
    if count == 0 {
        return (0, Duration::ZERO);
    }
    if count == 1 {
        return (0, Duration::MAX);
    }
    let total: Duration = (0..count)
        .map(|index| Duration::from(image.delay(index)))
        .sum();
    let total = total.max(Duration::from_nanos(1));
    if plays.is_some_and(|plays| elapsed.as_nanos() / total.as_nanos() >= u128::from(plays.get())) {
        return (count - 1, Duration::MAX);
    }
    let mut rest = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
    for index in 0..count {
        let delay = Duration::from(image.delay(index));
        if rest < delay {
            return (index, delay - rest);
        }
        rest -= delay;
    }
    (0, Duration::from(image.delay(0)))
}
