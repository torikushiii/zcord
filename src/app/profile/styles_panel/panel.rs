use super::editing_panel::editing_panel;
use super::*;

const SAVE_BAR_WIDTH: f32 = 500.;
const SCOPE_MENU_WIDTH: f32 = 262.;
const SCOPE_MENU_HEIGHT: f32 = 208.;

pub(super) fn panel_header(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> AnyElement {
    let label: SharedString = match editor.scope {
        StyleScope::Main => "Main Profile".into(),
        StyleScope::Guild(guild_id) => app
            .store
            .guild_by_id(guild_id)
            .map_or_else(|| "Server".into(), |guild| guild.name.clone().into()),
    };
    let open = editor.ui.scope_menu;
    div()
        .flex()
        .flex_row_reverse()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(49.))
        .px(rpx(8.))
        .py(rpx(4.))
        // Discord rules the scope picker off from the tiles below.
        .border_b_1()
        .border_color(rgba(theme::border_subtle()))
        .child(
            theme::icon_button_with_hover(
                "styles-hide",
                "chevrons-right",
                16.,
                theme::muted(),
                rgba(theme::hover()).into(),
            )
            .size(rpx(32.))
            .rounded(rpx(8.))
            .on_click(on_app(app, |app, cx| app.toggle_styles_panel_animated(cx))),
        )
        .child(
            div().flex_1().min_w_0().py(rpx(4.)).child(
                div()
                    .id("styles-scope")
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .max_w_full()
                    .h(rpx(32.))
                    .pl(rpx(8.))
                    .pr(rpx(4.))
                    .rounded(rpx(8.))
                    .cursor_pointer()
                    .when(open, |this| this.bg(rgba(theme::hover())))
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(on_app(app, |app, cx| {
                        app.update_styles_ui(|ui| ui.scope_menu = !ui.scope_menu, cx)
                    }))
                    .when(editor.scope != StyleScope::Main, |this| {
                        this.child(scope_icon(app, editor.scope, 20., cx))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(label),
                    )
                    .child(theme::icon(
                        if open { "chevron-up" } else { "chevron-down" },
                        16.,
                        theme::muted(),
                    )),
            ),
        )
        .into_any_element()
}

fn scope_icon(app: &DiscordApp, scope: StyleScope, size: f32, cx: &App) -> AnyElement {
    let guild = match scope {
        StyleScope::Main => return theme::logo(size).into_any_element(),
        StyleScope::Guild(guild_id) => app.store.guild_by_id(guild_id),
    };
    let Some(guild) = guild else {
        return div().size(rpx(size)).into_any_element();
    };
    if let Some(path) = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx))
    {
        return img(path)
            .size(rpx(size))
            .rounded(rpx(4.))
            .into_any_element();
    }
    div()
        .size(rpx(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(4.))
        .bg(rgb(guild.icon_color))
        .text_size(rpx(size * 0.4))
        .text_color(gpui::black())
        .child(guild.initials.clone())
        .into_any_element()
}

/// The server list under the scope button, drawn over the panel.
pub(crate) fn scope_menu(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> Option<AnyElement> {
    if !editor.ui.scope_menu {
        return None;
    }
    let row = |id: ElementId, scope: StyleScope, label: SharedString, icon: AnyElement| {
        let selected = editor.scope == scope;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(on_app(app, move |app, cx| {
                app.update_styles_ui(|ui| ui.scope_menu = false, cx);
                app.set_style_scope(scope, cx);
            }))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(label),
            )
            .when(selected, |this| {
                this.child(theme::icon("check", 16., theme::strong()))
            })
    };
    let rows = std::iter::once(
        row(
            "styles-scope-main".into(),
            StyleScope::Main,
            "Main Profile".into(),
            scope_icon(app, StyleScope::Main, 20., cx),
        )
        .into_any_element(),
    )
    .chain(app.store.guilds.iter().map(|guild| {
        row(
            ("styles-scope-guild", guild.id).into(),
            StyleScope::Guild(guild.id),
            guild.name.clone().into(),
            scope_icon(app, StyleScope::Guild(guild.id), 20., cx),
        )
        .into_any_element()
    }));
    Some(
        div()
            .id("styles-scope-menu")
            .without_text_selection()
            .absolute()
            .top(rpx(48.))
            .left(rpx(8.))
            .w(rpx(SCOPE_MENU_WIDTH))
            .max_h(rpx(SCOPE_MENU_HEIGHT))
            .overflow_y_scroll()
            .p(rpx(4.))
            .rounded(rpx(8.))
            .border_1()
            .border_color(rgba(theme::border()))
            .bg(rgb(theme::surface_highest()))
            .shadow(vec![
                BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
            ])
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .children(rows)
            .into_any_element(),
    )
}

const PANEL_SLIDE: Duration = Duration::from_millis(250);

/// The panel, sliding out from under the card or back behind it. It stays mounted at zero
/// width once hidden, so hiding can animate too.
pub(crate) fn sliding_panel(
    app: &DiscordApp,
    editor: &StylesEditor,
    height: f32,
    cx: &App,
) -> Option<AnyElement> {
    let toggles = editor.ui.panel_toggles;
    if !editor.expanded && toggles == 0 {
        return None;
    }
    let expanded = editor.expanded;
    let frame = div()
        .flex_none()
        .h(rpx(height))
        .overflow_hidden()
        .child(editing_panel(app, editor, cx));
    if toggles == 0 {
        return Some(frame.w(rpx(PANEL_WIDTH)).into_any_element());
    }
    Some(
        frame
            .with_animation(
                ("styles-panel-slide", toggles),
                Animation::new(PANEL_SLIDE).with_easing(ease_out_quint()),
                move |frame, delta| {
                    let open = if expanded { delta } else { 1. - delta };
                    frame.w(rpx(PANEL_WIDTH * open)).opacity(open)
                },
            )
            .into_any_element(),
    )
}

/// The tab hanging off the dialog's left edge while the panel is hidden.
pub(crate) fn show_styles_tab(
    app: &DiscordApp,
    editor: &StylesEditor,
    background: Background,
) -> AnyElement {
    let toggles = editor.ui.panel_toggles;
    div()
        .id("styles-show")
        .absolute()
        .top(rpx(40.))
        .left(rpx(-18.))
        .w(rpx(32.))
        .h(rpx(36.))
        .flex()
        .items_center()
        .justify_center()
        .pl(rpx(2.))
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(background)
        .cursor_pointer()
        .hover(|style| style.opacity(0.85))
        .on_click(on_app(app, |app, cx| app.toggle_styles_panel_animated(cx)))
        .child(theme::icon("paintbrush", 16., theme::bright()))
        .child(theme::hover_tooltip("styles-show-tip", "Show Styles"))
        .with_animation(
            ("styles-show-tab", toggles),
            Animation::new(PANEL_SLIDE).with_easing(ease_out_quint()),
            |tab, delta| tab.opacity(delta),
        )
        .into_any_element()
}

/// "Don't forget to save your changes!" under the dialog while the styles or widgets have
/// unsaved edits; it saves and resets both.
pub(crate) fn save_bar(app: &DiscordApp, editor: &StylesEditor) -> Option<AnyElement> {
    if editor.draft == StyleDraft::default()
        && editor.ui.widgets.draft().is_none()
        && !super::identity_editor::edited(app, editor)
    {
        return None;
    }
    let button = |id: &'static str, label: &'static str, primary: bool| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .border_1()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|this| {
                if primary {
                    this.bg(rgb(theme::BLURPLE))
                        .border_color(rgba(0xFFFFFF14))
                        .text_color(gpui::white())
                        .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                } else {
                    this.bg(rgba(theme::control_bg()))
                        .border_color(rgba(theme::border_soft()))
                        .text_color(rgb(theme::strong()))
                        .hover(|style| style.bg(rgba(theme::control_selected())))
                }
            })
            .child(label)
    };
    let saving = editor.saving || editor.ui.widgets.saving;
    Some(
        div()
            .id("styles-save-bar")
            .without_text_selection()
            .w(rpx(SAVE_BAR_WIDTH))
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(32.))
            .py(rpx(8.))
            .pr(rpx(8.))
            .pl(rpx(12.))
            .rounded(rpx(12.))
            .border_1()
            .border_color(rgba(theme::border_subtle()))
            .bg(rgb(theme::surface_highest()))
            .shadow(vec![
                BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
            ])
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::strong()))
                    .child("Don't forget to save your changes!"),
            )
            .child(
                div()
                    .flex()
                    .gap(rpx(8.))
                    .child(
                        button("styles-reset", "Reset", false)
                            .on_click(on_app(app, |app, cx| app.reset_style_draft(cx))),
                    )
                    .child(
                        button("styles-save", if saving { "Saving…" } else { "Save" }, true)
                            .when(saving, |this| this.opacity(0.5))
                            .on_click(on_app(app, move |app, cx| {
                                if !saving {
                                    app.save_style_draft(cx);
                                    app.save_widget_draft(cx);
                                }
                            })),
                    ),
            )
            .into_any_element(),
    )
}
