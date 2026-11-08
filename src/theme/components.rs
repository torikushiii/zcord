use gpui::{App, rgb, rgba};
use gpui_component::{Theme, ThemeMode};

use super::*;

pub fn init(cx: &mut App) {
    let mode = if is_light() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);
    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    colors.background = rgb(bg()).into();
    colors.foreground = rgb(text()).into();
    colors.border = rgba(border()).into();
    colors.input = rgba(border()).into();
    colors.caret = rgb(strong()).into();
    colors.ring = rgb(focus()).into();
    colors.selection = rgb(text_selection()).into();
    colors.link = rgb(link()).into();
    colors.link_hover = rgb(link()).into();
    colors.link_active = rgb(link()).into();
    colors.accent = rgba(hover()).into();
    colors.accent_foreground = rgb(strong()).into();
    colors.primary = rgb(BLURPLE).into();
    colors.primary_hover = rgb(BLURPLE).into();
    colors.primary_active = rgb(BLURPLE_ACTIVE).into();
    colors.primary_foreground = gpui::white();
    colors.secondary = rgba(control()).into();
    colors.secondary_hover = rgba(hover()).into();
    colors.secondary_active = rgba(active()).into();
    colors.secondary_foreground = rgb(strong()).into();
    colors.button = colors.secondary;
    colors.button_hover = colors.secondary_hover;
    colors.button_active = colors.secondary_active;
    colors.button_foreground = colors.secondary_foreground;
    colors.button_primary = colors.primary;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.button_secondary = colors.secondary;
    colors.button_secondary_hover = colors.secondary_hover;
    colors.button_secondary_active = colors.secondary_active;
    colors.button_secondary_foreground = colors.secondary_foreground;
    colors.muted = rgba(control()).into();
    colors.muted_foreground = rgb(faint()).into();
    colors.popover = rgb(elevated()).into();
    colors.popover_foreground = rgb(text()).into();
    colors.list = rgb(bg()).into();
    colors.list_hover = rgba(hover()).into();
    colors.list_active = rgba(selected()).into();
    colors.list_active_border = rgba(border()).into();
    colors.list_even = rgb(bg()).into();
    colors.list_head = rgb(surface_high()).into();
    colors.sidebar = rgb(bg()).into();
    colors.sidebar_foreground = rgb(muted()).into();
    colors.sidebar_border = rgba(border_subtle()).into();
    colors.sidebar_accent = rgba(selected()).into();
    colors.sidebar_accent_foreground = rgb(strong()).into();
    colors.sidebar_primary = rgb(BLURPLE).into();
    colors.sidebar_primary_foreground = gpui::white();
    colors.tab = rgb(bg()).into();
    colors.tab_bar = rgb(bg()).into();
    colors.tab_bar_segmented = rgb(surface_high()).into();
    colors.tab_foreground = rgb(muted()).into();
    colors.tab_active = rgba(selected()).into();
    colors.tab_active_foreground = rgb(strong()).into();
    colors.title_bar = rgb(bg()).into();
    colors.title_bar_border = rgba(border_subtle()).into();
    colors.status_bar = rgb(bg()).into();
    colors.status_bar_border = rgba(border_subtle()).into();
    colors.drag_border = rgb(focus()).into();
    colors.overlay = rgba(SCRIM).into();
    colors.scrollbar = gpui::transparent_black();
    colors.scrollbar_thumb = rgba(border_strong()).into();
    colors.scrollbar_thumb_hover = rgb(interactive_muted()).into();
    colors.window_border = rgba(border()).into();
    colors.skeleton = rgb(surface_highest()).into();
    theme.tokens = theme.colors.into();

    let tokens = theme.semantic_tokens();
    let scrollbar = colors_for_scrollbar(theme);
    let border = theme.border;
    let focus = theme.drag_border;
    let base = gpui_base::Theme::global_mut(cx);
    base.tokens = tokens;
    base.scrollbar.styles = scrollbar;
    base.resizable.handle = border;
    base.resizable.active_handle = focus;
}

fn colors_for_scrollbar(theme: &Theme) -> gpui_base::ScrollbarStyles {
    let track = theme.scrollbar;
    let thumb = theme.scrollbar_thumb;
    let hover = theme.scrollbar_thumb_hover;
    let border = theme.border;
    let radius = theme.radius;
    gpui_base::ScrollbarStyles::default()
        .track(move |style| style.bg(track))
        .track_hover(move |style| style.bg(track))
        .track_active(move |style| style.bg(track).border_color(border))
        .thumb(move |style| style.bg(thumb).radius(radius))
        .thumb_hover(move |style| style.bg(hover).radius(radius))
        .thumb_active(move |style| style.bg(hover).radius(radius))
}

#[cfg(test)]
mod tests {
    use crate::{store::Status, theme};
    use gpui::{Hsla, TestAppContext, rgb, rgba};
    use gpui_component::Theme;

    #[gpui::test]
    fn dark_theme_updates_components_and_base_without_changing_selection(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::init(cx);
            let ui = Theme::global(cx);
            assert!(ui.is_dark());
            assert_eq!(ui.background, Hsla::from(rgb(0x000000)));
            assert_eq!(ui.foreground, Hsla::from(rgb(0xD4D5D8)));
            assert_eq!(ui.popover, Hsla::from(rgb(0x121214)));
            assert_eq!(ui.muted_foreground, Hsla::from(rgb(0x81828A)));
            assert_eq!(ui.border, Hsla::from(rgba(0x96969F3D)));
            assert_eq!(ui.accent, Hsla::from(rgba(0x94949C1F)));
            assert_eq!(ui.selection, Hsla::from(rgb(0x0438A2)));
            assert_eq!(ui.link, Hsla::from(rgb(0x2781E7)));
            assert_eq!(ui.tokens.background.color, ui.background);
            assert_eq!(ui.tokens.list_hover.color, ui.list_hover);
            assert_eq!(ui.tokens.selection.color, ui.selection);
            let base = gpui_base::Theme::global(cx);
            assert_eq!(base.tokens.colors, ui.semantic_tokens().colors);
            assert_eq!(base.resizable.handle, ui.border);
            assert_eq!(base.resizable.active_handle, ui.drag_border);
        });
    }

    #[test]
    fn alpha_layers_follow_the_surface_but_status_colors_remain_opaque() {
        assert_eq!(theme::composite(theme::hover(), theme::bg()), 0x121213);
        assert_eq!(theme::composite(theme::selected(), theme::bg()), 0x242426);
        assert_ne!(
            theme::composite(theme::selected(), theme::bg()),
            theme::composite(theme::selected(), theme::elevated())
        );
        assert_eq!(theme::status_color(Status::Online), 0x3D9E60);
        assert_eq!(theme::status_color(Status::Idle), 0xFFCB6E);
        assert_eq!(theme::status_color(Status::DoNotDisturb), 0xDC4247);
        assert_eq!(theme::status_color(Status::Offline), 0x9D9EA5);
    }
}
