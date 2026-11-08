//! The message box's right-click menu: suggestions, the send button and paste.

use gpui::{prelude::*, *};
use gpui_component::input::{Paste, TextareaState};

use crate::{
    app::{
        DiscordApp,
        menu::{checkbox, divider, panel, row},
    },
    theme::{self, rpx},
};

const MENU_WIDTH: f32 = 216.;
const FLYOUT_WIDTH: f32 = 200.;
const UNAVAILABLE: &str = "Not available in zcord yet";
#[cfg(target_os = "macos")]
const PASTE_SHORTCUT: &str = "⌘V";
#[cfg(not(target_os = "macos"))]
const PASTE_SHORTCUT: &str = "Ctrl+V";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ComposerTarget {
    Main,
    Pane,
}

pub(in crate::app) struct ComposerMenu {
    target: ComposerTarget,
    position: Point<Pixels>,
    suggestions_open: bool,
    /// The flyout lies outside the menu; a click there must not count as outside.
    pointer_in_flyout: bool,
}

impl DiscordApp {
    pub(in crate::app) fn open_composer_menu(
        &mut self,
        target: ComposerTarget,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.composer_menu = Some(ComposerMenu {
            target,
            position,
            suggestions_open: false,
            pointer_in_flyout: false,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_composer_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.composer_menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn show_suggestions_flyout(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some(menu) = self
            .composer_menu
            .as_mut()
            .filter(|menu| menu.suggestions_open != open)
        else {
            return;
        };
        menu.suggestions_open = open;
        menu.pointer_in_flyout = false;
        cx.notify();
    }

    fn toggle_send_button(&mut self, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| {
            settings.composer_send_button = !settings.composer_send_button;
        });
        cx.notify();
    }

    fn composer_for(&self, target: ComposerTarget) -> Option<Entity<TextareaState>> {
        match target {
            ComposerTarget::Main => Some(self.compose.input.clone()),
            ComposerTarget::Pane => self.pane_composer(),
        }
    }

    /// Pastes as Ctrl+V would, files and images included, into the box the menu belongs to.
    fn paste_into(&mut self, target: ComposerTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.close_composer_menu(cx);
        let Some(input) = self.composer_for(target) else {
            return;
        };
        input.update(cx, |input, cx| input.focus(window, cx));
        window.dispatch_action(Box::new(Paste), cx);
    }
}

fn closes_flyout_row(app: &DiscordApp, key: &'static str, enabled: bool) -> Stateful<Div> {
    let entity = app.entity.clone();
    row(key, enabled).on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_suggestions_flyout(false, cx))
                .ok();
        }
    })
}

/// Shown like Discord's but dimmed, with a tooltip saying zcord doesn't do it yet. The tooltip
/// sits beside the dimmed row rather than in it, so it keeps full opacity.
fn unavailable(row: Stateful<Div>, key: &'static str) -> Stateful<Div> {
    div()
        .id(key)
        .relative()
        .child(row)
        .child(theme::hover_tooltip(key, UNAVAILABLE))
}

fn label(text: &'static str) -> Div {
    div().flex_1().min_w_0().truncate().child(text)
}

fn suggestions_flyout(app: &DiscordApp) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id("composer-suggestions-flyout")
        .absolute()
        .top(rpx(-9.))
        .left(relative(1.))
        .pl(rpx(8.))
        .child(
            panel("composer-suggestions", FLYOUT_WIDTH)
                .on_hover(move |hovered, _, cx| {
                    entity
                        .update(cx, |app, _| {
                            if let Some(menu) = app.composer_menu.as_mut() {
                                menu.pointer_in_flyout = *hovered;
                            }
                        })
                        .ok();
                })
                .child(unavailable(
                    row("composer-slash-commands", false)
                        .child(label("Slash Commands"))
                        .child(checkbox(false)),
                    "composer-slash-commands-tip",
                )),
        )
}

pub(in crate::app) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let menu = app.composer_menu.as_ref()?;
    let target = menu.target;
    let send_button = crate::settings::get(cx).composer_send_button;
    let (close, hover, toggle, paste) = (
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
    );
    let body = panel("composer-menu", MENU_WIDTH)
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| {
                    if !app
                        .composer_menu
                        .as_ref()
                        .is_some_and(|menu| menu.pointer_in_flyout)
                    {
                        app.close_composer_menu(cx);
                    }
                })
                .ok();
        })
        .child(
            row("composer-suggestions-row", true)
                .when(menu.suggestions_open, |this| {
                    this.bg(rgba(theme::selected()))
                })
                .on_hover(move |hovered, _, cx| {
                    if *hovered {
                        hover
                            .update(cx, |app, cx| app.show_suggestions_flyout(true, cx))
                            .ok();
                    }
                })
                .child(label("Suggestions"))
                .child(theme::icon("chevron-right", 20., theme::muted()))
                .when(menu.suggestions_open, |this| {
                    this.child(suggestions_flyout(app))
                }),
        )
        .child(
            closes_flyout_row(app, "composer-send-button", true)
                .on_click(move |_, _, cx| {
                    toggle.update(cx, |app, cx| app.toggle_send_button(cx)).ok();
                })
                .child(label("Send Message Button"))
                .child(checkbox(send_button)),
        )
        .child(divider())
        .child(
            closes_flyout_row(app, "composer-paste", true)
                .on_click(move |_, window, cx| {
                    paste
                        .update(cx, |app, cx| app.paste_into(target, window, cx))
                        .ok();
                })
                .child(label("Paste"))
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::muted()))
                        .child(PASTE_SHORTCUT),
                ),
        );
    Some(
        deferred(
            anchored()
                .position(menu.position)
                .snap_to_window_with_margin(px(8.))
                .child(body),
        )
        .with_priority(2)
        .into_any_element(),
    )
}
