use super::element::FRAME_LEAD;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Wake {
    AnimationFrame,
    Timer(Duration),
    None,
}

pub(super) fn wake(remaining: Duration, animated: bool, reduced: bool, active: bool) -> Wake {
    if !animated || reduced || !active || remaining == Duration::MAX {
        Wake::None
    } else if remaining <= FRAME_LEAD {
        Wake::AnimationFrame
    } else {
        Wake::Timer(remaining)
    }
}

pub(super) struct Wakeup {
    pub(super) deadline: Instant,
    task: Option<Task<()>>,
}

#[derive(Default)]
pub(super) struct Wakeups {
    pub(super) pending: HashMap<EntityId, Wakeup>,
    /// A fired task must not drop its own handle while it is still running; park it
    /// here and drop it from a later paint, where the executor is not inside it.
    retired: Vec<Task<()>>,
}

pub(super) struct AnimatedWakeups(pub(super) Rc<RefCell<Wakeups>>);
impl Global for AnimatedWakeups {}

impl Wakeups {
    pub(super) fn claim(&mut self, view: EntityId, deadline: Instant) -> bool {
        self.retired.clear();
        if self
            .pending
            .get(&view)
            .is_some_and(|wakeup| wakeup.deadline <= deadline)
        {
            return false;
        }
        self.pending.insert(
            view,
            Wakeup {
                deadline,
                task: None,
            },
        );
        true
    }

    fn attach(&mut self, view: EntityId, deadline: Instant, task: Task<()>) {
        if let Some(wakeup) = self.pending.get_mut(&view)
            && wakeup.deadline == deadline
        {
            wakeup.task = Some(task);
        }
    }

    pub(super) fn release(&mut self, view: EntityId, deadline: Instant) -> bool {
        let fired = self
            .pending
            .get(&view)
            .is_some_and(|wakeup| wakeup.deadline == deadline);
        if fired
            && let Some(wakeup) = self.pending.remove(&view)
            && let Some(task) = wakeup.task
        {
            self.retired.push(task);
        }
        fired
    }
}

pub(super) fn schedule_wakeup(view: EntityId, cx: &mut App, remaining: Duration, window: &Window) {
    let Some(clocks) = cx.try_global::<AnimatedWakeups>().map(|c| c.0.clone()) else {
        return;
    };
    let deadline = cx.background_executor().now() + remaining;
    if !clocks.borrow_mut().claim(view, deadline) {
        return;
    }
    let task = {
        let clocks = clocks.clone();
        window.spawn(cx, async move |cx| {
            cx.background_executor().timer(remaining).await;
            if clocks.borrow_mut().release(view, deadline) {
                cx.update(move |_, cx| cx.notify(view)).ok();
            }
        })
    };
    clocks.borrow_mut().attach(view, deadline, task);
}

pub(super) fn animations_enabled(spec: &ImageSpec) -> bool {
    match spec {
        ImageSpec::Emoji { .. } => crate::settings::emoji_animation(),
        ImageSpec::Sticker { .. } => {
            crate::settings::display().animate_stickers != StickerAnimation::Never
        }
        ImageSpec::Media { .. } | ImageSpec::Attachment { .. } | ImageSpec::LocalGif { .. } => {
            crate::settings::gif_autoplay() && crate::settings::window_focused()
        }
        ImageSpec::Animated { still } => animations_enabled(still),
        _ => true,
    }
}
