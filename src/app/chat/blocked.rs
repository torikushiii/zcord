use super::message::{render_message, render_pane_message};
use super::*;
use crate::store::RelationshipKind;

const BAR_HEIGHT: f32 = 44.;

pub(super) struct BlockedRun {
    pub(super) start: usize,
    pub(super) len: usize,
    pub(super) revealed: bool,
}

impl BlockedRun {
    pub(super) fn is_last(&self, index: usize) -> bool {
        index + 1 == self.start + self.len
    }
}

impl DiscordApp {
    fn toggle_blocked_run(&mut self, first_id: u64, cx: &mut Context<Self>) {
        if !self.revealed_blocked_run_starts.remove(&first_id) {
            self.revealed_blocked_run_starts.insert(first_id);
        }
        self.remeasure_message_lists();
        cx.notify();
    }

    pub(in crate::app) fn remeasure_message_lists(&self) {
        self.messages.remeasure();
        if let Some(pane) = &self.thread_pane {
            pane.remeasure();
        }
    }
}

fn blocked(app: &DiscordApp, message: &Message) -> bool {
    message.author_id != app.store.current_user_id
        && app
            .store
            .relationship(message.author_id)
            .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
}

pub(super) fn blocked_run(
    app: &DiscordApp,
    messages: &[Message],
    index: usize,
) -> Option<BlockedRun> {
    if !blocked(app, messages.get(index)?) {
        return None;
    }
    let joins = |earlier: &Message, later: &Message| {
        blocked(app, earlier) && earlier.timestamp.same_day(later.timestamp)
    };
    let mut start = index;
    while start > 0 && joins(&messages[start - 1], &messages[start]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < messages.len()
        && blocked(app, &messages[end])
        && joins(&messages[end - 1], &messages[end])
    {
        end += 1;
    }
    Some(BlockedRun {
        start,
        len: end - start,
        revealed: app
            .revealed_blocked_run_starts
            .contains(&messages[start].id),
    })
}

pub(super) fn blocked_row(
    app: &DiscordApp,
    messages: &[Message],
    index: usize,
    run: &BlockedRun,
    grouped: bool,
    pane: bool,
    cx: &App,
) -> AnyElement {
    let first_id = messages[run.start].id;
    if !run.revealed {
        return bar(app, first_id, run).mt(rpx(4.)).into_any_element();
    }
    div()
        .w_full()
        .bg(rgba(theme::hover()))
        .when(index == run.start, |this| {
            this.child(bar(app, first_id, run))
        })
        .when(run.is_last(index), |this| this.pb_2())
        .child({
            let grouped = grouped && index != run.start;
            if pane {
                let compact = crate::settings::display().message_display_compact;
                div()
                    .when(!grouped, |this| {
                        this.pt(rpx(crate::settings::message_group_spacing(compact) as f32))
                    })
                    .child(render_pane_message(app, &messages[index], grouped, cx))
                    .into_any_element()
            } else {
                render_message(app, &messages[index], grouped, false, cx)
            }
        })
        .into_any_element()
}

fn bar(app: &DiscordApp, first_id: u64, run: &BlockedRun) -> Stateful<Div> {
    let entity = app.entity.clone();
    let noun = if run.len == 1 {
        "blocked message"
    } else {
        "blocked messages"
    };
    div()
        .id(("blocked-run", first_id))
        .flex()
        .items_center()
        .w_full()
        .h(rpx(BAR_HEIGHT))
        .cursor_pointer()
        .text_size(rpx(16.))
        .without_text_selection()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.toggle_blocked_run(first_id, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .w(rpx(GUTTER))
                .child(theme::icon("ban", 18., theme::muted())),
        )
        .child(
            div()
                .text_color(rgb(theme::text()))
                .child(format!("{} {noun}", run.len)),
        )
        .child(
            div()
                .ml(rpx(4.))
                .text_color(rgb(theme::faint()))
                .child(if run.revealed { "— Hide" } else { "— Show" }),
        )
}
