use gpui::{App, rgb, rgba};
use gpui_component::{Theme, ThemeMode};

use super::*;

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    colors.background = rgb(BG).into();
    colors.foreground = rgb(TEXT).into();
    colors.border = rgba(BORDER).into();
    colors.input = rgba(BORDER).into();
    colors.caret = rgb(STRONG).into();
    colors.ring = rgb(FOCUS).into();
    colors.selection = rgb(TEXT_SELECTION).into();
    colors.link = rgb(LINK).into();
    colors.link_hover = rgb(LINK).into();
    colors.link_active = rgb(LINK).into();
    colors.accent = rgba(HOVER).into();
    colors.accent_foreground = rgb(STRONG).into();
    colors.primary = rgb(BLURPLE).into();
    colors.primary_hover = rgb(BLURPLE).into();
    colors.primary_active = rgb(BLURPLE_ACTIVE).into();
    colors.primary_foreground = gpui::white();
    colors.secondary = rgba(CONTROL).into();
    colors.secondary_hover = rgba(HOVER).into();
    colors.secondary_active = rgba(ACTIVE).into();
    colors.secondary_foreground = rgb(STRONG).into();
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
    colors.muted = rgba(CONTROL).into();
    colors.muted_foreground = rgb(FAINT).into();
    colors.popover = rgb(ELEVATED).into();
    colors.popover_foreground = rgb(TEXT).into();
    colors.list = rgb(BG).into();
    colors.list_hover = rgba(HOVER).into();
    colors.list_active = rgba(SELECTED).into();
    colors.list_active_border = rgba(BORDER).into();
    colors.list_even = rgb(BG).into();
    colors.list_head = rgb(SURFACE_HIGH).into();
    colors.sidebar = rgb(BG).into();
    colors.sidebar_foreground = rgb(MUTED).into();
    colors.sidebar_border = rgba(BORDER_SUBTLE).into();
    colors.sidebar_accent = rgba(SELECTED).into();
    colors.sidebar_accent_foreground = rgb(STRONG).into();
    colors.sidebar_primary = rgb(BLURPLE).into();
    colors.sidebar_primary_foreground = gpui::white();
    colors.tab = rgb(BG).into();
    colors.tab_bar = rgb(BG).into();
    colors.tab_bar_segmented = rgb(SURFACE_HIGH).into();
    colors.tab_foreground = rgb(MUTED).into();
    colors.tab_active = rgba(SELECTED).into();
    colors.tab_active_foreground = rgb(STRONG).into();
    colors.title_bar = rgb(BG).into();
    colors.title_bar_border = rgba(BORDER_SUBTLE).into();
    colors.status_bar = rgb(BG).into();
    colors.status_bar_border = rgba(BORDER_SUBTLE).into();
    colors.drag_border = rgb(FOCUS).into();
    colors.overlay = rgba(SCRIM).into();
    colors.scrollbar = gpui::transparent_black();
    colors.scrollbar_thumb = rgba(BORDER_STRONG).into();
    colors.scrollbar_thumb_hover = rgb(INTERACTIVE_MUTED).into();
    colors.window_border = rgba(BORDER).into();
    colors.skeleton = rgb(SURFACE_HIGHEST).into();
    theme.tokens = theme.colors.into();

    // Input/selection behavior and Base scrollbars keep their own theme projection.
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
    fn midnight_updates_components_and_base_without_changing_selection(cx: &mut TestAppContext) {
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
        assert_eq!(theme::composite(theme::HOVER, theme::BG), 0x121213);
        assert_eq!(theme::composite(theme::SELECTED, theme::BG), 0x242426);
        assert_ne!(
            theme::composite(theme::SELECTED, theme::BG),
            theme::composite(theme::SELECTED, theme::ELEVATED)
        );
        assert_eq!(theme::status_color(Status::Online), 0x3D9E60);
        assert_eq!(theme::status_color(Status::Idle), 0xFFCB6E);
        assert_eq!(theme::status_color(Status::DoNotDisturb), 0xDC4247);
        assert_eq!(theme::status_color(Status::Offline), 0x9D9EA5);
    }
}
