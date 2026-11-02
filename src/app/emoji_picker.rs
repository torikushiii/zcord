use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    app::{DiscordApp, gif_picker},
    media::animated,
    media::images,
    store::{EmojiAccess, EmojiChoice},
    text::emoji,
    theme::{self, rpx},
};

const COLUMNS: usize = 9;
const CELL: f32 = 48.;
const ROW: f32 = 48.;
const SIDEBAR: f32 = 48.;
pub(super) const HEIGHT: f32 = 488.;
pub(super) const PANEL_WIDTH: f32 = SIDEBAR + COLUMNS as f32 * CELL + 24.;
const SUGGESTIONS: usize = 10;
const GRID_EMOJI: f32 = 40.;
const PICKER_RISE: f32 = 8.;
const PICKER_ENTER: Duration = Duration::from_millis(150);
const PICKER_EXIT: Duration = Duration::from_millis(100);
/// A press that closed the picker via mouse-down-outside must not reopen it from
/// the toggle button handling the same press.
const REOPEN_GUARD: Duration = Duration::from_millis(150);
/// Representative emoji for the Unicode category buttons, in `emoji::categories()` order.
const CATEGORY_ICONS: [(&str, &str); 8] = [
    ("People", "😀"),
    ("Nature", "🐱"),
    ("Food", "🍎"),
    ("Activities", "⚽"),
    ("Travel", "🚗"),
    ("Objects", "💡"),
    ("Symbols", "❤️"),
    ("Flags", "🏁"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerTarget {
    Composer,
    Pane,
    Reaction(u64),
}

type Entry = (EmojiChoice, EmojiAccess);

#[derive(Clone, Copy)]
enum HoverUpdate<'a> {
    Enter(&'a Entry),
    Leave,
    Reset,
}

/// The footer keeps the last emoji the pointer entered. In particular, moving
/// from cell A to cell B can deliver B's enter before A's leave, and leaving
/// the grid entirely matches Discord by keeping the preview. Only rebuilding
/// the rows (a search change) resets it.
fn next_hovered(current: Option<&Entry>, update: HoverUpdate<'_>) -> Option<Entry> {
    match update {
        HoverUpdate::Enter(entry) => Some(entry.clone()),
        HoverUpdate::Leave => current.cloned(),
        HoverUpdate::Reset => None,
    }
}

enum Row {
    Header(SharedString),
    Emojis(Vec<Entry>),
}

enum SectionIcon {
    Recent,
    Guild(Option<images::ImageSpec>, SharedString),
    Unicode(&'static str),
}

struct Section {
    icon: SectionIcon,
    row: usize,
}

pub(crate) struct EmojiPicker {
    target: PickerTarget,
    position: Point<Pixels>,
    search: Entity<InputState>,
    rows: Rc<Vec<Row>>,
    sections: Vec<Section>,
    scroll: UniformListScrollHandle,
    hovered: Option<Entry>,
    closing: bool,
    swapped: bool,
    /// Unique per open, so reopening replays the entry animation.
    animation: u64,
    _search_events: Subscription,
}

/// Composer autocomplete state; the suggestions themselves are derived from the text.
#[derive(Default)]
pub(crate) struct Suggest {
    index: usize,
    dismissed: Option<String>,
}

impl DiscordApp {
    /// Toggles the picker; `position` is where the triggering press happened, and the
    /// picker opens above-left of it.
    pub(crate) fn toggle_emoji_picker(
        &mut self,
        target: PickerTarget,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .emoji_picker
            .as_ref()
            .is_some_and(|picker| picker.target == target && !picker.closing)
        {
            self.close_emoji_picker(cx);
            return;
        }
        // A closing picker reaches the guard below: it keeps the press that
        // started the close from reopening the picker. If the close task is
        // late and the guard has expired, opening drops the task instead.
        if self
            .emoji_picker_closed_at
            .is_some_and(|at| at.elapsed() < REOPEN_GUARD)
        {
            return;
        }
        self.open_emoji_picker(target, position, false, window, cx);
    }

    pub(super) fn open_emoji_tab(
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
        self.open_emoji_picker(target, position, true, window, cx);
    }

    /// Removes the composer picker at once (no exit animation) and returns its anchor.
    pub(super) fn take_emoji_picker(&mut self) -> Option<Point<Pixels>> {
        let picker = self.emoji_picker.take_if(|picker| {
            matches!(picker.target, PickerTarget::Composer | PickerTarget::Pane)
        })?;
        self.gifs.pane = picker.target == PickerTarget::Pane;
        self.emoji_picker_closing = None;
        Some(picker.position)
    }

    /// `swapped` opens in place of the GIF picker's tab, without the entry animation.
    fn open_emoji_picker(
        &mut self,
        target: PickerTarget,
        position: Point<Pixels>,
        swapped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_gif_picker(cx);
        // A pending close must never remove the picker opened in its place.
        self.emoji_picker_closing = None;
        self.emoji_picker_animation = self.emoji_picker_animation.wrapping_add(1);
        let animation = self.emoji_picker_animation;
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find the perfect emoji"));
        let events = cx.subscribe_in(&search, window, |app, _, event, window, cx| match event {
            InputEvent::Change => {
                app.rebuild_emoji_rows(cx);
                if let Some(picker) = &app.emoji_picker {
                    picker.scroll.scroll_to_item(0, ScrollStrategy::Top);
                }
            }
            InputEvent::PressEnter { shift, .. } => {
                let first = app.emoji_picker.as_ref().and_then(|picker| {
                    picker.rows.iter().find_map(|row| match row {
                        Row::Emojis(entries) => entries
                            .iter()
                            .find(|(_, access)| *access == EmojiAccess::Usable)
                            .cloned(),
                        Row::Header(_) => None,
                    })
                });
                if let Some((choice, _)) = first {
                    app.pick_emoji(&choice, *shift, window, cx);
                }
            }
            _ => {}
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        self.emoji_picker = Some(EmojiPicker {
            target,
            position,
            search,
            rows: Rc::new(Vec::new()),
            sections: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            hovered: None,
            closing: false,
            swapped,
            animation,
            _search_events: events,
        });
        self.emoji_picker_closed_at = None;
        self.rebuild_emoji_rows(cx);
    }

    pub(crate) fn picking_reaction_for(&self, message_id: u64) -> bool {
        self.emoji_picker
            .as_ref()
            .is_some_and(|picker| picker.target == PickerTarget::Reaction(message_id))
    }

    pub(crate) fn close_emoji_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = &mut self.emoji_picker else {
            return false;
        };
        if picker.closing {
            return false;
        }
        picker.closing = true;
        let animation = picker.animation;
        self.emoji_picker_closed_at = Some(Instant::now());
        if cx.reduce_motion() {
            self.emoji_picker = None;
            self.emoji_picker_closing = None;
            cx.notify();
            return true;
        }
        self.emoji_picker_closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PICKER_EXIT).await;
            this.update(cx, |app, cx| {
                // A newer open replaces the picker and cancels this task; if the
                // task still runs, only remove the exact picker it started with.
                let still_closing = app
                    .emoji_picker
                    .as_ref()
                    .is_some_and(|picker| picker.closing && picker.animation == animation);
                if still_closing {
                    app.emoji_picker = None;
                    app.emoji_picker_closing = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
        true
    }

    fn rebuild_emoji_rows(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &self.emoji_picker else {
            return;
        };
        let query = picker.search.read(cx).value().trim().to_owned();
        let (rows, sections) = build_rows(self, &query, cx);
        if let Some(picker) = &mut self.emoji_picker {
            picker.rows = Rc::new(rows);
            picker.sections = sections;
            let hovered = next_hovered(picker.hovered.as_ref(), HoverUpdate::Reset);
            picker.hovered = hovered;
        }
        cx.notify();
    }

    /// Shift-click (or shift-enter) keeps the picker open, like Discord.
    fn pick_emoji(
        &mut self,
        choice: &EmojiChoice,
        keep_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.emoji_picker.as_ref().map(|picker| picker.target) else {
            return;
        };
        match target {
            PickerTarget::Composer => self.insert_emoji(choice, window, cx),
            PickerTarget::Pane => self.insert_pane_emoji(choice, window, cx),
            PickerTarget::Reaction(message_id) => self.react(message_id, choice, cx),
        }
        if !keep_open {
            self.close_emoji_picker(cx);
        }
    }

    fn hover_emoji(&mut self, update: HoverUpdate, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.emoji_picker else {
            return;
        };
        let hovered = next_hovered(picker.hovered.as_ref(), update);
        if picker.hovered != hovered {
            picker.hovered = hovered;
            cx.notify();
        }
    }

    pub(crate) fn emoji_suggestions(&self, cx: &App) -> Option<(String, Vec<EmojiChoice>)> {
        let query = self.emoji_query(cx)?;
        if self.emoji_suggest.dismissed.as_deref() == Some(query.as_str()) {
            return None;
        }
        let choices: Vec<_> = self
            .store
            .search_emoji(
                &query,
                self.selected_guild_id,
                self.selected_channel_id,
                SUGGESTIONS * 3,
            )
            .into_iter()
            .filter(|(_, access)| *access == EmojiAccess::Usable)
            .map(|(choice, _)| choice)
            .take(SUGGESTIONS)
            .collect();
        (!choices.is_empty()).then_some((query, choices))
    }

    pub(crate) fn reset_emoji_suggest(&mut self, cx: &App) {
        self.emoji_suggest.index = 0;
        if self.emoji_suggest.dismissed.is_some()
            && self.emoji_suggest.dismissed != self.emoji_query(cx)
        {
            self.emoji_suggest.dismissed = None;
        }
    }

    /// Moves the highlighted suggestion; returns false when no list is showing.
    pub(crate) fn move_emoji_suggestion(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some((_, choices)) = self.emoji_suggestions(cx) else {
            return false;
        };
        let len = choices.len() as isize;
        let index =
            (self.emoji_suggest.index.min(choices.len() - 1) as isize + delta).rem_euclid(len);
        self.emoji_suggest.index = index as usize;
        cx.notify();
        true
    }

    /// Inserts the highlighted (or given) suggestion; returns false when no list is showing.
    pub(crate) fn accept_emoji_suggestion(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((query, choices)) = self.emoji_suggestions(cx) else {
            return false;
        };
        let index = index
            .unwrap_or(self.emoji_suggest.index)
            .min(choices.len() - 1);
        self.replace_emoji_query(query.chars().count() + 1, &choices[index], window, cx);
        self.emoji_suggest = Suggest::default();
        true
    }

    pub(crate) fn dismiss_emoji_suggestions(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((query, _)) = self.emoji_suggestions(cx) else {
            return false;
        };
        self.emoji_suggest.dismissed = Some(query);
        cx.notify();
        true
    }

    fn hover_emoji_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.emoji_suggest.index != index {
            self.emoji_suggest.index = index;
            cx.notify();
        }
    }
}

fn build_rows(app: &DiscordApp, query: &str, cx: &App) -> (Vec<Row>, Vec<Section>) {
    let (guild_id, channel_id) = (app.selected_guild_id, app.selected_channel_id);
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    let push = |rows: &mut Vec<Row>, entries: Vec<Entry>| {
        for chunk in entries.chunks(COLUMNS) {
            rows.push(Row::Emojis(chunk.to_vec()));
        }
    };
    if !query.is_empty() {
        let results = app.store.search_emoji(query, guild_id, channel_id, 400);
        push(&mut rows, results);
        return (rows, sections);
    }
    let access = |choice: &EmojiChoice| match choice {
        EmojiChoice::Custom(custom) => app.store.emoji_access(custom, guild_id, channel_id),
        EmojiChoice::Unicode { .. } => EmojiAccess::Usable,
    };
    let recent: Vec<Entry> = app
        .recent_emojis(cx)
        .into_iter()
        .map(|choice| {
            let access = access(&choice);
            (choice, access)
        })
        .collect();
    if !recent.is_empty() {
        sections.push(Section {
            icon: SectionIcon::Recent,
            row: rows.len(),
        });
        rows.push(Row::Header("Frequently Used".into()));
        push(&mut rows, recent);
    }
    for (_, name, icon, emojis) in app.store.emoji_guilds() {
        sections.push(Section {
            icon: SectionIcon::Guild(icon, initials(&name).into()),
            row: rows.len(),
        });
        rows.push(Row::Header(name.into()));
        let entries = emojis
            .into_iter()
            .map(|custom| {
                let choice = EmojiChoice::Custom(custom);
                let access = access(&choice);
                (choice, access)
            })
            .collect();
        push(&mut rows, entries);
    }
    for category in emoji::categories() {
        let icon = CATEGORY_ICONS
            .iter()
            .find(|(name, _)| *name == category.name)
            .map_or("😀", |(_, icon)| icon);
        sections.push(Section {
            icon: SectionIcon::Unicode(icon),
            row: rows.len(),
        });
        rows.push(Row::Header(category.name.into()));
        let entries = category
            .emojis
            .iter()
            .map(|(emoji, shortcode)| {
                (
                    EmojiChoice::Unicode {
                        emoji: (*emoji).to_owned(),
                        shortcode: (*shortcode).to_owned(),
                    },
                    EmojiAccess::Usable,
                )
            })
            .collect();
        push(&mut rows, entries);
    }
    (rows, sections)
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(3)
        .collect()
}

fn choice_name(choice: &EmojiChoice) -> &str {
    match choice {
        EmojiChoice::Custom(custom) => &custom.name,
        EmojiChoice::Unicode { shortcode, .. } => shortcode,
    }
}

/// Resolves an emoji through the image service. The footer uses this path for
/// its single animated preview; other previews use [`emoji_preview_image`] instead.
pub(crate) fn emoji_image(choice: &EmojiChoice, size: f32, animate: bool, cx: &App) -> AnyElement {
    let image: Option<ImageSource> = match choice {
        EmojiChoice::Custom(custom) => images::resolve(
            &images::ImageSpec::Emoji {
                id: custom.id,
                animated: animate && custom.animated,
            },
            theme::image_px(size),
            cx,
        )
        .map(Into::into),
        EmojiChoice::Unicode { emoji, .. } => emoji::image(emoji).map(Into::into),
    };
    div()
        .size(rpx(size))
        .flex_none()
        .children(image.map(|image| {
            img(image)
                .id("emoji-preview-image")
                .size_full()
                .object_fit(ObjectFit::Contain)
        }))
        .into_any_element()
}

/// Picker cells and autocomplete rows share frame-timed playback, with a still
/// fallback while the animation is decoding.
fn emoji_preview_image(choice: &EmojiChoice, size: f32, animate: bool, cx: &App) -> AnyElement {
    if animate
        && let EmojiChoice::Custom(custom) = choice
        && custom.animated
    {
        let spec = images::ImageSpec::Emoji {
            id: custom.id,
            animated: true,
        };
        if let Some(image) = animated::resolve(&spec, theme::image_px(size), cx) {
            return animated::image(image)
                .size(rpx(size))
                .flex_none()
                .object_fit(ObjectFit::Contain)
                .id(("picker-emoji", custom.id))
                .into_any_element();
        }
    }
    emoji_image(choice, size, false, cx)
}

fn access_note(access: EmojiAccess) -> Option<&'static str> {
    match access {
        EmojiAccess::Usable => None,
        EmojiAccess::NeedsNitro => Some("Unlock with Nitro to use this emoji here"),
        EmojiAccess::NeedsRole => Some("You don't have the role for this emoji"),
        EmojiAccess::Unavailable => Some("This emoji is unavailable"),
        EmojiAccess::NoExternalPermission => Some("External emoji aren't allowed in this channel"),
    }
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let picker = app.emoji_picker.as_ref()?;
    let closing = picker.closing;
    let close = app.entity.clone();
    let key = if closing {
        "emoji-picker-out"
    } else {
        "emoji-picker-in"
    };
    let animation = Animation::new(if closing { PICKER_EXIT } else { PICKER_ENTER })
        .with_easing(ease_out_quint());
    let animate_grid = !closing && !cx.reduce_motion();
    let panel = div()
        .id("emoji-picker")
        .flex()
        .flex_col()
        .w(rpx(PANEL_WIDTH))
        .h(gif_picker::panel_height(picker.position, window))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
        .shadow_lg()
        .overflow_hidden()
        .when(!closing, |this| {
            // Closing panels do not occlude and detach the grid/sidebar
            // handlers, so clicks fall through until the close task removes it.
            this.occlude().on_mouse_down_out(move |_, _, cx| {
                close.update(cx, |app, cx| app.close_emoji_picker(cx)).ok();
            })
        })
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .p(rpx(12.))
                .border_b_1()
                .border_color(rgba(theme::BORDER))
                .when(
                    matches!(picker.target, PickerTarget::Composer | PickerTarget::Pane),
                    |this| this.child(gif_picker::tabs(app, gif_picker::Tab::Emoji)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .px(rpx(12.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::BG))
                        .border_1()
                        .border_color(rgba(theme::BORDER))
                        .child(theme::icon("search", 18., theme::MUTED))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&picker.search)
                                    .appearance(false)
                                    .h(rpx(40.))
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
                .child(sidebar(picker, cx, !closing))
                .child(grid(app, picker, animate_grid, !closing)),
        )
        .child(footer(app, picker, cx));
    let panel = if picker.swapped && !closing {
        panel.into_any_element()
    } else {
        panel
            .with_animation((key, picker.animation), animation, move |this, delta| {
                let progress = if closing { 1. - delta } else { delta };
                this.relative()
                    .top(px((1. - progress) * PICKER_RISE))
                    .opacity(progress)
            })
            .into_any_element()
    };
    Some(
        deferred(
            anchored()
                .position(picker.position)
                .anchor(Anchor::BottomRight)
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

fn sidebar(picker: &EmojiPicker, cx: &App, interactive: bool) -> impl IntoElement {
    div()
        .id("emoji-picker-sidebar")
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(8.))
        .flex_none()
        .w(rpx(SIDEBAR))
        .py(rpx(8.))
        .bg(rgb(theme::BG))
        .overflow_y_scroll()
        .children(picker.sections.iter().enumerate().map(|(index, section)| {
            let scroll = picker.scroll.clone();
            let row = section.row;
            let icon: AnyElement = match &section.icon {
                SectionIcon::Recent => theme::icon("clock", 20., theme::MUTED).into_any_element(),
                SectionIcon::Unicode(emoji) => div()
                    .size(rpx(22.))
                    .children(
                        emoji::image(emoji)
                            .map(|image| img(image).size_full().object_fit(ObjectFit::Contain)),
                    )
                    .into_any_element(),
                SectionIcon::Guild(spec, initials) => {
                    let image = spec
                        .as_ref()
                        .and_then(|spec| images::resolve(spec, theme::image_px(28.), cx));
                    match image {
                        Some(path) => img(path)
                            .size(rpx(28.))
                            .rounded_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        None => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(rpx(28.))
                            .rounded_full()
                            .bg(rgba(theme::SELECTED))
                            .text_size(rpx(10.))
                            .text_color(rgb(theme::TEXT))
                            .child(initials.clone())
                            .into_any_element(),
                    }
                }
            };
            div()
                .id(("emoji-section", index))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded(rpx(6.))
                .when(interactive, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::SELECTED)))
                        .on_click(move |_, _, _| scroll.scroll_to_item(row, ScrollStrategy::Top))
                })
                .child(icon)
        }))
}

fn grid(
    app: &DiscordApp,
    picker: &EmojiPicker,
    animate: bool,
    interactive: bool,
) -> impl IntoElement {
    let rows = picker.rows.clone();
    let entity = app.entity.clone();
    let hovered = picker.hovered.clone();
    if rows.is_empty() {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_size(rpx(14.))
            .text_color(rgb(theme::MUTED))
            .child("No emoji match your search")
            .into_any_element();
    }
    uniform_list("emoji-grid", rows.len(), move |range, _, cx| {
        range
            .map(|index| match &rows[index] {
                Row::Header(name) => div()
                    .flex()
                    .items_end()
                    .h(rpx(ROW))
                    .px(rpx(4.))
                    .pb(rpx(8.))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .truncate()
                    .child(name.clone())
                    .into_any_element(),
                Row::Emojis(entries) => div()
                    .flex()
                    .h(rpx(ROW))
                    .children(entries.iter().enumerate().map(|(column, entry)| {
                        let (choice, access) = entry.clone();
                        let usable = access == EmojiAccess::Usable;
                        let selected = hovered.as_ref() == Some(entry);
                        let hover = entity.clone();
                        let click = entity.clone();
                        let hovered_entry = entry.clone();
                        div()
                            .id(("emoji-cell", index * COLUMNS + column))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(rpx(CELL))
                            .rounded(rpx(6.))
                            .when(selected, |this| this.bg(rgba(theme::SELECTED)))
                            .when(usable && interactive, |this| this.cursor_pointer())
                            .when(interactive, move |this| {
                                this.on_hover(move |hovered, _, cx| {
                                    let update = if *hovered {
                                        HoverUpdate::Enter(&hovered_entry)
                                    } else {
                                        HoverUpdate::Leave
                                    };
                                    hover.update(cx, |app, cx| app.hover_emoji(update, cx)).ok();
                                })
                                .on_click(
                                    move |event, window, cx| {
                                        if !usable {
                                            return;
                                        }
                                        let keep_open = event.modifiers().shift;
                                        click
                                            .update(cx, |app, cx| {
                                                app.pick_emoji(&choice, keep_open, window, cx)
                                            })
                                            .ok();
                                    },
                                )
                            })
                            .child(
                                div()
                                    .when(!usable, |this| this.opacity(0.3))
                                    .child(emoji_preview_image(&entry.0, GRID_EMOJI, animate, cx)),
                            )
                    }))
                    .into_any_element(),
            })
            .collect()
    })
    .track_scroll(&picker.scroll)
    .flex_1()
    .h_full()
    .px(rpx(8.))
    .into_any_element()
}

fn footer(app: &DiscordApp, picker: &EmojiPicker, cx: &App) -> impl IntoElement {
    let content: AnyElement = match &picker.hovered {
        Some((choice, access)) => {
            let source = match choice {
                EmojiChoice::Custom(custom) => app
                    .store
                    .guilds
                    .iter()
                    .find(|guild| guild.id == custom.guild_id)
                    .map(|guild| format!("from {}", guild.name)),
                EmojiChoice::Unicode { .. } => None,
            };
            let note = access_note(*access).map(str::to_owned).or(source);
            div()
                .flex()
                .items_center()
                .gap(rpx(10.))
                .min_w_0()
                .child(emoji_image(choice, 32., true, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(format!(":{}:", choice_name(choice))),
                        )
                        .children(note.map(|note| {
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(if *access == EmojiAccess::Usable {
                                    theme::MUTED
                                } else {
                                    theme::YELLOW
                                }))
                                .child(note)
                        })),
                )
                .into_any_element()
        }
        None => div()
            .text_size(rpx(13.))
            .text_color(rgb(theme::FAINT))
            .child("Hover an emoji to preview it")
            .into_any_element(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(56.))
        .px(rpx(12.))
        .border_t_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::BG))
        .child(content)
}

pub(super) fn suggestions(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let (query, choices) = app.emoji_suggestions(cx)?;
    let selected = app.emoji_suggest.index.min(choices.len() - 1);
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb(rpx(8.))
            .flex()
            .flex_col()
            .py(rpx(8.))
            .rounded(rpx(8.))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .bg(rgb(theme::ELEVATED))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .px(rpx(12.))
                    .pb(rpx(6.))
                    .text_size(rpx(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::MUTED))
                    .child(format!("EMOJI MATCHING :{}", query.to_uppercase())),
            )
            .children(choices.iter().enumerate().map(|(index, choice)| {
                let hover = app.entity.clone();
                let click = app.entity.clone();
                let source = match choice {
                    EmojiChoice::Custom(custom) => app
                        .store
                        .guilds
                        .iter()
                        .find(|guild| guild.id == custom.guild_id)
                        .map(|guild| guild.name.clone()),
                    EmojiChoice::Unicode { .. } => None,
                };
                div()
                    .id(("emoji-suggestion", index))
                    .flex()
                    .items_center()
                    .gap(rpx(10.))
                    .mx(rpx(8.))
                    .px(rpx(8.))
                    .h(rpx(36.))
                    .rounded(rpx(4.))
                    .cursor_pointer()
                    .when(index == selected, |this| this.bg(rgba(theme::SELECTED)))
                    .on_hover(move |hovered, _, cx| {
                        if *hovered {
                            hover
                                .update(cx, |app, cx| app.hover_emoji_suggestion(index, cx))
                                .ok();
                        }
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |_, window, cx| {
                        click
                            .update(cx, |app, cx| {
                                app.accept_emoji_suggestion(Some(index), window, cx)
                            })
                            .ok();
                    })
                    .child(emoji_preview_image(
                        choice,
                        24.,
                        crate::settings::emoji_animation(),
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(15.))
                            .text_color(rgb(theme::STRONG))
                            .child(format!(":{}:", choice_name(choice))),
                    )
                    .children(source.map(|source| {
                        div()
                            .flex_none()
                            .max_w(rpx(200.))
                            .truncate()
                            .text_size(rpx(12.))
                            .text_color(rgb(theme::MUTED))
                            .child(source)
                    }))
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Entry, HoverUpdate, next_hovered};
    use crate::store::{EmojiAccess, EmojiChoice};

    fn entry(shortcode: &str) -> Entry {
        (
            EmojiChoice::Unicode {
                emoji: "🙂".to_owned(),
                shortcode: shortcode.to_owned(),
            },
            EmojiAccess::Usable,
        )
    }

    #[test]
    fn hover_keeps_the_latest_cell_despite_stale_leave_events() {
        let first = entry("first");
        let second = entry("second");

        let hovered = next_hovered(None, HoverUpdate::Enter(&first));
        assert_eq!(hovered.as_ref(), Some(&first));

        // Cell B's enter can arrive before cell A's leave; the stale leave must
        // not blank the footer.
        let hovered = next_hovered(hovered.as_ref(), HoverUpdate::Enter(&second));
        let hovered = next_hovered(hovered.as_ref(), HoverUpdate::Leave);
        assert_eq!(hovered.as_ref(), Some(&second));

        // Leaving the grid keeps the last preview until the rows rebuild.
        let hovered = next_hovered(hovered.as_ref(), HoverUpdate::Leave);
        assert_eq!(hovered.as_ref(), Some(&second));
    }

    #[test]
    fn hover_resets_when_the_rows_rebuild() {
        let first = entry("first");
        let hovered = next_hovered(None, HoverUpdate::Enter(&first));
        assert!(hovered.is_some());
        assert_eq!(next_hovered(hovered.as_ref(), HoverUpdate::Reset), None);
        assert_eq!(next_hovered(None, HoverUpdate::Reset), None);
    }
}
