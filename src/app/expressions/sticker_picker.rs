use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
};

use super::super::{
    DiscordApp, JumpToPresent,
    expressions::emoji_picker::{self, PickerTarget},
    expressions::gif_picker,
};
use crate::{
    media::{animated, images},
    store::{PickerSticker, Sticker, StickerSection, sticker_matches},
    theme::{self, rpx},
};

const COLUMNS: usize = 5;
const SIDEBAR: f32 = 48.;
const GRID_PADDING: f32 = 8.;
const CELL: f32 = (emoji_picker::PANEL_WIDTH - SIDEBAR - 2. - GRID_PADDING * 2.) / COLUMNS as f32;
const GRID_IMAGE: f32 = 64.;
const PREVIEW_IMAGE: f32 = 40.;
const PICKER_ENTER: Duration = Duration::from_millis(150);
const PICKER_EXIT: Duration = Duration::from_millis(100);
const REOPEN_GUARD: Duration = Duration::from_millis(150);

type Entry = (usize, usize);

enum Row {
    Header(usize),
    Stickers(usize, Vec<usize>),
}

#[derive(Default)]
pub(crate) struct Stickers {
    picker: Option<StickerPicker>,
    closing: Option<Task<()>>,
    animation: u64,
    closed_at: Option<Instant>,
}

struct StickerPicker {
    target: PickerTarget,
    channel_id: u64,
    position: Point<Pixels>,
    search: Entity<InputState>,
    sections: Rc<Vec<StickerSection>>,
    rows: Rc<Vec<Row>>,
    section_rows: Vec<(usize, usize)>,
    scroll: ListState,
    layout: Cell<Option<(Pixels, Pixels, bool)>>,
    hovered: Option<Entry>,
    over: Option<Entry>,
    closing: bool,
    swapped: bool,
    animation: u64,
    _search_events: Subscription,
}

fn channel(app: &DiscordApp, target: PickerTarget) -> Option<u64> {
    match target {
        PickerTarget::Composer => Some(app.selected_channel_id),
        PickerTarget::Pane if app.thread_pane_open() => {
            app.thread_pane_channel().map(|channel| channel.id)
        }
        PickerTarget::Pane => None,
        PickerTarget::Reaction(_) | PickerTarget::Activity(_) | PickerTarget::CustomStatus => None,
    }
}

impl DiscordApp {
    pub(crate) fn toggle_sticker_picker(
        &mut self,
        target: PickerTarget,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stickers.picker.as_ref().is_some_and(|picker| {
            picker.target == target
                && channel(self, target) == Some(picker.channel_id)
                && !picker.closing
        }) {
            self.dismiss_sticker_picker(window, cx);
            return;
        }
        if self
            .stickers
            .closed_at
            .is_some_and(|at| at.elapsed() < REOPEN_GUARD)
        {
            return;
        }
        self.open_sticker_picker(target, position, false, window, cx);
    }

    pub(super) fn open_sticker_tab(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = if self.gifs.pane {
            PickerTarget::Pane
        } else {
            PickerTarget::Composer
        };
        self.open_sticker_picker(target, position, true, window, cx);
    }

    pub(super) fn take_sticker_picker(&mut self) -> Option<Point<Pixels>> {
        let picker = self.stickers.picker.take()?;
        self.stickers.closing = None;
        self.gifs.pane = picker.target == PickerTarget::Pane;
        Some(picker.position)
    }

    fn open_sticker_picker(
        &mut self,
        target: PickerTarget,
        position: Point<Pixels>,
        swapped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = channel(self, target) else {
            return;
        };
        self.close_gif_picker(cx);
        self.take_emoji_picker();
        self.close_emoji_picker(cx);
        self.gifs.pane = target == PickerTarget::Pane;
        self.stickers.closing = None;
        self.stickers.animation = self.stickers.animation.wrapping_add(1);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search for stickers"));
        let events = cx.subscribe_in(&search, window, |app, _, event, window, cx| match event {
            InputEvent::Change => app.rebuild_sticker_rows(cx),
            InputEvent::PressEnter { shift, .. } => {
                let first = app.stickers.picker.as_ref().and_then(|picker| {
                    picker.rows.iter().find_map(|row| match row {
                        Row::Stickers(section, entries) => entries.iter().find_map(|&index| {
                            picker.sections[*section].stickers[index]
                                .usable
                                .then_some((*section, index))
                        }),
                        Row::Header(_) => None,
                    })
                });
                if let Some(entry) = first {
                    let generation = app.stickers.animation;
                    app.pick_sticker(entry, generation, *shift, window, cx);
                }
            }
            _ => {}
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        let sections = self
            .store
            .sticker_sections(self.request_guild(), channel_id);
        let scroll = ListState::new(
            0,
            ListAlignment::Top,
            rpx(CELL).to_pixels(window.rem_size()),
        );
        let entity = self.entity.clone();
        scroll.set_scroll_handler(move |_, _, cx| {
            entity.update(cx, |_, cx| cx.notify()).ok();
        });
        self.stickers.picker = Some(StickerPicker {
            target,
            channel_id,
            position,
            search,
            sections: Rc::new(sections),
            rows: Rc::new(Vec::new()),
            section_rows: Vec::new(),
            scroll,
            layout: Cell::new(None),
            hovered: None,
            over: None,
            closing: false,
            swapped,
            animation: self.stickers.animation,
            _search_events: events,
        });
        self.stickers.closed_at = None;
        self.rebuild_sticker_rows(cx);
    }

    fn rebuild_sticker_rows(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.stickers.picker else {
            return;
        };
        let query = picker.search.read(cx).value().trim().to_owned();
        let mut rows = Vec::new();
        let mut section_rows = Vec::new();
        for (section_index, section) in picker.sections.iter().enumerate() {
            let entries: Vec<_> = section
                .stickers
                .iter()
                .enumerate()
                .filter_map(|(index, sticker)| sticker_matches(sticker, &query).then_some(index))
                .collect();
            if entries.is_empty() {
                continue;
            }
            section_rows.push((section_index, rows.len()));
            rows.push(Row::Header(section_index));
            for chunk in entries.chunks(COLUMNS) {
                rows.push(Row::Stickers(section_index, chunk.to_vec()));
            }
        }
        picker.scroll.reset(rows.len());
        picker.rows = Rc::new(rows);
        picker.section_rows = section_rows;
        picker.hovered = None;
        picker.over = None;
        cx.notify();
    }

    fn hover_sticker(&mut self, entry: Entry, generation: u64, over: bool, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.stickers.picker else {
            return;
        };
        if picker.animation != generation || picker.closing {
            return;
        }
        if over {
            picker.hovered = Some(entry);
            picker.over = Some(entry);
        } else if picker.over == Some(entry) {
            picker.over = None;
        }
        cx.notify();
    }

    fn pick_sticker(
        &mut self,
        entry: Entry,
        generation: u64,
        keep_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self
            .stickers
            .picker
            .as_ref()
            .filter(|picker| !picker.closing && picker.animation == generation)
        else {
            return;
        };
        if channel(self, picker.target) != Some(picker.channel_id) {
            return;
        }
        let sticker = &picker.sections[entry.0].stickers[entry.1];
        if !sticker.usable {
            return;
        }
        let (channel_id, sticker_id) = (picker.channel_id, sticker.id);
        let usable = self
            .store
            .sticker_sections(self.request_guild(), channel_id)
            .iter()
            .flat_map(|section| &section.stickers)
            .any(|sticker| sticker.id == sticker_id && sticker.usable);
        if !usable {
            return;
        }
        self.send_sticker(channel_id, sticker_id, window, cx);
        if !keep_open {
            self.dismiss_sticker_picker(window, cx);
        } else if let Some(picker) = &self.stickers.picker {
            picker
                .search
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn dismiss_sticker_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.stickers.picker.as_ref().map(|picker| picker.target);
        if !self.close_sticker_picker(cx) {
            return;
        }
        match target {
            Some(PickerTarget::Composer) => self
                .compose
                .input
                .update(cx, |input, cx| input.focus(window, cx)),
            Some(PickerTarget::Pane) => self.focus_thread_pane_composer(window, cx),
            _ => {}
        }
    }

    fn close_sticker_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = &mut self.stickers.picker else {
            return false;
        };
        if picker.closing {
            return false;
        }
        picker.closing = true;
        let animation = picker.animation;
        self.stickers.closed_at = Some(Instant::now());
        if cx.reduce_motion() {
            self.stickers.picker = None;
            self.stickers.closing = None;
        } else {
            self.stickers.closing = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(PICKER_EXIT).await;
                this.update(cx, |app, cx| {
                    if app
                        .stickers
                        .picker
                        .as_ref()
                        .is_some_and(|picker| picker.closing && picker.animation == animation)
                    {
                        app.stickers.picker = None;
                        app.stickers.closing = None;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        cx.notify();
        true
    }
}

pub(crate) fn sticker_image(
    sticker: &PickerSticker,
    extent: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    let sticker = Sticker::from(sticker);
    let pixels = theme::image_px(extent);
    let image = sticker.image_spec().and_then(|spec| {
        if animate
            && matches!(sticker.format, 2 | 4)
            && let Some(frames) = animated::resolve(&spec, pixels, cx)
        {
            return Some(
                animated::image(frames)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .id("animated")
                    .into_any_element(),
            );
        }
        if matches!(sticker.format, 2 | 4) {
            return animated::first_frame(&spec, pixels, cx).map(|frame| {
                frame
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .id("still")
                    .into_any_element()
            });
        }
        images::resolve(&spec, pixels, cx).map(|path| {
            img(path)
                .id("still")
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        })
    });
    div()
        .id(("picker-sticker-image", sticker.id))
        .size(rpx(extent))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(image.unwrap_or_else(|| {
            theme::icon("sticker", extent.min(32.), theme::muted()).into_any_element()
        }))
        .into_any_element()
}

fn guild_icon(section: &StickerSection, extent: f32, cx: &App) -> AnyElement {
    let image = section
        .icon
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(extent), cx));
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(extent))
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(theme::surface_highest()))
        .text_size(rpx(extent * 0.36))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::text()))
        .child(match image {
            Some(path) => img(path)
                .size_full()
                .rounded_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => div()
                .child(
                    section
                        .name
                        .split_whitespace()
                        .filter_map(|word| word.chars().next())
                        .take(2)
                        .collect::<String>(),
                )
                .into_any_element(),
        })
        .into_any_element()
}

pub(crate) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let picker = app.stickers.picker.as_ref()?;
    if channel(app, picker.target) != Some(picker.channel_id) {
        return None;
    }
    let closing = picker.closing;
    let close = app.entity.clone();
    let escape = app.entity.clone();
    let rem = window.rem_size();
    let height = gif_picker::panel_height(picker.position, window);
    let width = rpx(emoji_picker::PANEL_WIDTH)
        .to_pixels(rem)
        .min(window.viewport_size().width - rpx(16.).to_pixels(rem));
    let compact = height < rpx(360.).to_pixels(rem);
    let layout = (width, rem, compact);
    if picker.layout.replace(Some(layout)) != Some(layout) {
        picker.scroll.reset(picker.rows.len());
    }
    let cell = ((width / rpx(1.).to_pixels(rem) - SIDEBAR - 2. - GRID_PADDING * 2.)
        / COLUMNS as f32)
        .min(if compact { 72. } else { CELL });
    let panel = div()
        .id("sticker-picker")
        .flex()
        .flex_col()
        .w(width)
        .h(height)
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
        .shadow_lg()
        .overflow_hidden()
        .when(!closing, |this| {
            this.occlude()
                .on_mouse_down_out(move |_, _, cx| {
                    close
                        .update(cx, |app, cx| app.close_sticker_picker(cx))
                        .ok();
                })
                .capture_action(move |_: &JumpToPresent, window, cx| {
                    escape
                        .update(cx, |app, cx| app.dismiss_sticker_picker(window, cx))
                        .ok();
                    cx.stop_propagation();
                })
        })
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(rpx(if compact { 4. } else { 12. }))
                .p(rpx(if compact { 8. } else { 12. }))
                .border_b_1()
                .border_color(rgba(theme::border()))
                .child(gif_picker::tabs(app, gif_picker::Tab::Stickers))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .px(rpx(12.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::bg()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .child(theme::icon("search", 18., theme::muted()))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&picker.search)
                                    .appearance(false)
                                    .h(rpx(if compact { 28. } else { 40. }))
                                    .px_0()
                                    .text_size(rpx(16.)),
                            ),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(sidebar(app, picker, cx))
                .child(grid(app, picker, compact, cell, cx)),
        )
        .child(footer(picker, cx));
    let panel = if cx.reduce_motion() || picker.swapped && !closing {
        panel.into_any_element()
    } else {
        panel
            .with_animation(
                (
                    if closing {
                        "sticker-picker-out"
                    } else {
                        "sticker-picker-in"
                    },
                    picker.animation,
                ),
                Animation::new(if closing { PICKER_EXIT } else { PICKER_ENTER })
                    .with_easing(ease_out_quint()),
                move |this, delta| {
                    let progress = if closing { 1. - delta } else { delta };
                    this.relative()
                        .top(rpx((1. - progress) * 8.))
                        .opacity(progress)
                },
            )
            .into_any_element()
    };
    Some(
        deferred(
            anchored()
                .position(picker.position)
                .anchor(Anchor::BottomRight)
                .snap_to_window_with_margin(rpx(8.).to_pixels(window.rem_size()))
                .child(panel),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

fn sidebar(app: &DiscordApp, picker: &StickerPicker, cx: &App) -> impl IntoElement {
    let top = picker.scroll.logical_scroll_top().item_ix;
    let active = picker
        .section_rows
        .iter()
        .rev()
        .find(|(_, row)| *row <= top)
        .map(|(section, _)| *section);
    div()
        .id("sticker-picker-sidebar")
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(8.))
        .flex_none()
        .w(rpx(SIDEBAR))
        .py(rpx(8.))
        .bg(rgb(theme::base_lowest()))
        .overflow_y_scroll()
        .children(picker.section_rows.iter().map(|&(section_index, row)| {
            let section = &picker.sections[section_index];
            let scroll = picker.scroll.clone();
            let entity = app.entity.clone();
            div()
                .id(("sticker-section", section.guild_id))
                .role(Role::Button)
                .aria_label(section.name.clone())
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded(rpx(6.))
                .when(active == Some(section_index), |this| {
                    this.bg(rgba(theme::selected()))
                })
                .when(!picker.closing, |this| {
                    let keyboard_scroll = scroll.clone();
                    let keyboard = entity.clone();
                    this.focusable()
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .focus_visible(|style| style.bg(rgba(theme::selected())))
                        .on_click(move |_, _, cx| {
                            scroll.scroll_to(ListOffset {
                                item_ix: row,
                                offset_in_item: px(0.),
                            });
                            entity.update(cx, |_, cx| cx.notify()).ok();
                        })
                        .on_key_down(move |event, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                keyboard_scroll.scroll_to(ListOffset {
                                    item_ix: row,
                                    offset_in_item: px(0.),
                                });
                                keyboard.update(cx, |_, cx| cx.notify()).ok();
                                cx.stop_propagation();
                            }
                        })
                })
                .child(guild_icon(section, 28., cx))
                .child(theme::hover_tooltip(
                    ("sticker-section-tip", section.guild_id),
                    section.name.clone(),
                ))
        }))
}

fn grid(
    app: &DiscordApp,
    picker: &StickerPicker,
    compact: bool,
    cell: f32,
    cx: &App,
) -> AnyElement {
    if picker.rows.is_empty() {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(rpx(12.))
            .p(rpx(16.))
            .child(theme::icon("sticker", 40., theme::muted()))
            .child(
                div()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(if picker.search.read(cx).value().trim().is_empty() {
                        "No stickers available"
                    } else {
                        "No stickers found"
                    }),
            )
            .into_any_element();
    }
    let sections = picker.sections.clone();
    let rows = picker.rows.clone();
    let over = picker.over;
    let interactive = !picker.closing;
    let animate = interactive && !cx.reduce_motion();
    let generation = picker.animation;
    let entity = app.entity.clone();
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .relative()
        .child(
            list(picker.scroll.clone(), move |row, _, cx| match &rows[row] {
                Row::Header(section) => div()
                    .flex()
                    .items_end()
                    .gap(rpx(8.))
                    .h(rpx(if compact { 28. } else { 48. }))
                    .px(rpx(4.))
                    .pb(rpx(8.))
                    .child(guild_icon(&sections[*section], 16., cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::text()))
                            .child(sections[*section].name.clone()),
                    )
                    .into_any_element(),
                Row::Stickers(section, entries) => div()
                    .flex()
                    .justify_between()
                    .h(rpx(cell))
                    .children(entries.iter().map(|&index| {
                        let sticker = &sections[*section].stickers[index];
                        let entry = (*section, index);
                        let hover = entity.clone();
                        let click = entity.clone();
                        let keyboard = entity.clone();
                        let usable = sticker.usable;
                        div()
                            .id(("sticker-cell", sticker.id))
                            .role(Role::Button)
                            .aria_label(if usable {
                                format!("Send {}", sticker.name)
                            } else {
                                format!("{}, unavailable in this channel", sticker.name)
                            })
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(rpx(cell))
                            .rounded(rpx(6.))
                            .when(over == Some(entry), |this| this.bg(rgba(theme::hover())))
                            .when(interactive, |this| {
                                this.on_hover(move |over, _, cx| {
                                    hover
                                        .update(cx, |app, cx| {
                                            app.hover_sticker(entry, generation, *over, cx)
                                        })
                                        .ok();
                                })
                                .when(usable, |this| {
                                    this.focusable()
                                        .tab_index(0)
                                        .cursor_pointer()
                                        .focus_visible(|style| style.bg(rgba(theme::selected())))
                                        .on_click(move |event, window, cx| {
                                            click
                                                .update(cx, |app, cx| {
                                                    app.pick_sticker(
                                                        entry,
                                                        generation,
                                                        event.modifiers().shift,
                                                        window,
                                                        cx,
                                                    )
                                                })
                                                .ok();
                                            cx.stop_propagation();
                                        })
                                        .on_key_down(move |event, window, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                keyboard
                                                    .update(cx, |app, cx| {
                                                        app.pick_sticker(
                                                            entry,
                                                            generation,
                                                            event.keystroke.modifiers.shift,
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                    .ok();
                                                cx.stop_propagation();
                                            }
                                        })
                                })
                            })
                            .child(div().when(!usable, |this| this.opacity(0.35)).child(
                                sticker_image(
                                    sticker,
                                    if compact { 48. } else { GRID_IMAGE }.min(cell - 8.),
                                    animate && over == Some(entry),
                                    cx,
                                ),
                            ))
                            .when(!usable, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .right(rpx(6.))
                                        .bottom(rpx(6.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(rpx(22.))
                                        .rounded_full()
                                        .bg(rgb(theme::elevated()))
                                        .child(theme::icon("lock", 14., theme::muted())),
                                )
                            })
                    }))
                    .children((entries.len()..COLUMNS).map(|_| div().size(rpx(cell)).flex_none()))
                    .into_any_element(),
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .px(rpx(GRID_PADDING)),
        )
        .vertical_scrollbar(&picker.scroll)
        .into_any_element()
}

fn footer(picker: &StickerPicker, cx: &App) -> impl IntoElement {
    let entry = picker.hovered.or_else(|| {
        picker.rows.iter().find_map(|row| match row {
            Row::Stickers(section, entries) => entries.first().map(|&index| (*section, index)),
            Row::Header(_) => None,
        })
    });
    div()
        .id("sticker-picker-footer")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(56.))
        .px(rpx(12.))
        .border_t_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::bg()))
        .children(entry.map(|(section, index)| {
            let section = &picker.sections[section];
            let sticker = &section.stickers[index];
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(rpx(12.))
                .child(sticker_image(
                    sticker,
                    PREVIEW_IMAGE,
                    !picker.closing && !cx.reduce_motion(),
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rpx(2.))
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(sticker.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(theme::muted()))
                                .child(section.name.clone()),
                        )
                        .when(!sticker.usable, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(4.))
                                    .child(theme::icon("lock", 12., theme::muted()))
                                    .child(
                                        div()
                                            .text_size(rpx(12.))
                                            .text_color(rgb(theme::muted()))
                                            .child("Unavailable in this channel"),
                                    ),
                            )
                        }),
                )
        }))
}
