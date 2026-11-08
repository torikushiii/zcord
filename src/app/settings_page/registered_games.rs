use std::time::Duration;

use crate::app::menu::PopupButton as _;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use super::{Page, SettingsPage, sections};
use crate::{
    app::{DiscordApp, calls::voice::switch},
    games::{
        programs::{self, Program},
        registry::{self, GameKey, RegisteredGame},
    },
    theme::{self, rpx},
};

const ADD_WIDTH: f32 = 296.;
const OPTION_HEIGHT: f32 = 40.;
const OPTIONS_SHOWN: f32 = 5.;

#[derive(Default)]
pub(super) struct GamesPage {
    adding: Option<Adding>,
    renaming: Option<Renaming>,
    menu: Option<GameKey>,
    hovered: Option<GameKey>,
}

struct Adding {
    programs: Option<Vec<Program>>,
    filter: Entity<InputState>,
    list_open: bool,
    chosen: Option<Program>,
    _events: Subscription,
    _load: Task<()>,
}

struct Renaming {
    key: GameKey,
    input: Entity<InputState>,
    _events: Subscription,
}

impl DiscordApp {
    fn games_page(&mut self) -> Option<&mut GamesPage> {
        self.settings_page
            .as_mut()
            .map(|settings| &mut settings.games)
    }

    fn open_add_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Select"));
        let events = cx.subscribe_in(&filter, window, |app, _, event, _, cx| {
            if let (InputEvent::Change, Some(adding)) = (
                event,
                app.games_page().and_then(|page| page.adding.as_mut()),
            ) {
                adding.list_open = true;
                adding.chosen = None;
                cx.notify();
            }
        });
        let load = cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async { programs::running() })
                .await;
            this.update(cx, |app, cx| {
                if let Some(adding) = app.games_page().and_then(|page| page.adding.as_mut()) {
                    adding.programs = Some(found);
                    cx.notify();
                }
            })
            .ok();
        });
        if let Some(page) = self.games_page() {
            page.adding = Some(Adding {
                programs: None,
                filter,
                list_open: false,
                chosen: None,
                _events: events,
                _load: load,
            });
        }
        cx.notify();
    }

    fn close_add_game(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.games_page() {
            page.adding = None;
        }
        cx.notify();
    }

    fn toggle_program_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.games_page().and_then(|page| page.adding.as_mut()) else {
            return;
        };
        adding.list_open ^= true;
        let filter = adding.filter.clone();
        filter.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn choose_program(&mut self, program: Program, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.games_page().and_then(|page| page.adding.as_mut()) else {
            return;
        };
        let filter = adding.filter.clone();
        let name = program.name.clone();
        filter.update(cx, |input, cx| input.set_value(name, window, cx));
        // Setting the value above reports a change, which clears the choice; set it after.
        if let Some(adding) = self.games_page().and_then(|page| page.adding.as_mut()) {
            adding.chosen = Some(program);
            adding.list_open = false;
        }
        cx.notify();
    }

    fn add_chosen_game(&mut self, cx: &mut Context<Self>) {
        let Some(program) = self
            .games_page()
            .and_then(|page| page.adding.as_ref())
            .and_then(|adding| adding.chosen.clone())
        else {
            return;
        };
        registry::add_program(&program.name, &program.exe);
        self.close_add_game(cx);
    }

    fn set_game_hovered(&mut self, key: GameKey, hovered: bool, cx: &mut Context<Self>) {
        let Some(page) = self.games_page() else {
            return;
        };
        if hovered {
            page.hovered = Some(key);
        } else if page.hovered.as_ref() == Some(&key) {
            page.hovered = None;
        } else {
            return;
        }
        cx.notify();
    }

    fn set_game_menu(&mut self, key: Option<GameKey>, cx: &mut Context<Self>) {
        if let Some(page) = self.games_page() {
            page.menu = key;
            cx.notify();
        }
    }

    fn start_game_rename(
        &mut self,
        game: &RegisteredGame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = game.key();
        let name = game.name.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let events = cx.subscribe_in(&input, window, |app, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                app.finish_game_rename(cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        if let Some(page) = self.games_page() {
            page.renaming = Some(Renaming {
                key,
                input,
                _events: events,
            });
        }
        cx.notify();
    }

    fn finish_game_rename(&mut self, cx: &mut Context<Self>) {
        let Some(renaming) = self.games_page().and_then(|page| page.renaming.take()) else {
            return;
        };
        let name = renaming.input.read(cx).value().to_string();
        registry::rename(&renaming.key, &name);
        cx.notify();
    }

    pub(super) fn watch_registered_games(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut seen = registry::revision();
            let mut ticks = 0u32;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                ticks = ticks.wrapping_add(1);
                let revision = registry::revision();
                let due = revision != seen || ticks.is_multiple_of(60);
                seen = revision;
                let open = this
                    .update(cx, |app, cx| {
                        let showing = app
                            .settings_page
                            .as_ref()
                            .is_some_and(|settings| settings.page == Page::RegisteredGames);
                        if showing && due {
                            cx.notify();
                        }
                        app.settings_page.is_some()
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        })
    }
}

pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    let games = registry::games();
    let playing = |game: &RegisteredGame| match (game.application_id, &app.game.current) {
        (_, None) => false,
        (Some(id), Some(_)) => app.game.application == Some(id),
        (None, Some(name)) => app.game.application.is_none() && *name == game.name,
    };
    let current = games.iter().find(|game| playing(game));
    let mut current_rows = sections::detected_games(app, settings);
    current_rows.push(current_card(app, current, app.game.current.as_deref()));
    current_rows.push(add_line(app, &settings.games, current.is_some(), cx));
    let added = if games.is_empty() {
        vec![sections::note("No games added")]
    } else {
        vec![
            div()
                .flex()
                .flex_col()
                .gap(rpx(24.))
                .pt(rpx(8.))
                .children(games.iter().enumerate().map(|(index, game)| {
                    game_row(app, &settings.games, index, game, playing(game))
                }))
                .into_any_element(),
        ]
    };
    vec![
        sections::section("Current Game", current_rows),
        sections::section("Added Games", added),
    ]
}

fn verified_badge() -> impl IntoElement {
    div()
        .flex_none()
        .child(theme::icon("badge-check", 18., theme::BLURPLE))
}

fn game_name(name: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(rpx(16.))
        .line_height(rpx(22.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(name.into())
}

fn detail_line() -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
}

fn current_card(
    app: &DiscordApp,
    game: Option<&RegisteredGame>,
    playing: Option<&str>,
) -> AnyElement {
    let (name, verified) = match (game, playing) {
        (Some(game), _) => (game.name.clone(), game.verified()),
        (None, Some(name)) => (name.to_owned(), app.game.application.is_some()),
        (None, None) => ("No game detected".to_owned(), false),
    };
    let toggle = game.map(|game| {
        let key = game.key();
        let on = game.detection;
        div()
            .id("settings-current-game-detection")
            .flex_none()
            .cursor_pointer()
            .on_click(move |_, _, _| registry::set_detection(&key, !on))
            .child(switch("settings-current-game-detection", on, None))
    });
    div()
        .flex()
        .items_center()
        .gap(rpx(16.))
        .mt(rpx(8.))
        .p(rpx(16.))
        .rounded(rpx(12.))
        .bg(rgba(theme::control_bg()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .min_w_0()
                        .child(game_name(name))
                        .when(verified, |this| this.child(verified_badge())),
                )
                .when(playing.is_some(), |this| {
                    this.child(detail_line().child("Now playing!"))
                }),
        )
        .children(toggle)
        .into_any_element()
}

fn add_line(app: &DiscordApp, page: &GamesPage, playing: bool, cx: &App) -> AnyElement {
    let entity = app.entity.clone();
    let open = page.adding.is_some();
    div()
        .relative()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .mt(rpx(8.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
        .child(if playing {
            "Not seeing your game?"
        } else {
            "What are you playing?!"
        })
        .child(
            div()
                .id("settings-add-game")
                .cursor_pointer()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::link()))
                .hover(|style| style.underline())
                .on_popup_click(
                    crate::app::menu::shown(app, |app| {
                        app.settings_page
                            .as_ref()
                            .is_some_and(|settings| settings.games.adding.is_some())
                    }),
                    move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| {
                                if open {
                                    app.close_add_game(cx)
                                } else {
                                    app.open_add_game(window, cx)
                                }
                            })
                            .ok();
                    },
                )
                .child("Add it!"),
        )
        .children(
            page.adding
                .as_ref()
                .map(|adding| add_popout(app, adding, cx)),
        )
        .into_any_element()
}

fn add_popout(app: &DiscordApp, adding: &Adding, cx: &App) -> AnyElement {
    let close = app.entity.clone();
    let cancel = app.entity.clone();
    let add = app.entity.clone();
    let toggle = app.entity.clone();
    let chosen = adding.chosen.is_some();
    let select = div()
        .id("settings-add-game-select")
        .relative()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(40.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgb(theme::input_bg()))
        .border_1()
        .border_color(if adding.list_open {
            rgb(theme::BLURPLE)
        } else {
            rgb(theme::input_border())
        })
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            toggle
                .update(cx, |app, cx| app.toggle_program_list(window, cx))
                .ok();
        })
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&adding.filter)
                    .appearance(false)
                    .aria_label("Select")
                    .px_0()
                    .py_0()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .text_color(rgb(theme::text())),
            ),
        )
        .child(theme::icon(
            if adding.list_open {
                "chevron-up"
            } else {
                "chevron-down"
            },
            20.,
            theme::text(),
        ))
        .when(adding.list_open, |this| {
            this.child(program_list(app, adding, cx))
        });
    let buttons = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(rpx(8.))
        .child(
            div()
                .id("settings-add-game-cancel")
                .flex()
                .items_center()
                .h(rpx(38.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .cursor_pointer()
                .text_size(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    cancel.update(cx, |app, cx| app.close_add_game(cx)).ok();
                })
                .child("Cancel"),
        )
        .child(
            div()
                .id("settings-add-game-confirm")
                .flex()
                .items_center()
                .h(rpx(38.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .bg(rgb(theme::BLURPLE))
                .text_size(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white())
                .map(|this| {
                    if chosen {
                        this.cursor_pointer()
                            .hover(|style| style.opacity(0.85))
                            .on_click(move |_, _, cx| {
                                add.update(cx, |app, cx| app.add_chosen_game(cx)).ok();
                            })
                    } else {
                        this.opacity(0.5)
                    }
                })
                .child("Add Game"),
        );
    div()
        .absolute()
        .top(rpx(26.))
        .left_0()
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(Edges::all(px(8.)))
                    .child(
                        div()
                            .id("settings-add-game-popout")
                            .flex()
                            .flex_col()
                            .gap(rpx(16.))
                            .w(rpx(ADD_WIDTH))
                            .p(rpx(8.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::elevated()))
                            .border_1()
                            .border_color(rgba(theme::border_bright()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(move |_, _, cx| {
                                close.update(cx, |app, cx| app.close_add_game(cx)).ok();
                            })
                            .child(select)
                            .child(buttons),
                    ),
            )
            .with_priority(3),
        )
        .into_any_element()
}

fn program_list(app: &DiscordApp, adding: &Adding, cx: &App) -> AnyElement {
    let query = adding.filter.read(cx).value().trim().to_lowercase();
    let filtering = adding.chosen.is_none() && !query.is_empty();
    let body = match &adding.programs {
        None => vec![list_note("Loading\u{2026}")],
        Some(programs) => {
            let options: Vec<AnyElement> = programs
                .iter()
                .filter(|program| !filtering || program.name.to_lowercase().contains(&query))
                .enumerate()
                .map(|(index, program)| {
                    let entity = app.entity.clone();
                    let choice = program.clone();
                    div()
                        .id(("settings-add-game-option", index))
                        .flex()
                        .items_center()
                        .flex_none()
                        .h(rpx(OPTION_HEIGHT))
                        .px(rpx(12.))
                        .rounded(rpx(4.))
                        .cursor_pointer()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .text_color(rgb(theme::text()))
                        .hover(|style| style.bg(rgba(theme::selected())))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let choice = choice.clone();
                            entity
                                .update(cx, |app, cx| app.choose_program(choice, window, cx))
                                .ok();
                        })
                        .child(div().min_w_0().truncate().child(program.name.clone()))
                        .into_any_element()
                })
                .collect();
            if options.is_empty() {
                vec![list_note("No programs found")]
            } else {
                options
            }
        }
    };
    div()
        .absolute()
        .top(rpx(42.))
        .left(rpx(-1.))
        .size_0()
        .child(
            deferred(
                div()
                    .id("settings-add-game-options")
                    .w(rpx(ADD_WIDTH - 16.))
                    .max_h(rpx(OPTION_HEIGHT * OPTIONS_SHOWN + 8.))
                    .overflow_y_scroll()
                    .p(rpx(4.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border_bright()))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(body),
            )
            .with_priority(4),
        )
        .into_any_element()
}

fn list_note(text: &'static str) -> AnyElement {
    div()
        .px(rpx(12.))
        .py(rpx(10.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
        .child(text)
        .into_any_element()
}

fn game_row(
    app: &DiscordApp,
    page: &GamesPage,
    index: usize,
    game: &RegisteredGame,
    playing: bool,
) -> AnyElement {
    let key = game.key();
    let hovered = page.hovered.as_ref() == Some(&key);
    let menu_open = page.menu.as_ref() == Some(&key);
    let renaming = page
        .renaming
        .as_ref()
        .filter(|renaming| renaming.key == key);
    let name = match renaming {
        Some(renaming) => div()
            .w(rpx(240.))
            .ml(rpx(-5.))
            .px(rpx(4.))
            .rounded(rpx(3.))
            .border_1()
            .border_color(rgb(theme::input_border()))
            .child(
                Input::new(&renaming.input)
                    .appearance(false)
                    .px_0()
                    .py_0()
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong())),
            )
            .into_any_element(),
        None if !game.verified() => {
            let entity = app.entity.clone();
            let editable = game.clone();
            div()
                .id(("settings-game-name", index))
                .min_w_0()
                .cursor_text()
                .on_click(move |_, window, cx| {
                    entity
                        .update(cx, |app, cx| app.start_game_rename(&editable, window, cx))
                        .ok();
                })
                .child(game_name(game.name.clone()))
                .into_any_element()
        }
        None => game_name(game.name.clone()).into_any_element(),
    };
    let detail = if hovered {
        detail_line()
            .child(game.exe.to_uppercase())
            .into_any_element()
    } else if playing {
        detail_line().child("Now playing!").into_any_element()
    } else {
        let since = registry_ago(game.last_played_ms);
        detail_line()
            .child("Last played ")
            .child(div().font_weight(FontWeight::SEMIBOLD).child(since))
            .flex()
            .into_any_element()
    };
    let hover_key = key.clone();
    let hover = app.entity.clone();
    let toggle_key = key.clone();
    let on = game.detection;
    div()
        .id(("settings-game", index))
        .flex()
        .items_center()
        .gap(rpx(16.))
        .on_hover(move |over, _, cx| {
            let key = hover_key.clone();
            hover
                .update(cx, |app, cx| app.set_game_hovered(key, *over, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .min_w_0()
                        .child(name)
                        .when(game.verified(), |this| this.child(verified_badge())),
                )
                .child(detail),
        )
        .child(more_button(
            app,
            index,
            key,
            hovered || menu_open,
            menu_open,
        ))
        .child(
            div()
                .id(("settings-game-detection", index))
                .flex_none()
                .cursor_pointer()
                .on_click(move |_, _, _| registry::set_detection(&toggle_key, !on))
                .child(switch("settings-game-detection", on, None)),
        )
        .into_any_element()
}

fn more_button(
    app: &DiscordApp,
    index: usize,
    key: GameKey,
    shown: bool,
    open: bool,
) -> impl IntoElement {
    let entity = app.entity.clone();
    let menu_key = key.clone();
    div()
        .relative()
        .flex_none()
        .child(
            theme::icon_button(
                ("settings-game-more", index),
                "ellipsis",
                16.,
                theme::muted(),
            )
            .size(rpx(32.))
            .rounded(rpx(8.))
            .when(!shown, |this| this.opacity(0.))
            .on_popup_click(
                {
                    let key = key.clone();
                    crate::app::menu::shown(app, move |app| {
                        app.settings_page
                            .as_ref()
                            .is_some_and(|settings| settings.games.menu.as_ref() == Some(&key))
                    })
                },
                move |_, _, cx| {
                    let key = (!open).then(|| menu_key.clone());
                    entity.update(cx, |app, cx| app.set_game_menu(key, cx)).ok();
                },
            ),
        )
        .when(open, |this| this.child(game_menu(app, key)))
}

fn game_menu(app: &DiscordApp, key: GameKey) -> AnyElement {
    let close = app.entity.clone();
    let remove = app.entity.clone();
    div()
        .absolute()
        .top(rpx(36.))
        .right_0()
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(Edges::all(px(8.)))
                    .child(
                        div()
                            .id("settings-game-menu")
                            .flex()
                            .flex_col()
                            .min_w(rpx(160.))
                            .p(rpx(8.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::elevated()))
                            .border_1()
                            .border_color(rgba(theme::border_bright()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(move |_, _, cx| {
                                close.update(cx, |app, cx| app.set_game_menu(None, cx)).ok();
                            })
                            .child(
                                div()
                                    .id("settings-game-remove")
                                    .flex()
                                    .items_center()
                                    .h(rpx(36.))
                                    .px(rpx(10.))
                                    .rounded(rpx(4.))
                                    .cursor_pointer()
                                    .text_size(rpx(14.))
                                    .line_height(rpx(18.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::RED))
                                    .hover(|style| style.bg(rgba(theme::selected())))
                                    .on_click(move |_, _, cx| {
                                        registry::remove(&key);
                                        remove
                                            .update(cx, |app, cx| app.set_game_menu(None, cx))
                                            .ok();
                                    })
                                    .child("Remove"),
                            ),
                    ),
            )
            .with_priority(3),
        )
        .into_any_element()
}

fn registry_ago(last_played_ms: u64) -> String {
    ago(registry::now_ms().saturating_sub(last_played_ms) / 1000)
}

fn ago(seconds: u64) -> String {
    let minutes = (seconds as f64 / 60.).round() as u64;
    let hours = (seconds as f64 / 3600.).round() as u64;
    let days = (seconds as f64 / 86_400.).round() as u64;
    let months = (seconds as f64 / 2_629_746.).round() as u64;
    let years = (seconds as f64 / 31_556_952.).round() as u64;
    match seconds {
        0..45 => "a few seconds ago".into(),
        45..90 => "a minute ago".into(),
        90..2_700 => format!("{minutes} minutes ago"),
        2_700..5_400 => "an hour ago".into(),
        5_400..79_200 => format!("{hours} hours ago"),
        79_200..129_600 => "a day ago".into(),
        129_600..2_246_400 => format!("{days} days ago"),
        2_246_400..3_888_000 => "a month ago".into(),
        3_888_000..27_648_000 => format!("{months} months ago"),
        27_648_000..47_347_200 => "a year ago".into(),
        _ => format!("{years} years ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::ago;

    #[test]
    fn last_played_uses_discord_wording() {
        assert_eq!(ago(10), "a few seconds ago");
        assert_eq!(ago(60), "a minute ago");
        assert_eq!(ago(5 * 3600), "5 hours ago");
        assert_eq!(ago(36 * 3600), "2 days ago");
        assert_eq!(ago(21 * 86_400), "21 days ago");
        assert_eq!(ago(30 * 86_400), "a month ago");
        assert_eq!(ago(61 * 86_400), "2 months ago");
        assert_eq!(ago(400 * 86_400), "a year ago");
    }
}
