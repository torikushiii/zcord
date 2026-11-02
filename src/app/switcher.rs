use gpui_component::scroll::ScrollableElement as _;
use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    app::{DIALOG_EXIT, DiscordApp, message_body::label},
    media::images,
    quick_switcher::{self, Item, ItemKind, Section, SectionKind, Target},
    theme::{self, rpx},
};

const ENTER: Duration = Duration::from_millis(200);
const RISE: f32 = 8.;
const LIMIT: usize = 50;
const WIDTH: f32 = 695.;
const ROW_HEIGHT: f32 = 34.;
const RESULTS_HEIGHT: f32 = 234.;
const INPUT_HEIGHT: f32 = 70.;
const PANEL_BG: u32 = 0x0C0C0E;
const PANEL_BORDER: u32 = 0x26262B;
const INPUT_BG: u32 = 0x0F0F12;
const INPUT_BORDER: u32 = 0x2E2E34;
const SIGILS: [&str; 2] = ["#", "*"];

pub(crate) struct QuickSwitcher {
    search: Entity<InputState>,
    sections: Vec<Section>,
    selected: usize,
    scroll: ScrollHandle,
    closing: Option<Task<()>>,
    /// Unique per open, so reopening replays the entry animation.
    animation: u64,
    _search_events: Subscription,
}

impl QuickSwitcher {
    fn items(&self) -> impl Iterator<Item = &Item> {
        self.sections.iter().flat_map(|section| &section.items)
    }

    /// Index of the item's element among the results' children, which include section headers.
    fn child_index(&self, item: usize) -> usize {
        let mut seen = 0;
        let mut child = 0;
        for section in &self.sections {
            child += usize::from(header(section.kind).is_some());
            if item < seen + section.items.len() {
                return child + item - seen;
            }
            seen += section.items.len();
            child += section.items.len();
        }
        child
    }
}

impl DiscordApp {
    pub(crate) fn toggle_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_quick_switcher(window, cx) {
            return;
        }
        self.close_emoji_picker(cx);
        self.close_profile(cx);
        self.quick_switcher_opens = self.quick_switcher_opens.wrapping_add(1);
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Where would you like to go?"));
        let events = cx.subscribe_in(&search, window, |app, _, event, window, cx| match event {
            InputEvent::Change => app.rebuild_quick_switcher(cx),
            InputEvent::PressEnter { .. } => {
                let target = app.quick_switcher.as_ref().and_then(|switcher| {
                    switcher
                        .items()
                        .nth(switcher.selected)
                        .map(|item| item.target)
                });
                if let Some(target) = target {
                    app.quick_switch(target, window, cx);
                }
            }
            _ => {}
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        self.quick_switcher = Some(QuickSwitcher {
            search,
            sections: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            closing: None,
            animation: self.quick_switcher_opens,
            _search_events: events,
        });
        self.rebuild_quick_switcher(cx);
    }

    /// Returns false when there was no open switcher to close.
    pub(crate) fn close_quick_switcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(switcher) = &mut self.quick_switcher else {
            return false;
        };
        if switcher.closing.is_some() {
            return false;
        }
        let animation = switcher.animation;
        if cx.reduce_motion() {
            self.quick_switcher = None;
            self.focus_composer(window, cx);
            cx.notify();
            return true;
        }
        switcher.closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIALOG_EXIT).await;
            this.update(cx, |app, cx| {
                if app
                    .quick_switcher
                    .as_ref()
                    .is_some_and(|switcher| switcher.animation == animation)
                {
                    app.quick_switcher = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        self.focus_composer(window, cx);
        cx.notify();
        true
    }

    fn rebuild_quick_switcher(&mut self, cx: &mut Context<Self>) {
        let Some(switcher) = &self.quick_switcher else {
            return;
        };
        let query = switcher.search.read(cx).value().to_string();
        let sections = quick_switcher::search(
            &self.store.guilds,
            &query,
            &self.recent_channels,
            self.selected_guild_id,
            self.selected_channel_id,
            LIMIT,
        );
        if let Some(switcher) = &mut self.quick_switcher {
            switcher.sections = sections;
            switcher.selected = 0;
            switcher.scroll.scroll_to_item(0);
        }
        cx.notify();
    }

    /// Moves the highlighted result by `delta`, wrapping like Discord. Returns false when the
    /// switcher isn't open, so the arrow keys keep their usual meaning.
    pub(crate) fn move_quick_switcher(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(switcher) = self
            .quick_switcher
            .as_mut()
            .filter(|switcher| switcher.closing.is_none())
        else {
            return false;
        };
        let count = switcher.items().count();
        if count > 0 {
            switcher.selected =
                (switcher.selected as isize + delta).rem_euclid(count as isize) as usize;
            switcher
                .scroll
                .scroll_to_item(switcher.child_index(switcher.selected));
            cx.notify();
        }
        true
    }

    fn quick_switch(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_switcher(window, cx);
        match target {
            Target::Channel {
                guild_id,
                channel_id,
            } if guild_id == self.selected_guild_id => self.select_channel(channel_id, window, cx),
            Target::Channel {
                guild_id,
                channel_id,
            } => self.select_guild_channel(guild_id, Some(channel_id), window, cx),
            Target::Guild(id) if id == self.selected_guild_id => {}
            Target::Guild(id) => self.select_guild(id, window, cx),
        }
    }
}

fn header(kind: SectionKind) -> Option<&'static str> {
    match kind {
        SectionKind::Previous => Some("Previous Channels"),
        SectionKind::Mentions => Some("Mentions"),
        SectionKind::Results => None,
    }
}

pub(super) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let switcher = app.quick_switcher.as_ref()?;
    let closing = switcher.closing.is_some();
    let (key, duration) = if closing {
        ("quick-switcher-out", DIALOG_EXIT)
    } else {
        ("quick-switcher-in", ENTER)
    };
    let animation = Animation::new(duration).with_easing(ease_out_quint());
    let progress = move |delta: f32| if closing { 1. - delta } else { delta };

    let mut results = Vec::new();
    let mut index = 0;
    for section in &switcher.sections {
        if let Some(title) = header(section.kind) {
            results.push(
                div()
                    .px(rpx(12.))
                    .pt(rpx(12.))
                    .pb(rpx(6.))
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::MUTED))
                    .child(title)
                    .into_any_element(),
            );
        }
        for item in &section.items {
            results.push(item_row(app, item, index, index == switcher.selected, cx));
            index += 1;
        }
    }
    let query = switcher.search.read(cx).value();
    let empty = results.is_empty() && !query.trim().is_empty();

    let backdrop = app.entity.clone();
    let panel = div()
        .id("quick-switcher")
        .flex()
        .flex_col()
        .w(rpx(WIDTH))
        .max_w_full()
        .px(rpx(22.))
        .pt(rpx(22.))
        .pb(rpx(18.))
        .rounded(rpx(12.))
        .bg(rgb(PANEL_BG))
        .border_1()
        .border_color(rgb(PANEL_BORDER))
        .shadow_lg()
        .occlude()
        .child(
            // Input::h only sizes multi-line inputs, so the wrapper sets the height.
            div()
                .flex()
                .items_center()
                .h(rpx(INPUT_HEIGHT))
                .px(rpx(14.))
                .rounded(rpx(8.))
                .bg(rgb(INPUT_BG))
                .border_1()
                .border_color(rgb(INPUT_BORDER))
                .child(
                    Input::new(&switcher.search)
                        .appearance(false)
                        .px_0()
                        .text_size(rpx(24.)),
                ),
        )
        .when(!results.is_empty(), |this| {
            this.child(
                div()
                    .relative()
                    .mt(rpx(18.))
                    .child(
                        div()
                            .id("quick-switcher-results")
                            .max_h(rpx(RESULTS_HEIGHT))
                            .overflow_y_scroll()
                            .track_scroll(&switcher.scroll)
                            .children(results),
                    )
                    .vertical_scrollbar(&switcher.scroll),
            )
        })
        .when(empty, |this| {
            this.child(
                div()
                    .py(rpx(24.))
                    .text_center()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::MUTED))
                    .child("No results found"),
            )
        })
        .child(protip())
        .with_animation(
            (key, switcher.animation),
            animation.clone(),
            move |this, delta| {
                let progress = progress(delta);
                this.relative()
                    .top(px((1. - progress) * RISE))
                    .opacity(progress)
            },
        );
    Some(
        div()
            .id("quick-switcher-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p(rpx(40.))
            .occlude()
            .on_click(move |_, window, cx| {
                backdrop
                    .update(cx, |app, cx| app.close_quick_switcher(window, cx))
                    .ok();
            })
            .with_animation((key, switcher.animation), animation, move |this, delta| {
                this.bg(hsla(0., 0., 0., 0.7 * progress(delta)))
            })
            .child(panel)
            .into_any_element(),
    )
}

fn item_row(app: &DiscordApp, item: &Item, index: usize, selected: bool, cx: &App) -> AnyElement {
    let hover = app.entity.clone();
    let click = app.entity.clone();
    let target = item.target;
    let bright = selected || item.unread || item.mentions > 0;
    let icon = match item.kind {
        ItemKind::Guild => guild_icon(app, target, cx),
        kind => theme::icon(
            match kind {
                ItemKind::Announcement => "megaphone",
                ItemKind::Forum => "messages-square",
                ItemKind::Thread => "message-square",
                _ => "hash",
            },
            20.,
            if bright { theme::TEXT } else { theme::FAINT },
        )
        .into_any_element(),
    };
    let badge = (item.mentions > 0).then(|| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(rpx(16.))
            .min_w(rpx(16.))
            .px(rpx(4.))
            .rounded(rpx(4.))
            .bg(rgb(theme::RED))
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .font_weight(FontWeight::BOLD)
            .text_color(white())
            .child(if item.mentions >= 1000 {
                format!("{}K+", item.mentions / 1000)
            } else {
                item.mentions.to_string()
            })
    });
    let context = item.context.as_ref().map(|context| {
        div()
            .min_w_0()
            .truncate()
            .text_size(rpx(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::MUTED))
            .child(label(("quick-switcher-context", index), context))
    });
    let guild = item.guild_name.as_ref().map(|name| {
        div()
            .flex_none()
            .max_w(rpx(180.))
            .truncate()
            .text_size(rpx(14.))
            .text_color(rgb(theme::MUTED))
            .child(label(("quick-switcher-guild", index), name))
    });
    div()
        .id(("quick-switcher-item", index))
        .flex()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(ROW_HEIGHT))
        .px(rpx(12.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .when(selected, |this| this.bg(rgba(theme::SELECTED)))
        .on_mouse_move(move |_, _, cx| {
            hover
                .update(cx, |app, cx| {
                    if let Some(switcher) = &mut app.quick_switcher
                        && switcher.selected != index
                    {
                        switcher.selected = index;
                        cx.notify();
                    }
                })
                .ok();
        })
        .on_click(move |_, window, cx| {
            click
                .update(cx, |app, cx| app.quick_switch(target, window, cx))
                .ok();
        })
        .child(div().flex_none().mr(rpx(2.)).child(icon))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(if item.unread || item.mentions > 0 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(if bright { theme::STRONG } else { theme::MUTED }))
                .child(label(("quick-switcher-name", index), &item.name)),
        )
        .children(badge)
        .children(context)
        .child(div().flex_1())
        .children(guild)
        .into_any_element()
}

fn guild_icon(app: &DiscordApp, target: Target, cx: &App) -> AnyElement {
    let Target::Guild(id) = target else {
        return div().into_any_element();
    };
    let guild = app.store.guilds.iter().find(|guild| guild.id == id);
    let image = guild
        .and_then(|guild| guild.icon_spec())
        .and_then(|spec| images::resolve(&spec, theme::image_px(20.), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(20.))
            .flex_none()
            .rounded_full()
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(20.))
            .rounded_full()
            .bg(rgba(theme::SELECTED))
            .text_size(rpx(8.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::TEXT))
            .children(guild.map(|guild| guild.initials.clone()))
            .into_any_element(),
    }
}

fn protip() -> impl IntoElement {
    let sigil = |text: &'static str| {
        div()
            .px(rpx(4.))
            .rounded(rpx(3.))
            .bg(rgb(theme::BG))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::STRONG))
            .child(text)
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .mt(rpx(24.))
        .text_size(rpx(13.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::TEXT))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::GREEN))
                .child("Protip:"),
        )
        .child("Start searches with")
        .children(SIGILS.map(sigil))
        .child("to narrow results.")
}
