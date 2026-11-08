use super::*;

const SWATCH: f32 = 48.;
const SWATCH_GAP: f32 = 8.;
const SWATCHES_PER_ROW: usize = 12;
const CARD_SWATCHES_PER_ROW: usize = 11;

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

pub(super) fn theme_section(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
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
