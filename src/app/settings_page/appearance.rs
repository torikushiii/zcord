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
    app::{DiscordApp, appearance::chosen_base, voice::switch},
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

const SWATCH: f32 = 48.;
const SWATCH_GAP: f32 = 8.;
const SWATCHES_PER_ROW: usize = 12;
const CARD_SWATCHES_PER_ROW: usize = 11;

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

fn swatch(
    id: impl Into<ElementId>,
    name: &'static str,
    fill: AnyElement,
    outlined: bool,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let id: ElementId = id.into();
    let tip = ElementId::Name(format!("{id}-tip").into());
    div()
        .id(id)
        .group("theme-swatch")
        .relative()
        .flex_none()
        .size(rpx(SWATCH))
        .cursor_pointer()
        .on_click(on_click)
        .child(theme::hover_tooltip(tip, name))
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(8.))
                .overflow_hidden()
                .child(fill),
        )
        .when(outlined, |this| {
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(rpx(8.))
                    .border_1()
                    .border_color(rgb(theme::muted())),
            )
        })
        .when(selected, |this| {
            this.child(
                div()
                    .absolute()
                    .top(rpx(-2.))
                    .left(rpx(-2.))
                    .size(rpx(SWATCH + 4.))
                    .rounded(rpx(8.))
                    .border(rpx(4.))
                    .border_color(rgb(theme::BLURPLE)),
            )
            .child(
                div()
                    .absolute()
                    .top(rpx(-8.))
                    .right(rpx(-8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(24.))
                    .rounded_full()
                    .bg(rgb(theme::BLURPLE))
                    .border(rpx(2.))
                    .border_color(rgb(theme::bg()))
                    .child(theme::icon("check", 14., 0xFFFFFF)),
            )
        })
        .into_any_element()
}

fn solid(color: u32) -> AnyElement {
    div()
        .size_full()
        .rounded(rpx(8.))
        .bg(rgb(color))
        .into_any_element()
}

fn painted(gradient: &Gradient) -> AnyElement {
    div()
        .relative()
        .size_full()
        .child(gradient.tile(rpx(8.)))
        .into_any_element()
}

fn icon_fill(icon: &'static str, background: u32) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgb(background))
        .child(theme::icon(icon, 24., theme::text()))
        .into_any_element()
}

/// Explicit rows: a wrapping flex row is measured as one line inside the settings column.
fn swatch_grid(children: Vec<AnyElement>, per_row: usize) -> AnyElement {
    let mut rows = Vec::new();
    let mut children = children.into_iter().peekable();
    while children.peek().is_some() {
        rows.push(
            div()
                .flex()
                .gap(rpx(SWATCH_GAP))
                .children(children.by_ref().take(per_row)),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(rpx(SWATCH_GAP))
        .mt(rpx(24.))
        .children(rows)
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

fn theme_section(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let saved = settings::get(cx);
    let follows_device = saved.theme_follows_device;
    let chosen = chosen_base(&saved);
    let mut defaults: Vec<AnyElement> = [
        (BaseTheme::Light, "Light", 0xFFFFFF),
        (BaseTheme::Ash, "Ash", 0x323339),
        (BaseTheme::Dark, "Dark", 0x1D1D21),
        (BaseTheme::Onyx, "Onyx", 0x000000),
    ]
    .into_iter()
    .map(|(base, name, color)| {
        let entity = app.entity.clone();
        swatch(
            ("settings-base-theme", base as u64),
            name,
            solid(color),
            true,
            !follows_device && saved.client_theme.is_none() && chosen == base,
            move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.close_custom_theme(cx);
                        app.choose_theme(base, cx);
                    })
                    .ok();
            },
        )
    })
    .collect();
    defaults.push({
        let entity = app.entity.clone();
        swatch(
            "settings-base-theme-sync",
            "Sync with computer",
            icon_fill("rotate-cw", 0x000000),
            true,
            follows_device,
            move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.set_theme_follows_device(true, cx))
                    .ok();
            },
        )
    });

    let custom_selected = matches!(saved.client_theme, Some(SavedClientTheme::Custom { .. }));
    let mut colors = vec![{
        let entity = app.entity.clone();
        let fill = match (&saved.client_theme, theme::gradient()) {
            (Some(SavedClientTheme::Custom { .. }), Some(gradient)) => painted(&gradient),
            _ => icon_fill("palette", theme::surface_high()),
        };
        swatch(
            "settings-custom-theme",
            "Custom Theme",
            fill,
            true,
            custom_selected,
            move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.open_custom_theme(window, cx))
                    .ok();
            },
        )
    }];
    colors.extend(gradient::PRESETS.iter().map(|preset| {
        let entity = app.entity.clone();
        let id = preset.id;
        swatch(
            ("settings-color-theme", id as u64),
            preset.name,
            painted(&Gradient::from_preset(preset)),
            false,
            saved.client_theme == Some(SavedClientTheme::Preset(id)),
            move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.close_custom_theme(cx);
                        app.choose_client_theme(SavedClientTheme::Preset(id), cx);
                    })
                    .ok();
            },
        )
    }));

    let follow = {
        let entity = app.entity.clone();
        toggle_row(
            app,
            settings,
            "settings-theme-device",
            "Same as Device Theme",
            Some("Match your device's light or dark mode."),
            follows_device,
            move |cx, on| {
                entity
                    .update(cx, |app, cx| app.set_theme_follows_device(on, cx))
                    .ok();
            },
        )
    };
    let sync = {
        let entity = app.entity.clone();
        toggle_row(
            app,
            settings,
            "settings-theme-sync",
            "Sync theme across my devices",
            None,
            !saved.theme_sync_off,
            move |cx, on| {
                entity.update(cx, |app, cx| app.set_theme_sync(on, cx)).ok();
            },
        )
    };
    let profiles = toggle_row(
        app,
        settings,
        "settings-profile-theme-sync",
        "Apply theme to other users' profiles",
        None,
        settings::profile_theme_sync(),
        settings::set_profile_theme_sync,
    );

    let mut rows = vec![
        follow,
        group_heading("Default Themes", None),
        swatch_grid(defaults, SWATCHES_PER_ROW),
    ];
    if app.can_sync_client_theme() {
        rows.extend([
            group_heading("Color Themes", Some("Make Discord yours. Only with Nitro.")),
            swatch_grid(colors, SWATCHES_PER_ROW),
        ]);
    } else {
        rows.push(nitro_color_themes(colors));
    }
    if let Some(editor) = &settings.custom_theme {
        rows.push(custom_editor(app, editor, cx));
    }
    rows.push(div().h(rpx(16.)).into_any_element());
    rows.extend([sync, profiles]);
    rows.push(related_accessibility(
        app,
        "settings-related-theme",
        "Change role colors, on/off indicators, motion and more",
    ));
    section("Theme", rows)
}

fn nitro_color_themes(colors: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .mt(rpx(24.))
        .p(rpx(24.))
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgba((theme::muted() << 8) | 0x3D))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(2.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::text()))
                .child("Color Themes")
                .child(theme::icon("nitro", 16., theme::text())),
        )
        .child(swatch_grid(colors, CARD_SWATCHES_PER_ROW))
        .into_any_element()
}

fn custom_editor(app: &DiscordApp, editor: &CustomEditor, cx: &App) -> AnyElement {
    let add = app.entity.clone();
    let remove = app.entity.clone();
    let done = app.entity.clone();
    let can_add = editor.colors.len() < MAX_CUSTOM_COLORS;
    let can_remove = editor.colors.len() > 1;
    let step = |id: &'static str, icon: &'static str, enabled: bool| {
        theme::icon_button(
            id,
            icon,
            16.,
            if enabled {
                theme::text()
            } else {
                theme::faint()
            },
        )
        .size(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(theme::control_bg()))
    };
    let slider = |label: &'static str, state: &Entity<SliderState>, value: String| {
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(label)
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(theme::muted()))
                            .child(value),
                    ),
            )
            .child(
                div().h(rpx(20.)).child(
                    Slider::new(state)
                        .bg(rgb(theme::BLURPLE))
                        .text_color(white()),
                ),
            )
    };
    div()
        .mt(rpx(16.))
        .p(rpx(16.))
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgba(theme::control()))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Custom Theme"),
                )
                .child(
                    theme::icon_button("settings-custom-done", "x", 16., theme::muted())
                        .size(rpx(28.))
                        .rounded(rpx(8.))
                        .on_click(move |_, _, cx| {
                            done.update(cx, |app, cx| app.close_custom_theme(cx)).ok();
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .children(
                    editor
                        .colors
                        .iter()
                        .map(|picker| ColorPicker::new(picker).into_any_element()),
                )
                .child(step("settings-custom-remove", "minus", can_remove).when(
                    can_remove,
                    |this| {
                        this.on_click(move |_, _, cx| {
                            remove
                                .update(cx, |app, cx| app.remove_custom_color(cx))
                                .ok();
                        })
                    },
                ))
                .child(
                    step("settings-custom-add", "plus", can_add).when(can_add, |this| {
                        this.on_click(move |_, window, cx| {
                            add.update(cx, |app, cx| app.add_custom_color(window, cx))
                                .ok();
                        })
                    }),
                ),
        )
        .child(slider(
            "Gradient Angle",
            &editor.angle,
            format!("{:.0}°", editor.angle.read(cx).value().end()),
        ))
        .child(slider(
            "Color Mix",
            &editor.mix,
            format!("{:.0}%", editor.mix.read(cx).value().end()),
        ))
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

const SPOILER_MODES: [(&str, SpoilerMode); 3] = [
    ("On click", SpoilerMode::OnClick),
    ("On servers I moderate", SpoilerMode::IfModerator),
    ("Always", SpoilerMode::Always),
];

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

fn spoiler_select(app: &DiscordApp, settings: &SettingsPage, current: SpoilerMode) -> AnyElement {
    let options = SPOILER_MODES
        .iter()
        .map(|&(label, mode)| {
            let entity = app.entity.clone();
            let choose: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_text_images(|display| display.render_spoilers = mode, cx)
                    })
                    .ok();
            });
            (label, mode == current, choose)
        })
        .collect();
    select_field(
        app,
        "settings-spoilers",
        settings.spoiler_menu_open,
        |page| &mut page.spoiler_menu_open,
        options,
    )
}

fn messages(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let display = app.text_images();
    let avatars = if display.message_display_compact {
        display_toggle(
            app,
            settings,
            "settings-compact-avatars",
            "Show user avatars",
            None,
            display.display_compact_avatars,
            |display, on| display.display_compact_avatars = on,
        )
    } else {
        disabled_row(
            settings,
            "settings-compact-avatars",
            "Show user avatars",
            "To hide avatars, go to Accessibility and select Compact under Chat Message Display.",
            true,
        )
    };
    let favorites = {
        let entity = app.entity.clone();
        toggle_row(
            app,
            settings,
            "settings-favorites-server",
            "Show Favorites server",
            Some("Your Favorites stay saved, even if you toggle this off."),
            app.favorites_visible(),
            move |cx, on| {
                entity
                    .update(cx, |app, cx| app.change_favorites_visible(on, cx))
                    .ok();
            },
        )
    };
    section(
        "Messages",
        vec![
            legend("Show images, videos, and lolcats..."),
            display_toggle(
                app,
                settings,
                "settings-inline-embeds",
                "When posted as links to chat",
                None,
                display.inline_embed_media,
                |display, on| display.inline_embed_media = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-inline-attachments",
                "When uploaded directly to Discord",
                None,
                display.inline_attachment_media,
                |display, on| display.inline_attachment_media = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-render-embeds",
                "Show embeds and link previews",
                None,
                display.render_embeds,
                |display, on| display.render_embeds = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-render-reactions",
                "Show emoji reactions",
                None,
                display.render_reactions,
                |display, on| display.render_reactions = on,
            ),
            row(
                "Show spoiler content",
                None::<&str>,
                spoiler_select(app, settings, display.render_spoilers),
            ),
            display_toggle(
                app,
                settings,
                "settings-thread-sidebar",
                "Open threads in split view",
                None,
                display.use_thread_sidebar,
                |display, on| display.use_thread_sidebar = on,
            ),
            avatars,
            favorites,
            related_accessibility(
                app,
                "settings-related-messages",
                "Adjust message display and the space between message groups",
            ),
        ],
    )
}

fn chat_box(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let display = app.text_images();
    let send_button = crate::settings::get(cx).composer_send_button;
    section(
        "Chat Box",
        vec![
            display_toggle(
                app,
                settings,
                "settings-rich-input",
                "Preview emoji, mentions, and markdown syntax as you type",
                None,
                display.use_rich_chat_input,
                |display, on| display.use_rich_chat_input = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-convert-emoticons",
                "Automatically convert emoticons in your messages to emoji",
                Some("For example, when you type :) Discord will convert it to 🙂"),
                display.convert_emoticons,
                |display, on| display.convert_emoticons = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-sticker-suggestions",
                "Show stickers in autocomplete results",
                None,
                display.include_stickers_in_autocomplete,
                |display, on| display.include_stickers_in_autocomplete = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-game-suggestions",
                "Show games in autocomplete results",
                None,
                display.include_game_mentions_in_autocomplete,
                |display, on| display.include_game_mentions_in_autocomplete = on,
            ),
            toggle_row(
                app,
                settings,
                "settings-send-button",
                "Show send message button",
                None,
                send_button,
                |cx, on| {
                    crate::settings::update(cx, |settings| settings.composer_send_button = on);
                    cx.refresh_windows();
                },
            ),
        ],
    )
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

fn search(app: &DiscordApp) -> AnyElement {
    let cross = app.text_images().is_cross_dm_search_enabled;
    let options = [
        ("Searches only in the selected DM", false),
        ("Searches across all my DMs", true),
    ]
    .into_iter()
    .map(|(label, value)| {
        let entity = app.entity.clone();
        div()
            .id(("settings-dm-search", u64::from(value)))
            .flex()
            .items_center()
            .gap(rpx(12.))
            .h(rpx(20.))
            .cursor_pointer()
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .text_color(rgb(theme::text()))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_text_images(
                            |display| display.is_cross_dm_search_enabled = value,
                            cx,
                        )
                    })
                    .ok();
            })
            .child(radio(cross == value))
            .child(label)
            .into_any_element()
    });
    section(
        "Search",
        vec![
            div()
                .mt(rpx(4.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child("By default, searching in DMs...")
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .pt(rpx(16.))
                .pb(rpx(4.))
                .children(options)
                .into_any_element(),
        ],
    )
}

fn streamer(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let current = settings::streamer();
    let option =
        |id: &'static str, label: &'static str, on: bool, set: fn(&mut StreamerMode, bool)| {
            toggle_row(app, settings, id, label, None, on, move |cx, on| {
                let mut streamer = settings::streamer();
                set(&mut streamer, on);
                settings::set_streamer(cx, streamer);
            })
        };
    let mut rows = vec![
        toggle_row(
            app,
            settings,
            "settings-streamer",
            "Enable Streamer Mode",
            Some(
                "Streamer Mode hides your personal info so others watching your stream can't see it.",
            ),
            current.enabled,
            |cx, on| {
                settings::set_streamer(
                    cx,
                    StreamerMode {
                        enabled: on,
                        ..settings::streamer()
                    },
                );
            },
        ),
        option(
            "settings-streamer-auto",
            "Automatically enable Streamer Mode if OBS or XSplit is running",
            current.auto,
            |streamer, on| streamer.auto = on,
        ),
    ];
    rows.push(legend("If Streamer Mode is enabled..."));
    rows.extend([
        option(
            "settings-streamer-personal",
            "Hide my personal information, such as email, connected accounts, and notes",
            current.hide_personal,
            |streamer, on| streamer.hide_personal = on,
        ),
        option(
            "settings-streamer-sounds",
            "Disable all sound effects",
            current.mute_sounds,
            |streamer, on| streamer.mute_sounds = on,
        ),
        option(
            "settings-streamer-notifications",
            "Disable notifications",
            current.mute_notifications,
            |streamer, on| streamer.mute_notifications = on,
        ),
    ]);
    section("Streamer Mode", rows)
}

pub(super) fn message_display(app: &DiscordApp) -> AnyElement {
    let compact = app.text_images().message_display_compact;
    let options = [("Default", false), ("Compact", true)]
        .into_iter()
        .map(|(label, value)| {
            let entity = app.entity.clone();
            (
                label,
                compact == value,
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    entity
                        .update(cx, |app, cx| {
                            app.change_text_images(
                                |display| display.message_display_compact = value,
                                cx,
                            )
                        })
                        .ok();
                },
            )
        })
        .collect();
    row(
        "Chat Message Display",
        Some(if compact {
            "Compact fits more messages on screen, with names and times inline."
        } else {
            "Default shows avatars and names above each group of messages."
        }),
        segmented("settings-message-display", options),
    )
}
