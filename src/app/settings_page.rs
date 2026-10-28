use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    app::{DiscordApp, profile},
    theme::{self, rpx},
};

mod sections;
mod voice_audio;

pub(crate) use voice_audio::apply_preferences;

const ENTER: Duration = Duration::from_millis(180);
const EXIT: Duration = Duration::from_millis(120);
const MARGIN: f32 = 32.;
const MAX_WIDTH: f32 = 1400.;
const MAX_HEIGHT: f32 = 1030.;
const SIDEBAR_WIDTH: f32 = 252.;
const SIDEBAR_MUTED: u32 = theme::MUTED;
const SIDEBAR_STRONG: u32 = theme::STRONG;
const SETTINGS_TEXT: u32 = theme::TEXT;
/// Discord's settings borders: its muted grey at 20% opacity.
const EDGE: u32 = theme::BORDER_SUBTLE;
const CONTENT_WIDTH: f32 = 784.;
/// Voice & Video's measured panel includes 16px padding on each side.
const VOICE_CONTENT_WIDTH: f32 = 728.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Account,
    VoiceVideo,
    Appearance,
    Accessibility,
    Chat,
    LanguageTime,
    Advanced,
}

struct PageInfo {
    page: Page,
    group: &'static str,
    label: &'static str,
    icon: &'static str,
    /// Headings inside the page, listed under it in the sidebar while it's selected.
    sections: &'static [&'static str],
    /// Extra words the search box matches, beyond the label and section names.
    keywords: &'static [&'static str],
}

const PAGES: &[PageInfo] = &[
    PageInfo {
        page: Page::Account,
        group: "User Settings",
        label: "Account",
        icon: "user",
        sections: &["Account Info", "Password & Security"],
        keywords: &["username", "email", "phone", "two-factor", "mfa", "2fa"],
    },
    PageInfo {
        page: Page::VoiceVideo,
        group: "Experience",
        label: "Voice & Video",
        icon: "mic",
        sections: &[],
        keywords: &[
            "noise suppression",
            "microphone",
            "input",
            "output",
            "speaker",
            "audio",
            "volume",
            "sensitivity",
            "mic test",
        ],
    },
    PageInfo {
        page: Page::Appearance,
        group: "Experience",
        label: "Appearance",
        icon: "palette",
        sections: &[],
        keywords: &["zoom", "scale", "size", "font"],
    },
    PageInfo {
        page: Page::Accessibility,
        group: "Experience",
        label: "Accessibility",
        icon: "accessibility",
        sections: &[],
        keywords: &["reduce motion", "animation"],
    },
    PageInfo {
        page: Page::Chat,
        group: "Experience",
        label: "Chat",
        icon: "message-square",
        sections: &[],
        keywords: &[
            "gif",
            "klipy",
            "tenor",
            "giphy",
            "autoplay",
            "emoji",
            "link preview",
            "embed",
        ],
    },
    PageInfo {
        page: Page::LanguageTime,
        group: "Experience",
        label: "Language & Time",
        icon: "languages",
        sections: &[],
        keywords: &["clock", "24-hour", "12-hour", "time format"],
    },
    PageInfo {
        page: Page::Advanced,
        group: "App Settings",
        label: "Advanced",
        icon: "settings",
        sections: &[],
        keywords: &["startup", "launch", "windows", "cache", "data folder"],
    },
];

pub(crate) struct SettingsPage {
    page: Page,
    search: Entity<InputState>,
    scroll: ScrollHandle,
    reveal_email: bool,
    reveal_phone: bool,
    closing: bool,
    /// Unique per open, so reopening replays the entry animation.
    animation: u64,
    /// Switches flipped since the dialog opened; only those slide, the rest render at rest.
    moved_switches: Vec<&'static str>,
    voice_audio: Option<voice_audio::VoiceAudio>,
    /// Read once per open; the registry doesn't change behind our back.
    launch_on_startup: bool,
    cache_status: Option<SharedString>,
    _cache_clear: Option<Task<()>>,
    _search_events: Subscription,
    _close: Option<Task<()>>,
}

impl DiscordApp {
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .settings_page
            .as_ref()
            .is_some_and(|page| !page.closing)
        {
            return;
        }
        // The search field draws its placeholder using the sidebar's own muted color.
        let search = cx.new(|cx| InputState::new(window, cx));
        let events = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        });
        self.settings_animation = self.settings_animation.wrapping_add(1);
        self.settings_page = Some(SettingsPage {
            page: Page::Account,
            search,
            scroll: ScrollHandle::new(),
            reveal_email: false,
            reveal_phone: false,
            closing: false,
            animation: self.settings_animation,
            moved_switches: Vec::new(),
            voice_audio: None,
            launch_on_startup: crate::startup::enabled(),
            cache_status: None,
            _cache_clear: None,
            _search_events: events,
            _close: None,
        });
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(page) = self.settings_page.as_mut().filter(|page| !page.closing) else {
            return false;
        };
        page.closing = true;
        let animation = page.animation;
        page._close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EXIT).await;
            this.update(cx, |app, cx| {
                if app
                    .settings_page
                    .as_ref()
                    .is_some_and(|page| page.closing && page.animation == animation)
                {
                    app.settings_page = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
        true
    }

    fn select_settings_page(&mut self, page: Page, section: Option<usize>, cx: &mut Context<Self>) {
        let Some(settings) = &mut self.settings_page else {
            return;
        };
        if settings.page != page {
            settings.page = page;
            settings.scroll.set_offset(point(px(0.), px(0.)));
        }
        if let Some(section) = section {
            settings.scroll.scroll_to_item(section);
        }
        if page == Page::VoiceVideo {
            self.ensure_voice_audio(cx);
        }
        cx.notify();
    }

    fn toggle_launch_on_startup(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = &mut self.settings_page else {
            return;
        };
        let on = !settings.launch_on_startup;
        match crate::startup::set_enabled(on) {
            Ok(()) => settings.launch_on_startup = on,
            Err(error) => log::warn!("Could not change launch on startup: {error}"),
        }
        self.mark_switch_moved("settings-startup");
        cx.notify();
    }

    fn clear_image_cache(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = &mut self.settings_page else {
            return;
        };
        if settings._cache_clear.is_some() {
            return;
        }
        settings.cache_status = Some("Clearing…".into());
        let task = crate::media::images::clear_cache(cx);
        if let Some(settings) = &mut self.settings_page {
            settings._cache_clear = Some(cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |app, cx| {
                    if let Some(settings) = &mut app.settings_page {
                        settings.cache_status = Some(match result {
                            Ok(bytes) => format!("Cleared {}.", format_bytes(bytes)).into(),
                            Err(error) => format!("Couldn't clear the cache: {error}").into(),
                        });
                        settings._cache_clear = None;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn mark_switch_moved(&mut self, id: &'static str) {
        if let Some(settings) = &mut self.settings_page
            && !settings.moved_switches.contains(&id)
        {
            settings.moved_switches.push(id);
        }
    }
}

pub(crate) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let settings = app.settings_page.as_ref()?;
    let closing = settings.closing;
    let reduce_motion = cx.reduce_motion();
    let close = app.entity.clone();
    // Discord caps the dialog at 1400×1030 and centres it, keeping at least a 32px margin.
    let css = |length: Pixels| length / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let side = ((css(viewport.width) - MAX_WIDTH) / 2.).max(MARGIN);
    let vertical = ((css(viewport.height) - MAX_HEIGHT) / 2.).max(MARGIN);
    let dialog = div()
        .id("settings-dialog")
        .absolute()
        .top(rpx(vertical))
        .bottom(rpx(vertical))
        .left(rpx(side))
        .right(rpx(side))
        .flex()
        .rounded(rpx(12.))
        .overflow_hidden()
        .bg(black())
        .border_1()
        .border_color(rgba(EDGE))
        .shadow_lg()
        .when(!closing, |this| this.occlude())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(sidebar(app, settings, cx))
        .child(content(app, settings, cx));
    let dialog = if reduce_motion {
        dialog.into_any_element()
    } else {
        let animation =
            Animation::new(if closing { EXIT } else { ENTER }).with_easing(ease_out_quint());
        let key = if closing {
            "settings-out"
        } else {
            "settings-in"
        };
        dialog
            .with_animation((key, settings.animation), animation, move |this, delta| {
                let progress = if closing { 1. - delta } else { delta };
                this.opacity(progress).mt(rpx((1. - progress) * 12.))
            })
            .into_any_element()
    };
    Some(
        div()
            .id("settings-backdrop")
            .absolute()
            .inset_0()
            .when(!closing, |this| {
                this.occlude().on_click(move |_, _, cx| {
                    close.update(cx, |app, cx| app.close_settings(cx)).ok();
                })
            })
            .bg(rgba(theme::SCRIM).opacity(if closing { 0. } else { 1. }))
            .child(dialog)
            .into_any_element(),
    )
}

fn search_query(settings: &SettingsPage, cx: &App) -> String {
    settings.search.read(cx).value().trim().to_lowercase()
}

fn page_matches(info: &PageInfo, query: &str) -> bool {
    query.is_empty()
        || info.label.to_lowercase().contains(query)
        || info
            .sections
            .iter()
            .chain(info.keywords)
            .any(|word| word.to_lowercase().contains(query))
}

fn sidebar(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> impl IntoElement {
    let query = search_query(settings, cx);
    let user = app.store.user(app.store.current_user_id);
    let user_id = app.store.current_user_id;
    let header = div()
        .id("settings-profile")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(64.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::HOVER)))
        .on_click(profile::opener(app, user_id, Anchor::TopLeft))
        .child(theme::user_avatar(user, user_id, 48., None, false, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(SIDEBAR_STRONG))
                        .child(user.map_or_else(String::new, |user| user.name.clone())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(SIDEBAR_MUTED))
                        .child("View Profile")
                        .child(theme::icon("pencil", 12., SIDEBAR_MUTED)),
                ),
        );
    let search_empty = settings.search.read(cx).value().is_empty();
    let search = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(40.))
        .pl(rpx(4.))
        .pr(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgba(0x0000001F))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .text_color(rgb(SETTINGS_TEXT))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(rpx(32.))
                .h(rpx(38.))
                .child(theme::icon("search", 16., SETTINGS_TEXT)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h(rpx(38.))
                .children(search_empty.then(|| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(SIDEBAR_MUTED))
                        .child("Search")
                }))
                .child(
                    Input::new(&settings.search)
                        .appearance(false)
                        .aria_label("Search settings")
                        .h_full()
                        .px_0()
                        .py_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(SETTINGS_TEXT)),
                ),
        );
    let pages: Vec<_> = PAGES
        .iter()
        .filter(|info| page_matches(info, &query))
        .collect();
    let mut items: Vec<AnyElement> = pages
        .chunk_by(|a, b| a.group == b.group)
        .map(|group| {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(rpx(4.))
                .pb(rpx(8.))
                .border_b_1()
                .border_color(rgba(EDGE))
                .child(
                    div()
                        .flex_none()
                        .h(rpx(30.))
                        .px(rpx(8.))
                        .pt(rpx(8.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(SIDEBAR_MUTED))
                        .child(group[0].group),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(4.))
                        .children(group.iter().map(|info| sidebar_item(app, settings, info))),
                )
                .into_any_element()
        })
        .collect();
    if items.is_empty() {
        items.push(
            div()
                .px(rpx(8.))
                .py(rpx(12.))
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child("No results")
                .into_any_element(),
        );
    }
    let log_out = {
        let entity = app.entity.clone();
        div()
            .id("settings-log-out")
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(12.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::HOVER)))
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .text_color(rgb(theme::RED))
            .on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.settings_page = None;
                        app.log_out(window, cx);
                    })
                    .ok();
            })
            .child(theme::icon("log-out", 20., theme::RED))
            .child("Log Out")
    };
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(SIDEBAR_WIDTH))
        .h_full()
        .pt(rpx(16.))
        .pl(rpx(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .pl(rpx(4.))
                .pr(rpx(16.))
                .child(div().pb(rpx(8.)).child(header))
                .child(search),
        )
        .child(
            div()
                .id("settings-sidebar")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(rpx(24.))
                .pt(rpx(12.))
                .pb(rpx(16.))
                .pl(rpx(4.))
                .pr(rpx(16.))
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .gap(rpx(8.))
                        .children(items)
                        .when(query.is_empty(), |this| this.child(log_out)),
                )
                .when(query.is_empty(), |this| this.child(footer())),
        )
        .child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(1.))
                .bg(rgba(EDGE)),
        )
}

fn sidebar_item(app: &DiscordApp, settings: &SettingsPage, info: &'static PageInfo) -> Div {
    let selected = settings.page == info.page;
    let foreground = if selected {
        SIDEBAR_STRONG
    } else {
        SIDEBAR_MUTED
    };
    let page = info.page;
    let entity = app.entity.clone();
    let row = div()
        .id(info.label)
        .flex()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(36.))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .text_color(rgb(foreground))
        .map(|this| {
            if selected {
                this.bg(rgba(theme::ACTIVE))
            } else {
                this.hover(|style| style.bg(rgba(theme::HOVER)))
            }
        })
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.select_settings_page(page, None, cx))
                .ok();
        })
        .child(theme::icon(info.icon, 20., foreground))
        .child(info.label);
    let subsections = selected.then(|| {
        div()
            .flex()
            .flex_col()
            .ml(rpx(18.))
            .my(rpx(4.))
            .border_l_1()
            .border_color(rgba(theme::BORDER))
            .children(info.sections.iter().enumerate().map(|(index, name)| {
                let entity = app.entity.clone();
                div()
                    .id((info.label, index as u64))
                    .flex()
                    .items_center()
                    .h(rpx(36.))
                    .pl(rpx(13.))
                    .cursor_pointer()
                    .text_size(rpx(16.))
                    .text_color(rgb(SIDEBAR_MUTED))
                    .hover(|style| style.text_color(rgb(theme::TEXT)))
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.select_settings_page(page, Some(index), cx)
                            })
                            .ok();
                    })
                    .child(*name)
            }))
    });
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(row)
        .children(subsections.filter(|_| !info.sections.is_empty()))
}

fn footer() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(rpx(4.))
        .text_size(rpx(10.))
        .line_height(rpx(12.))
        .text_color(rgb(SIDEBAR_MUTED))
        .child(format!("zcord {}", env!("CARGO_PKG_VERSION")))
        .child(
            div().flex().gap(rpx(4.)).child("GPL-3.0").child("·").child(
                div()
                    .id("settings-github")
                    .cursor_pointer()
                    .text_color(rgb(crate::app::chat::LINK))
                    .hover(|style| style.underline())
                    .on_click(|_, _, cx| cx.open_url("https://github.com/torikushiii/zcord"))
                    .child("GitHub"),
            ),
        )
}

fn content(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> impl IntoElement {
    let info = PAGES
        .iter()
        .find(|info| info.page == settings.page)
        .unwrap_or(&PAGES[0]);
    let voice_page = settings.page == Page::VoiceVideo;
    let content_width = if voice_page {
        VOICE_CONTENT_WIDTH
    } else {
        CONTENT_WIDTH
    };
    let close = app.entity.clone();
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(rpx(49.))
        .pl(rpx(16.))
        .pr(rpx(8.))
        .border_b_1()
        .border_color(rgba(theme::BORDER_MUTED))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(SETTINGS_TEXT))
                .child(info.label),
        )
        .child(
            theme::icon_button("settings-close", "x", 16., theme::MUTED)
                .size(rpx(32.))
                .rounded(rpx(8.))
                .on_click(move |_, _, cx| {
                    close.update(cx, |app, cx| app.close_settings(cx)).ok();
                }),
        );
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(header)
        .child(
            div()
                .id("settings-content")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&settings.scroll)
                .flex()
                .flex_col()
                .px(rpx(if voice_page { 0. } else { 40. }))
                .pt(rpx(if voice_page { 64. } else { 48. }))
                .pb(rpx(80.))
                // Each section is a direct child so the sidebar can scroll to it by index.
                .children(
                    sections::page(app, settings, cx)
                        .into_iter()
                        .map(|section| {
                            div()
                                .flex_none()
                                .mx_auto()
                                .w_full()
                                .max_w(rpx(content_width))
                                .px(rpx(if voice_page { 16. } else { 0. }))
                                .child(section)
                        }),
                ),
        )
}

fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024. * 1024.;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn moved(settings: &SettingsPage, id: &'static str) -> Option<u64> {
    settings
        .moved_switches
        .contains(&id)
        .then_some(settings.animation)
}
