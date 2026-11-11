use super::*;
use crate::app::state::{TrimCache, Trimmed};
use crate::store::Message;

const LOAD_REGION: f32 = 500.;
const UNREAD_LOAD_INSET: f32 = 18.;
const LOAD_PAUSE: f32 = 100.;
const PAGE_OVERLAP: f32 = 40.;
const WINDOW_LIMIT: usize = 120;
const WINDOW_KEEP: usize = 60;
const TRIM_CACHE_LIMIT: usize = 150;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
}

fn css(cx: &App, value: f32) -> Pixels {
    px(value * crate::theme::ZOOM_LEVELS[crate::theme::zoom_index(cx)])
}

impl TrimCache {
    fn push_newest(&mut self, mut messages: Vec<Message>, at_edge: bool) {
        if self.messages.is_empty() {
            self.at_edge = at_edge;
        }
        self.messages.append(&mut messages);
        if self.messages.len() > TRIM_CACHE_LIMIT {
            self.at_edge = false;
            self.messages
                .drain(..self.messages.len() - TRIM_CACHE_LIMIT);
        }
    }

    fn push_oldest(&mut self, mut messages: Vec<Message>, at_edge: bool) {
        if self.messages.is_empty() {
            self.at_edge = at_edge;
        }
        messages.append(&mut self.messages);
        self.messages = messages;
        if self.messages.len() > TRIM_CACHE_LIMIT {
            self.at_edge = false;
            self.messages.truncate(TRIM_CACHE_LIMIT);
        }
    }

    fn contains(&self, message_id: u64) -> bool {
        self.messages.iter().any(|message| message.id == message_id)
    }
}

impl DiscordApp {
    pub(in crate::app) fn page_size(&self, cx: &App) -> usize {
        let zoom = crate::theme::ZOOM_LEVELS[crate::theme::zoom_index(cx)];
        let available = (self.messages.viewport_bounds().size.height.as_f32() / zoom).max(400.);
        let row = if crate::settings::display().message_display_compact {
            65.
        } else {
            91.
        } * crate::settings::chat_font_size()
            / 16.;
        (((available / row).ceil() * 1.05).ceil() as usize).clamp(10, 30)
    }

    fn scroll_page_size(&self, cx: &App) -> usize {
        (2 * self.page_size(cx)).min(JUMP_PAGE)
    }

    pub(crate) fn edge_placeholder_height(&self) -> Pixels {
        let height = self.history.placeholder_height.get();
        if height > px(0.) { height } else { px(600.) }
    }

    pub(crate) fn has_older_history(&self, channel_id: u64) -> bool {
        self.live && !self.history.complete.contains(&channel_id)
    }

    fn content_height(&self) -> Pixels {
        self.messages.offset_for_item(self.messages.item_count())
    }

    fn load_edge(&self, cx: &App) -> Option<Edge> {
        let channel_id = self.selected_channel_id;
        let viewport = self.messages.viewport_bounds().size.height;
        if viewport <= px(0.) {
            return None;
        }
        let content = self.content_height();
        let bottom = (content - viewport).max(px(0.));
        let top = self.list_offset().min(bottom);
        let placeholder = self.edge_placeholder_height();
        let region = css(cx, LOAD_REGION);
        if self.has_older_history(channel_id) && content > viewport {
            let reach = if self.unread.since.is_some() {
                placeholder - css(cx, UNREAD_LOAD_INSET)
            } else {
                placeholder + region
            };
            if top <= reach {
                return Some(Edge::Top);
            }
        }
        (!self.store.at_present(channel_id) && top >= bottom - placeholder - region)
            .then_some(Edge::Bottom)
    }

    pub(in crate::app) fn check_message_edges(
        &mut self,
        user_scroll: bool,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        if !self.live
            || !matches!(self.screen, Screen::Chat)
            || self.selected_forum().is_some()
            || !self
                .channel()
                .is_some_and(|channel| channel.messages_loaded)
            || self.history.loading.contains(&channel_id)
        {
            return;
        }
        let edge = self.load_edge(cx);
        if user_scroll && edge.is_none() {
            self.history.paused.remove(&channel_id);
        }
        if self.jumps.scroll.is_some()
            || self.messages.is_scrollbar_dragging()
            || self.history.paused.contains(&channel_id)
            || self.history.older.contains(&channel_id)
            || self.history.newer.contains(&channel_id)
        {
            return;
        }
        match edge {
            Some(Edge::Top) => self.load_older_page(cx),
            Some(Edge::Bottom) => self.load_newer_page(cx),
            None => {}
        }
    }

    pub(crate) fn run_edge_check(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.history.edge_check)
            && let Some((channel_id, rows)) = self.history.load_rows.take()
            && channel_id == self.selected_channel_id
        {
            let mut added =
                self.messages.offset_for_item(rows.end) - self.messages.offset_for_item(rows.start);
            if rows.start == 0 && self.has_older_history(channel_id) {
                added -= self.edge_placeholder_height();
            }
            if added < css(cx, LOAD_PAUSE) {
                self.history.paused.insert(channel_id);
            }
        }
        self.check_message_edges(false, cx);
    }

    fn load_older_page(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let limit = self.scroll_page_size(cx);
        let cached = self
            .history
            .trimmed
            .get_mut(&channel_id)
            .filter(|trimmed| !trimmed.older.messages.is_empty())
            .map(|trimmed| {
                let older = &mut trimmed.older;
                let batch = older
                    .messages
                    .split_off(older.messages.len().saturating_sub(limit));
                (batch, older.messages.is_empty() && older.at_edge)
            });
        let Some((batch, reached_start)) = cached else {
            self.fetch_older_page(channel_id, limit, cx);
            return;
        };
        let optimistic = self.has_unsent(channel_id);
        let inserted = self.store.restore_older(channel_id, batch);
        if reached_start {
            self.history.complete.insert(channel_id);
        }
        self.older_loaded(inserted, optimistic, cx);
    }

    fn load_newer_page(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let limit = self.scroll_page_size(cx);
        let cached = self
            .history
            .trimmed
            .get_mut(&channel_id)
            .filter(|trimmed| !trimmed.newer.messages.is_empty())
            .map(|trimmed| {
                let newer = &mut trimmed.newer;
                let rest = newer.messages.split_off(limit.min(newer.messages.len()));
                let batch = std::mem::replace(&mut newer.messages, rest);
                (batch, newer.messages.is_empty() && newer.at_edge)
            });
        let Some((batch, reached_present)) = cached else {
            self.fetch_newer(limit, cx);
            return;
        };
        let start = self.channel().map_or(0, |channel| channel.messages.len());
        let inserted = self.store.restore_newer(channel_id, batch, reached_present);
        self.messages.splice(start..start, inserted);
        self.newer_loaded(start, inserted, cx);
    }

    fn has_unsent(&self, channel_id: u64) -> bool {
        self.channel_by_id(channel_id).is_some_and(|channel| {
            channel
                .messages
                .iter()
                .any(|message| message.delivery != crate::store::Delivery::Sent)
        })
    }

    /// GPUI doesn't lay out rows above a negative offset, so a viewport inside the skeleton
    /// lands on the old first message.
    pub(super) fn older_loaded(
        &mut self,
        inserted: usize,
        optimistic: bool,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let placeholder = self.edge_placeholder_height();
        let top = self.messages.logical_scroll_top();
        let following = self.messages.is_following_tail();
        if inserted > 0 {
            self.messages.splice(0..0, inserted);
        }
        if !following && top.item_ix == 0 {
            self.messages.scroll_to(ListOffset {
                item_ix: inserted,
                offset_in_item: (top.offset_in_item - placeholder).max(px(0.)),
            });
        }
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        self.messages.remeasure_items(if optimistic {
            0..len
        } else {
            0..(inserted + 1).min(len)
        });
        self.history.load_rows = Some((channel_id, 0..inserted));
        self.trim_newer_window();
        self.finish_scroll_load(cx);
    }

    pub(super) fn newer_loaded(&mut self, start: usize, inserted: usize, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        self.messages
            .remeasure_items(start.saturating_sub(1)..(start + 1).min(len));
        let dropped = self.trim_older_window(WINDOW_LIMIT);
        let start = start.saturating_sub(dropped);
        self.history.load_rows = Some((channel_id, start..start + inserted));
        self.finish_scroll_load(cx);
    }

    fn finish_scroll_load(&mut self, cx: &mut Context<Self>) {
        let len = self.messages.item_count();
        if len > 0 {
            self.messages.remeasure_items(0..1);
            self.messages.remeasure_items(len - 1..len);
        }
        self.history.edge_check = true;
        self.views.messages.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn trims_selected(&self) -> bool {
        self.thread_pane_channel()
            .is_none_or(|channel| channel.id != self.selected_channel_id)
    }

    fn trim_newer_window(&mut self) {
        let channel_id = self.selected_channel_id;
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        let top = self.messages.logical_scroll_top();
        if len <= WINDOW_LIMIT
            || !self.trims_selected()
            || top.item_ix >= WINDOW_KEEP
            || self.messages.item_is_below_viewport(WINDOW_KEEP) == Some(false)
        {
            return;
        }
        let was_present = self.store.at_present(channel_id);
        let dropped = self.store.trim_newer(channel_id, WINDOW_KEEP);
        if dropped.is_empty() {
            return;
        }
        let count = dropped.len();
        self.history
            .trimmed
            .entry(channel_id)
            .or_default()
            .newer
            .push_oldest(dropped, was_present);
        self.messages.set_follow_mode(FollowMode::Normal);
        self.messages.splice(WINDOW_KEEP..WINDOW_KEEP + count, 0);
        self.messages.remeasure_items(WINDOW_KEEP - 1..WINDOW_KEEP);
    }

    fn trim_older_window(&mut self, limit: usize) -> usize {
        let channel_id = self.selected_channel_id;
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        if len <= limit || !self.trims_selected() {
            return 0;
        }
        let excess = len - WINDOW_KEEP;
        if !self.messages.is_following_tail() && self.messages.logical_scroll_top().item_ix < excess
        {
            return 0;
        }
        let complete = self.history.complete.remove(&channel_id);
        let dropped = self.store.trim_older(channel_id, WINDOW_KEEP);
        if dropped.is_empty() {
            if complete {
                self.history.complete.insert(channel_id);
            }
            return 0;
        }
        let count = dropped.len();
        self.history
            .trimmed
            .entry(channel_id)
            .or_default()
            .older
            .push_newest(dropped, complete);
        self.messages.splice(0..count, 0);
        self.messages.remeasure_items(0..1);
        count
    }

    pub(in crate::app) fn window_after_insert(&mut self) -> usize {
        if self.messages.is_following_tail() {
            self.trim_older_window(WINDOW_KEEP)
        } else {
            self.trim_newer_window();
            0
        }
    }

    pub(super) fn present_from_trimmed(
        &mut self,
        limit: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let channel_id = self.selected_channel_id;
        let Some(trimmed) = self
            .history
            .trimmed
            .get_mut(&channel_id)
            .filter(|trimmed| trimmed.newer.at_edge && !trimmed.newer.messages.is_empty())
        else {
            return false;
        };
        let mut rest = std::mem::take(&mut trimmed.newer).messages;
        let present = rest.split_off(rest.len().saturating_sub(limit));
        let complete = self.history.complete.remove(&channel_id);
        let previous = self.store.swap_window(channel_id, present, true);
        let older = &mut self.history.trimmed.entry(channel_id).or_default().older;
        older.push_newest(previous, complete);
        older.push_newest(rest, false);
        let count = self.channel().map_or(0, |channel| channel.messages.len());
        self.reset_messages(count);
        self.glide_down_from_top(window, cx);
        self.views.messages.update(cx, |_, cx| cx.notify());
        true
    }

    pub(super) fn glide_down_from_top(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.reduce_motion() || self.messages.item_count() == 0 {
            return;
        }
        self.messages.set_follow_mode(FollowMode::Normal);
        self.messages.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        let entity = self.entity.clone();
        window.on_next_frame(move |window, cx| {
            entity
                .update(cx, |app, cx| {
                    app.start_jump(JumpTarget::Present, window, cx)
                })
                .ok();
        });
    }

    pub(in crate::app) fn trimmed_message_created(&mut self, channel_id: u64) {
        if let Some(trimmed) = self.history.trimmed.get_mut(&channel_id) {
            trimmed.newer.at_edge = false;
        }
    }

    pub(in crate::app) fn trimmed_message_changed(&mut self, channel_id: u64, message_id: u64) {
        let Some(trimmed) = self.history.trimmed.get_mut(&channel_id) else {
            return;
        };
        for side in [&mut trimmed.older, &mut trimmed.newer] {
            if side.contains(message_id) {
                *side = TrimCache::default();
            }
        }
    }

    pub(in crate::app) fn trimmed_message_deleted(&mut self, channel_id: u64, message_id: u64) {
        if let Some(trimmed) = self.history.trimmed.get_mut(&channel_id) {
            for side in [&mut trimmed.older, &mut trimmed.newer] {
                side.messages.retain(|message| message.id != message_id);
            }
        }
    }

    pub(in crate::app) fn forget_trimmed(&mut self) {
        self.history.trimmed.clear();
        self.history.paused.clear();
        self.history.load_rows = None;
    }

    pub(in crate::app) fn remember_scroll(&mut self) {
        let channel_id = self.selected_channel_id;
        let rows = self.channel().map(|channel| channel.messages.len());
        if self.messages.is_following_tail() || rows != Some(self.messages.item_count()) {
            self.history.scroll_memory.remove(&channel_id);
            return;
        }
        let top = self.messages.logical_scroll_top();
        match self
            .channel()
            .and_then(|channel| channel.messages.get(top.item_ix))
        {
            Some(message) => {
                self.history
                    .scroll_memory
                    .insert(channel_id, (message.id, top.offset_in_item));
            }
            None => {
                self.history.scroll_memory.remove(&channel_id);
            }
        }
    }

    pub(in crate::app) fn restore_scroll(&mut self) -> bool {
        let Some((message_id, offset)) =
            self.history.scroll_memory.remove(&self.selected_channel_id)
        else {
            return false;
        };
        let Some(index) = self.message_index(message_id) else {
            return false;
        };
        self.messages.set_follow_mode(FollowMode::Normal);
        self.messages.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: offset,
        });
        true
    }

    pub(crate) fn scroll_page(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat)
            || self.selected_forum().is_some()
            || !self
                .channel()
                .is_some_and(|channel| channel.messages_loaded)
        {
            return;
        }
        smooth_scroll::cancel(cx);
        self.history.paused.remove(&self.selected_channel_id);
        let viewport = self.messages.viewport_bounds().size.height;
        let step = (viewport - css(cx, PAGE_OVERLAP)).max(viewport / 2.);
        let distance = if down { step } else { -step };
        if cx.reduce_motion() {
            self.messages.scroll_by(distance);
            self.history.edge_check = true;
            self.views.messages.update(cx, |_, cx| cx.notify());
            return;
        }
        let bottom = (self.content_height() - viewport).max(px(0.));
        let from = match self.jumps.scroll.as_ref().map(|jump| jump.target) {
            Some(JumpTarget::Offset(target)) => target,
            _ => self.list_offset().min(bottom),
        };
        self.start_jump(JumpTarget::Offset(from + distance), window, cx);
    }
}

impl Trimmed {
    pub(crate) fn older_unread<'a>(
        &'a self,
        since: u64,
        me: u64,
    ) -> impl Iterator<Item = &'a Message> + 'a {
        self.older.messages.iter().filter(move |message| {
            message.delivery == crate::store::Delivery::Sent
                && message.author_id != me
                && message.id > since
        })
    }

    pub(crate) fn older_reaches_start(&self) -> bool {
        self.older.at_edge
    }
}
