mod sections;
mod themes;

pub(super) use sections::message_display;
use sections::{chat_box, messages, search, streamer};
use themes::theme_section;

use std::rc::Rc;

use gpui::{prelude::*, *};
use gpui_component::{
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    slider::{Slider, SliderEvent, SliderState},
};

use super::{
    SettingsPage, moved,
    sections::{row, section, segmented, toggle_row},
};
use crate::{
    app::{DiscordApp, appearance::chosen_base, calls::voice::switch},
    discord::settings_proto::{SpoilerMode, TextImages},
    settings::{self, SavedClientTheme, StreamerMode},
    theme::{self, BaseTheme, Gradient, gradient, rpx},
};

const MAX_CUSTOM_COLORS: usize = 5;
const DEFAULT_CUSTOM: [u32; 2] = [0x5865F2, 0xEB459E];

pub(crate) struct CustomEditor {
    colors: Vec<Entity<ColorPickerState>>,
    angle: Entity<SliderState>,
    mix: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

fn packed(color: Hsla) -> u32 {
    let color: Rgba = color.into();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

impl DiscordApp {
    fn open_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (colors, angle, base_mix) = match settings::get(cx).client_theme {
            Some(SavedClientTheme::Custom {
                colors,
                angle,
                base_mix,
            }) => (colors, angle, base_mix),
            _ => (DEFAULT_CUSTOM.to_vec(), 180, 74),
        };
        let angle = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(360.)
                .step(1.)
                .default_value(angle as f32)
        });
        let mix = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(base_mix as f32)
        });
        let mut subscriptions = vec![
            cx.subscribe(&angle, |app, _, event: &SliderEvent, cx| {
                app.custom_theme_edited(matches!(event, SliderEvent::Release(_)), cx);
            }),
            cx.subscribe(&mix, |app, _, event: &SliderEvent, cx| {
                app.custom_theme_edited(matches!(event, SliderEvent::Release(_)), cx);
            }),
        ];
        let colors: Vec<_> = colors
            .into_iter()
            .map(|color| {
                let picker =
                    cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(color)));
                subscriptions.push(Self::watch_custom_color(&picker, cx));
                picker
            })
            .collect();
        if let Some(page) = &mut self.settings_page {
            page.custom_theme = Some(CustomEditor {
                colors,
                angle,
                mix,
                _subscriptions: subscriptions,
            });
        }
        self.custom_theme_edited(true, cx);
    }

    fn watch_custom_color(
        picker: &Entity<ColorPickerState>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(picker, |app, _, event: &ColorPickerEvent, cx| {
            let ColorPickerEvent::Change(_) = event;
            app.custom_theme_edited(true, cx);
        })
    }

    fn add_custom_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self
            .settings_page
            .as_ref()
            .and_then(|page| page.custom_theme.as_ref())
        else {
            return;
        };
        if editor.colors.len() >= MAX_CUSTOM_COLORS {
            return;
        }
        let last = editor
            .colors
            .last()
            .and_then(|picker| picker.read(cx).value())
            .map_or(DEFAULT_CUSTOM[0], packed);
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(last)));
        let subscription = Self::watch_custom_color(&picker, cx);
        if let Some(editor) = self
            .settings_page
            .as_mut()
            .and_then(|page| page.custom_theme.as_mut())
        {
            editor.colors.push(picker);
            editor._subscriptions.push(subscription);
        }
        self.custom_theme_edited(true, cx);
    }

    fn remove_custom_color(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self
            .settings_page
            .as_mut()
            .and_then(|page| page.custom_theme.as_mut())
            && editor.colors.len() > 1
        {
            editor.colors.pop();
        }
        self.custom_theme_edited(true, cx);
    }

    fn custom_theme_edited(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(editor) = self
            .settings_page
            .as_ref()
            .and_then(|page| page.custom_theme.as_ref())
        else {
            return;
        };
        let colors: Vec<u32> = editor
            .colors
            .iter()
            .filter_map(|picker| picker.read(cx).displayed_color())
            .map(packed)
            .collect();
        if colors.is_empty() {
            return;
        }
        let theme = SavedClientTheme::Custom {
            colors,
            angle: editor.angle.read(cx).value().end().round() as i32,
            base_mix: editor.mix.read(cx).value().end().round() as i32,
        };
        if commit {
            self.choose_client_theme(theme, cx);
        } else {
            settings::update(cx, |settings| settings.client_theme = Some(theme));
            crate::app::apply_saved_theme(cx);
        }
        cx.notify();
    }

    fn close_custom_theme(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.settings_page {
            page.custom_theme = None;
        }
        cx.notify();
    }
}

pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    vec![
        theme_section(app, settings, cx),
        messages(app, settings),
        chat_box(app, settings, cx),
        search(app),
        streamer(app, settings),
    ]
}

fn group_heading(text: &'static str, subtitle: Option<&'static str>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .mt(rpx(24.))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::text()))
                .child(text),
        )
        .children(subtitle.map(|subtitle| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::muted()))
                .child(subtitle)
        }))
        .into_any_element()
}

fn legend(text: &'static str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .h(rpx(28.))
        .mt(rpx(8.))
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(text)
        .into_any_element()
}

fn related_accessibility(
    app: &DiscordApp,
    id: &'static str,
    description: &'static str,
) -> AnyElement {
    related_setting(
        app,
        id,
        super::Page::Accessibility,
        "accessibility",
        "Accessibility",
        description,
    )
}

/// A "Related Settings" link that opens another settings page.
pub(super) fn related_setting(
    app: &DiscordApp,
    id: &'static str,
    page: super::Page,
    icon: &'static str,
    title: &'static str,
    description: &'static str,
) -> AnyElement {
    let entity = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .mt(rpx(16.))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::faint()))
                .child("Related Settings"),
        )
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(rpx(12.))
                .mx(rpx(-12.))
                .p(rpx(12.))
                .rounded(rpx(16.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::hover())))
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.select_settings_page(page, None, cx))
                        .ok();
                })
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(48.))
                        .rounded(rpx(8.))
                        .bg(rgba(theme::control()))
                        .child(theme::icon(icon, 24., theme::text())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rpx(4.))
                        .child(
                            div()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::strong()))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(theme::muted()))
                                .child(description),
                        ),
                )
                .child(theme::icon("chevron-right", 24., theme::text())),
        )
        .into_any_element()
}

fn display_toggle(
    app: &DiscordApp,
    settings: &SettingsPage,
    id: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    on: bool,
    set: fn(&mut TextImages, bool),
) -> AnyElement {
    let entity = app.entity.clone();
    toggle_row(app, settings, id, label, description, on, move |cx, on| {
        entity
            .update(cx, |app, cx| {
                app.change_text_images(|display| set(display, on), cx)
            })
            .ok();
    })
}

fn disabled_row(
    settings: &SettingsPage,
    id: &'static str,
    label: &'static str,
    description: &'static str,
    on: bool,
) -> AnyElement {
    row(
        label,
        Some(description),
        div()
            .opacity(0.5)
            .child(switch(id, on, moved(settings, id)))
            .into_any_element(),
    )
}

/// One choice in a `select_field`; `choose` runs after the menu closes.
pub(super) type SelectOption = (&'static str, bool, Rc<dyn Fn(&mut App)>);

/// A dropdown whose open state lives in the `SettingsPage` field `flag` points at.
pub(super) fn select_field(
    app: &DiscordApp,
    id: &'static str,
    open: bool,
    flag: fn(&mut SettingsPage) -> &mut bool,
    options: Vec<SelectOption>,
) -> AnyElement {
    let label = options
        .iter()
        .find(|(_, chosen, _)| *chosen)
        .or(options.first())
        .map_or("", |(label, _, _)| label);
    let toggle = app.entity.clone();
    let field = div()
        .id(id)
        .flex()
        .items_center()
        .w(rpx(248.))
        .h(rpx(40.))
        .pl(rpx(12.))
        .pr(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(if open {
            rgb(theme::focus())
        } else {
            rgba(theme::border())
        })
        .on_click(move |_, _, cx| {
            toggle
                .update(cx, |app, cx| {
                    if let Some(page) = &mut app.settings_page {
                        let flag = flag(page);
                        *flag = !*flag;
                    }
                    cx.notify();
                })
                .ok();
        })
        .child(
            div()
                .flex_1()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_color(rgb(theme::strong()))
                .child(label),
        )
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            18.,
            theme::text(),
        ));
    let list = open.then(|| {
        let close = app.entity.clone();
        let options = options
            .into_iter()
            .enumerate()
            .map(|(index, (label, chosen, choose))| {
                let entity = app.entity.clone();
                div()
                    .id((id, index as u64))
                    .flex()
                    .items_center()
                    .h(rpx(40.))
                    .px(rpx(8.))
                    .rounded(rpx(6.))
                    .cursor_pointer()
                    .text_size(rpx(16.))
                    .text_color(rgb(theme::strong()))
                    .when(chosen, |this| this.bg(rgba(theme::hover())))
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                if let Some(page) = &mut app.settings_page {
                                    *flag(page) = false;
                                }
                                cx.notify();
                            })
                            .ok();
                        choose(cx);
                    })
                    .child(div().flex_1().child(label))
                    .when(chosen, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(rpx(20.))
                                .rounded_full()
                                .bg(rgb(theme::BLURPLE))
                                .child(theme::icon("check", 12., 0xFFFFFF)),
                        )
                    })
            });
        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                div()
                    .id((id, u64::MAX))
                    .w(rpx(248.))
                    .mt(rpx(4.))
                    .p(rpx(4.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close
                            .update(cx, |app, cx| {
                                if let Some(page) = &mut app.settings_page {
                                    *flag(page) = false;
                                }
                                cx.notify();
                            })
                            .ok();
                    })
                    .children(options),
            ),
        )
        .with_priority(1)
    });
    div()
        .flex()
        .flex_col()
        .child(field)
        .children(list.map(|list| div().h_0().child(list)))
        .into_any_element()
}

pub(super) fn radio(selected: bool) -> Div {
    let layer = |name, color: Rgba| {
        div()
            .absolute()
            .inset_0()
            .child(theme::icon(name, 20., 0).text_color(color))
    };
    div().relative().flex_none().size(rpx(20.)).map(|this| {
        if selected {
            this.child(layer("radio-disc", rgb(theme::BLURPLE)))
                .child(layer("radio-center", rgb(0xFFFFFF)))
        } else {
            this.child(layer("radio-outline", rgba((theme::muted() << 8) | 0xA3)))
        }
    })
}
