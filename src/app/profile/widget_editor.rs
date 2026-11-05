//! Editing your own Board while the styles editor is open, as Discord's "Your Widgets" view:
//! drag widgets to reorder them, remove them, and edit each game's comment and tags. Edits stay
//! a draft until the save bar sends the whole list with `PUT /users/@me/widgets`.

use gpui_component::input::{InputEvent, Textarea, TextareaState};

use super::styles::StylesEditor;
use super::styles_panel::on_app;
use super::*;
use crate::discord::model::{ProfileWidget, WidgetGame};

const COMMENT_LIMIT: usize = 200;
const SHOWN_TAGS: usize = 4;
/// Where a widget's handle sits inside its row, from the gutter's left and the card's top.
const HANDLE_INSET: [f32; 2] = [8., 18.];
/// The gutter left of each widget that its drag handle sits in; the tab's scroll area
/// reaches into it while editing so the handles aren't clipped.
pub(super) const HANDLE_GUTTER: f32 = 32.;

const TAG_MENU_SIZE: [f32; 2] = [171., 398.];
/// Discord's tag picker: one skill level, then any number of the rest.
const SKILL_TAGS: [&str; 4] = ["casual", "intermediate", "expert", "better_than_you"];
const RATING_TAGS: [&str; 5] = [
    "obsessed",
    "love_it",
    "kind_of_love_it",
    "kind_of_hate_it",
    "rage_quitting",
];
const LOOKING_TAGS: [&str; 5] = [
    "looking_for_group",
    "open_to_play",
    "looking_for_tips",
    "open_to_teach",
    "looking_to_discuss",
];

#[derive(Default)]
pub(crate) struct WidgetsUi {
    /// Unsaved edits to the whole list; the save bar saves or resets them with the styles.
    draft: Option<Vec<ProfileWidget>>,
    /// Bumped per edit, so a save can tell whether the draft changed while it ran.
    revision: u64,
    pub(crate) saving: bool,
    /// A style save finished while widgets were saving; its profile refresh waits for them.
    pub(crate) refresh_after_save: bool,
    pub(crate) error: Option<SharedString>,
    pub(crate) comment: Option<CommentEdit>,
    /// The game whose tag picker is open, and the "+ Tags" pill it hangs from.
    pub(crate) tag_menu: Option<(u64, u64, Option<Bounds<Pixels>>)>,
    /// The widget whose manage menu is open, beside its handle.
    pub(crate) manage_menu: Option<(u64, Option<Bounds<Pixels>>)>,
    pub(crate) expanded_tags: HashSet<(u64, u64)>,
    /// The widget last picked up; it stays faded in place while gpui still has a drag.
    dragging: Option<u64>,
}

pub(crate) struct CommentEdit {
    widget_id: u64,
    game_id: u64,
    input: Entity<TextareaState>,
    _events: Subscription,
}

fn same_widgets(a: &[ProfileWidget], b: &[ProfileWidget]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.body() == b.body())
}

impl WidgetsUi {
    pub(crate) fn draft(&self) -> Option<&Vec<ProfileWidget>> {
        self.draft.as_ref()
    }
}

/// The id a widget drag carries; its own type so no other drag lands here.
#[derive(Clone, Copy)]
struct WidgetDrag(u64);

#[derive(Clone)]
struct DraggedWidget {
    id: u64,
    app: WeakEntity<DiscordApp>,
    width: Option<Pixels>,
}

impl Render for DraggedWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let card = self.app.upgrade().and_then(|app| {
            let app = app.read(cx);
            let card = app.profile.as_ref()?;
            let widget = card
                .profile
                .as_ref()?
                .widgets
                .iter()
                .find(|widget| widget.id == self.id)?;
            super::board::widget_element(app, card, widget, true, cx)
        });
        // The pointer grabbed the handle, which sits this far left of and below the card's
        // corner.
        div()
            .ml(rpx(HANDLE_GUTTER - HANDLE_INSET[0]))
            .mt(rpx(-HANDLE_INSET[1]))
            .when_some(self.width, |this, width| this.w(width))
            .opacity(0.8)
            .rounded(rpx(16.))
            .shadow(vec![
                BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.3)).blur_radius(px(24.)),
            ])
            .children(card)
    }
}

impl DiscordApp {
    fn update_widgets_ui(&mut self, update: impl FnOnce(&mut WidgetsUi), cx: &mut Context<Self>) {
        self.update_styles_ui(|ui| update(&mut ui.widgets), cx);
    }

    fn edit_widgets(
        &mut self,
        change: impl FnOnce(&mut Vec<ProfileWidget>),
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.profile_styles.as_mut() else {
            return;
        };
        let Some(saved) = editor
            .saved_profile
            .as_ref()
            .map(|profile| &profile.widgets)
        else {
            return;
        };
        let ui = &mut editor.ui.widgets;
        let mut widgets = ui.draft.clone().unwrap_or_else(|| saved.clone());
        change(&mut widgets);
        ui.draft = (!same_widgets(&widgets, saved)).then_some(widgets);
        ui.revision += 1;
        ui.error = None;
        self.rebuild_style_preview();
        cx.notify();
    }

    pub(crate) fn reset_widget_draft(&mut self) {
        if let Some(editor) = self.profile_styles.as_mut() {
            let ui = &mut editor.ui.widgets;
            ui.draft = None;
            ui.comment = None;
            ui.error = None;
            ui.revision += 1;
        }
        self.rebuild_style_preview();
    }

    pub(crate) fn save_widget_draft(&mut self, cx: &mut Context<Self>) {
        self.finish_widget_comment(cx);
        let user_id = self.store.current_user_id;
        let session = self.session.clone().filter(|_| self.live);
        let Some(editor) = self.profile_styles.as_mut() else {
            return;
        };
        let ui = &mut editor.ui.widgets;
        let Some(widgets) = ui.draft.clone().filter(|_| !ui.saving) else {
            return;
        };
        let Some(session) = session else {
            ui.error = Some("Connect to Discord to save your widgets.".into());
            cx.notify();
            return;
        };
        ui.saving = true;
        ui.error = None;
        let revision = ui.revision;
        let identity = editor.identity();
        let request = Tokio::spawn(
            cx,
            async move { session.save_widgets(user_id, &widgets).await },
        );
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity(), &identity))
                else {
                    return;
                };
                let ui = &mut editor.ui.widgets;
                ui.saving = false;
                let refresh = std::mem::take(&mut ui.refresh_after_save);
                match result {
                    Ok(Ok(saved)) => {
                        if let Some(profile) = editor.saved_profile.as_mut() {
                            let shown = ui.draft.take();
                            let profile = Arc::make_mut(profile);
                            // Whatever the user moved to while saving stays as the next draft,
                            // even when it matches the list from before the save.
                            if ui.revision != revision {
                                let shown = shown.unwrap_or_else(|| profile.widgets.clone());
                                ui.draft = (!same_widgets(&shown, &saved)).then_some(shown);
                            }
                            profile.widgets = saved;
                        }
                    }
                    Ok(Err(error)) => {
                        log::warn!("Saving widgets failed: {error}");
                        ui.error = Some("Could not save your widgets.".into());
                    }
                    Err(error) => {
                        log::warn!("Widget save task failed: {error}");
                        ui.error = Some("Could not save your widgets.".into());
                    }
                }
                app.rebuild_style_preview();
                if refresh {
                    app.request_profile(ProfileKind::Modal, true, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Moves widget `id` to sit before `before`, or last.
    fn move_widget(&mut self, id: u64, before: Option<u64>, cx: &mut Context<Self>) {
        if before == Some(id) {
            return;
        }
        self.edit_widgets(
            |widgets| {
                let Some(from) = widgets.iter().position(|widget| widget.id == id) else {
                    return;
                };
                let widget = widgets.remove(from);
                let to = before
                    .and_then(|before| widgets.iter().position(|widget| widget.id == before))
                    .unwrap_or(widgets.len());
                widgets.insert(to, widget);
            },
            cx,
        );
    }

    fn shift_widget(&mut self, id: u64, up: bool, cx: &mut Context<Self>) {
        self.edit_widgets(
            |widgets| {
                let Some(from) = widgets.iter().position(|widget| widget.id == id) else {
                    return;
                };
                let to = if up {
                    from.checked_sub(1)
                } else {
                    Some(from + 1).filter(|to| *to < widgets.len())
                };
                if let Some(to) = to {
                    widgets.swap(from, to);
                }
            },
            cx,
        );
    }

    fn remove_widget(&mut self, id: u64, cx: &mut Context<Self>) {
        self.edit_widgets(|widgets| widgets.retain(|widget| widget.id != id), cx);
    }

    fn edit_widget_game(
        &mut self,
        widget_id: u64,
        game_id: u64,
        change: impl FnOnce(&mut WidgetGame),
        cx: &mut Context<Self>,
    ) {
        self.edit_widgets(
            |widgets| {
                if let Some(game) = widgets
                    .iter_mut()
                    .find(|widget| widget.id == widget_id)
                    .and_then(|widget| widget.data.games_mut())
                    .and_then(|games| games.iter_mut().find(|game| game.game_id == game_id))
                {
                    change(game);
                }
            },
            cx,
        );
    }

    fn remove_widget_game(&mut self, widget_id: u64, game_id: u64, cx: &mut Context<Self>) {
        self.edit_widgets(
            |widgets| {
                if let Some(games) = widgets
                    .iter_mut()
                    .find(|widget| widget.id == widget_id)
                    .and_then(|widget| widget.data.games_mut())
                {
                    games.retain(|game| game.game_id != game_id);
                }
            },
            cx,
        );
    }

    /// Toggles `tag`; a skill level replaces any other, and `None` clears it.
    fn toggle_widget_tag(
        &mut self,
        widget_id: u64,
        game_id: u64,
        tag: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.edit_widget_game(
            widget_id,
            game_id,
            |game| match tag {
                Some(tag) if game.tags.iter().any(|current| current == tag) => {
                    game.tags.retain(|current| current != tag);
                }
                Some(tag) => {
                    if SKILL_TAGS.contains(&tag) {
                        game.tags
                            .retain(|current| !SKILL_TAGS.contains(&current.as_str()));
                    }
                    game.tags.push(tag.to_owned());
                }
                None => game
                    .tags
                    .retain(|current| !SKILL_TAGS.contains(&current.as_str())),
            },
            cx,
        );
    }

    fn start_widget_comment(
        &mut self,
        widget_id: u64,
        game_id: u64,
        comment: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_widget_comment(cx);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .placeholder("Let everyone know why this is your favorite")
                .default_value(comment)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let events = cx.subscribe_in(&input, window, |app, _, event, _, cx| match event {
            InputEvent::PressEnter { shift: false, .. } | InputEvent::Blur => {
                app.finish_widget_comment(cx);
            }
            _ => {}
        });
        self.update_widgets_ui(
            |ui| {
                ui.comment = Some(CommentEdit {
                    widget_id,
                    game_id,
                    input,
                    _events: events,
                })
            },
            cx,
        );
    }

    fn finish_widget_comment(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self
            .profile_styles
            .as_mut()
            .and_then(|editor| editor.ui.widgets.comment.take())
        else {
            return;
        };
        let text = edit.input.read(cx).value().trim().to_owned();
        let comment: String = text.chars().take(COMMENT_LIMIT).collect();
        let comment = (!comment.is_empty()).then_some(comment);
        self.edit_widget_game(
            edit.widget_id,
            edit.game_id,
            |game| game.comment = comment,
            cx,
        );
        cx.notify();
    }
}

/// The Board tab while you style your own profile.
pub(super) fn panel(
    app: &DiscordApp,
    card: &ProfileCard,
    editor: &StylesEditor,
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let ui = &editor.ui.widgets;
    let widgets = card.profile.iter().flat_map(|profile| &profile.widgets);
    let header = div()
        .flex()
        .items_center()
        .gap(rpx(16.))
        .h(rpx(32.))
        .mb(rpx(4.))
        .child(
            div()
                .flex_1()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child("Your Widgets"),
        );
    let rows: Vec<AnyElement> = widgets
        .filter_map(|widget| {
            let element = super::board::widget_element(app, card, widget, true, cx)?;
            Some(widget_row(app, widget, element, ui, palette, cx))
        })
        .collect();
    let empty = rows.is_empty();
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .child(div().pl(rpx(HANDLE_GUTTER)).child(header))
        .children(ui.error.clone().map(|error| {
            div()
                .pl(rpx(HANDLE_GUTTER))
                .text_size(rpx(14.))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .children(rows)
        .when(empty, |this| {
            this.child(
                div()
                    .pl(rpx(HANDLE_GUTTER))
                    .text_size(rpx(14.))
                    .text_color(rgb(palette.muted))
                    .child("You have no widgets."),
            )
        })
        .child(
            div()
                .id("widget-drop-end")
                .group("widget-drop-end")
                .relative()
                .h(rpx(24.))
                .child(drop_line("widget-drop-end".into(), -7.))
                .on_drop(drop_handler(app, None)),
        )
        .into_any_element()
}

fn drop_handler(
    app: &DiscordApp,
    before: Option<u64>,
) -> impl Fn(&WidgetDrag, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |&WidgetDrag(id), _, cx| {
        entity
            .update(cx, |app, cx| {
                app.update_widgets_ui(|ui| ui.dragging = None, cx);
                app.move_widget(id, before, cx);
            })
            .ok();
    }
}

fn widget_row(
    app: &DiscordApp,
    widget: &ProfileWidget,
    element: AnyElement,
    ui: &WidgetsUi,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let id = widget.id;
    let group: SharedString = format!("widget-row-{id}").into();
    let bounds = Rc::new(Cell::new(None));
    let card_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let menu_open = ui.manage_menu.is_some_and(|(open, _)| open == id);
    let picked_up = ui.dragging == Some(id) && cx.has_active_drag();
    let entity = app.entity.clone();
    let width = card_bounds.clone();
    let handle = div()
        .id(("widget-handle", id))
        .absolute()
        .left(rpx(HANDLE_INSET[0]))
        .top(rpx(HANDLE_INSET[1]))
        .size(rpx(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(4.))
        .cursor_grab()
        .opacity(if menu_open || picked_up { 1. } else { 0. })
        .group_hover(group.clone(), |style| style.opacity(1.))
        .hover(move |style| style.bg(palette.hover))
        .child(super::styles_panel::measure_into(bounds.clone()))
        .child(theme::icon("drag-handle", 18., palette.muted))
        .on_drag(WidgetDrag(id), move |&WidgetDrag(id), _, _, cx| {
            if let Some(app) = entity.upgrade() {
                app.update(cx, |app, cx| {
                    app.update_widgets_ui(
                        |ui| {
                            ui.dragging = Some(id);
                            ui.manage_menu = None;
                        },
                        cx,
                    )
                });
            }
            let dragged = DraggedWidget {
                id,
                app: entity.clone(),
                width: width.get().map(|bounds| bounds.size.width),
            };
            cx.new(|_| dragged)
        })
        .on_click(on_app(app, move |app, cx| {
            let open = app.profile_styles.as_ref().is_some_and(|editor| {
                editor
                    .ui
                    .widgets
                    .manage_menu
                    .is_some_and(|(open, _)| open == id)
            });
            let anchor = bounds.get();
            app.update_widgets_ui(|ui| ui.manage_menu = (!open).then_some((id, anchor)), cx);
        }))
        .child(theme::hover_tooltip(
            ("widget-handle-tooltip", id),
            "Drag to reorder, click to manage",
        ));
    div()
        .id(("widget-row", id))
        .group(group.clone())
        .relative()
        .pl(rpx(HANDLE_GUTTER))
        .on_drop(drop_handler(app, Some(id)))
        .when(!picked_up, |this| this.child(drop_line(group, -7.)))
        .child(handle)
        .child(
            div()
                .relative()
                .when(picked_up, |this| this.opacity(0.3))
                .child(super::styles_panel::measure_into(card_bounds))
                .child(element),
        )
        .into_any_element()
}

/// The white line Discord draws where a dragged widget would land, shown while one is over
/// `group`; `top` centres it in the gap above.
fn drop_line(group: SharedString, top: f32) -> impl IntoElement {
    div()
        .absolute()
        .top(rpx(top))
        .left(rpx(HANDLE_GUTTER))
        .right_0()
        .h(rpx(2.))
        .rounded_full()
        .bg(gpui::white())
        .opacity(0.)
        // gpui only checks drag-over styles on elements with a hitbox, and a group drag style
        // alone doesn't make one; an empty group hover style does.
        .group_hover(group.clone(), |style| style)
        .group_drag_over::<WidgetDrag>(group, |style| style.opacity(1.))
}

type MenuAction = Box<dyn Fn(&mut DiscordApp, &mut Context<DiscordApp>)>;

pub(super) fn manage_menu(
    app: &DiscordApp,
    card: &ProfileCard,
    editor: &StylesEditor,
) -> Option<AnyElement> {
    let (id, anchor) = editor.ui.widgets.manage_menu?;
    let widgets = &card.profile.as_ref()?.widgets;
    let index = widgets.iter().position(|widget| widget.id == id)?;
    let item = |key: &'static str, label: &'static str, danger: bool, action: MenuAction| {
        div()
            .id((key, id))
            .flex()
            .items_center()
            .h(rpx(32.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(if danger { theme::RED } else { theme::strong() }))
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(on_app(app, move |app, cx| {
                app.update_widgets_ui(|ui| ui.manage_menu = None, cx);
                action(app, cx);
            }))
            .child(label)
    };
    let mut items = Vec::new();
    if index > 0 {
        items.push(item(
            "widget-move-up",
            "Move Up",
            false,
            Box::new(move |app, cx| app.shift_widget(id, true, cx)),
        ));
    }
    if index + 1 < widgets.len() {
        items.push(item(
            "widget-move-down",
            "Move Down",
            false,
            Box::new(move |app, cx| app.shift_widget(id, false, cx)),
        ));
    }
    items.push(item(
        "widget-remove",
        "Remove Widget",
        true,
        Box::new(move |app, cx| app.remove_widget(id, cx)),
    ));
    Some(popover(
        app,
        "widget-manage-menu",
        anchor,
        160.,
        None,
        |app, cx| app.update_widgets_ui(|ui| ui.manage_menu = None, cx),
        div().flex().flex_col().children(items),
    ))
}

pub(super) fn popover(
    app: &DiscordApp,
    id: &'static str,
    anchor: Option<Bounds<Pixels>>,
    width: f32,
    max_height: Option<f32>,
    close: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
    content: impl IntoElement,
) -> AnyElement {
    let panel = popover_panel(id, width, max_height).child(content);
    anchored_popover(app, id, anchor, close, panel)
}

pub(super) fn popover_panel(
    id: &'static str,
    width: f32,
    max_height: Option<f32>,
) -> Stateful<Div> {
    div()
        .id(id)
        .without_text_selection()
        .w(rpx(width))
        .map(|this| match max_height {
            // Discord's scrolling menus leave the right padding to the scrollbar gutter.
            Some(height) => this
                .max_h(rpx(height))
                .overflow_y_scroll()
                .py(rpx(8.))
                .pl(rpx(8.)),
            None => this.p(rpx(6.)),
        })
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::surface_highest()))
        .shadow(vec![
            BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
        ])
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
}

pub(super) fn anchored_popover(
    app: &DiscordApp,
    id: &'static str,
    anchor: Option<Bounds<Pixels>>,
    close: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
    panel: impl IntoElement,
) -> AnyElement {
    let position = anchor.map_or_else(Point::default, |anchor| {
        point(anchor.left(), anchor.bottom() + px(4.))
    });
    div()
        .id(SharedString::from(format!("{id}-scrim")))
        .absolute()
        .inset_0()
        .occlude()
        .on_click(on_app(app, close))
        .child(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(2),
        )
        .into_any_element()
}

pub(super) fn tag_menu(
    app: &DiscordApp,
    card: &ProfileCard,
    editor: &StylesEditor,
) -> Option<AnyElement> {
    let (widget_id, game_id, anchor) = editor.ui.widgets.tag_menu?;
    let game = card
        .profile
        .as_ref()?
        .widgets
        .iter()
        .find(|widget| widget.id == widget_id)?
        .data
        .games()?
        .iter()
        .find(|game| game.game_id == game_id)?;
    let has = |tag: &str| game.tags.iter().any(|current| current == tag);
    let row = |tag: Option<&'static str>, checked: bool, radio: bool| {
        let label: SharedString =
            tag.map_or_else(|| "None".into(), |tag| super::board::tag_label(tag).into());
        // gpui draws thin borders on small circles unevenly, so rings are an icon and the
        // selected radio is two filled circles.
        let mark = if radio {
            let mark = div()
                .size(rpx(20.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center();
            if checked {
                mark.rounded_full()
                    .bg(rgb(theme::BLURPLE))
                    .child(div().size(rpx(8.)).rounded_full().bg(gpui::white()))
            } else {
                mark.child(theme::icon("radio-ring", 20., theme::muted()))
            }
        } else {
            div()
                .size(rpx(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(rpx(4.))
                .border_2()
                .border_color(if checked {
                    rgb(theme::BLURPLE)
                } else {
                    rgb(theme::muted())
                })
                .when(checked, |this| {
                    this.bg(rgb(theme::BLURPLE))
                        .child(theme::icon("check", 12., 0xFFFFFF))
                })
        };
        div()
            .id(SharedString::from(format!(
                "widget-tag-{}",
                tag.unwrap_or("none")
            )))
            .flex()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(32.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(on_app(app, move |app, cx| {
                app.toggle_widget_tag(widget_id, game_id, tag, cx)
            }))
            .child(
                div()
                    .flex_1()
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(label),
            )
            .child(mark)
    };
    let heading = |title: &'static str| {
        div()
            .px(rpx(8.))
            .pt(rpx(6.))
            .pb(rpx(2.))
            .text_size(rpx(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::muted()))
            .child(title)
    };
    let separator = || div().my(rpx(8.)).h(px(1.)).bg(rgba(theme::border_subtle()));
    let skilled = SKILL_TAGS.iter().any(|tag| has(tag));
    let content = div()
        .flex()
        .flex_col()
        .child(heading("Skill level"))
        .child(row(None, !skilled, true))
        .children(SKILL_TAGS.iter().map(|&tag| row(Some(tag), has(tag), true)))
        .child(separator())
        .child(heading("Rating"))
        .children(
            RATING_TAGS
                .iter()
                .map(|&tag| row(Some(tag), has(tag), false)),
        )
        .child(separator())
        .child(heading("Looking for"))
        .children(
            LOOKING_TAGS
                .iter()
                .map(|&tag| row(Some(tag), has(tag), false)),
        );
    Some(popover(
        app,
        "widget-tag-menu",
        anchor,
        TAG_MENU_SIZE[0],
        Some(TAG_MENU_SIZE[1]),
        |app, cx| app.update_widgets_ui(|ui| ui.tag_menu = None, cx),
        content,
    ))
}

pub(in crate::app::profile) fn game_row(
    app: &DiscordApp,
    widget_id: u64,
    entry: &WidgetGame,
    cover: AnyElement,
    name: Div,
    palette: ProfilePalette,
) -> AnyElement {
    let game_id = entry.game_id;
    let key = widget_id ^ game_id;
    let group: SharedString = format!("widget-game-{key}").into();
    let ui = app.profile_styles.as_ref().map(|editor| &editor.ui.widgets);
    let editing = ui
        .and_then(|ui| ui.comment.as_ref())
        .filter(|edit| edit.widget_id == widget_id && edit.game_id == game_id);
    let comment = match editing {
        Some(edit) => Textarea::new(&edit.input)
            .appearance(false)
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .into_any_element(),
        None => {
            let text = entry.comment.clone().unwrap_or_default();
            let empty = text.is_empty();
            div()
                .id(("widget-comment", key))
                .px(rpx(7.))
                .py(rpx(1.))
                .mx(rpx(-7.))
                .rounded(rpx(8.))
                .cursor_text()
                .hover(move |style| style.bg(palette.hover))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(if empty { palette.faint } else { palette.muted }))
                .child(if empty {
                    "Let everyone know why this is your favorite".to_owned()
                } else {
                    text.clone()
                })
                .on_click({
                    let entity = app.entity.clone();
                    move |_, window, cx| {
                        cx.stop_propagation();
                        let text = text.clone();
                        entity
                            .update(cx, |app, cx| {
                                app.start_widget_comment(widget_id, game_id, text, window, cx)
                            })
                            .ok();
                    }
                })
                .into_any_element()
        }
    };
    let expanded = ui.is_some_and(|ui| ui.expanded_tags.contains(&(widget_id, game_id)));
    let shown = if expanded {
        entry.tags.len()
    } else {
        entry.tags.len().min(SHOWN_TAGS)
    };
    let pill = || {
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .h(rpx(20.))
            .pl(rpx(6.))
            .pr(rpx(8.))
            .rounded(rpx(8.))
            .border_1()
            .text_size(rpx(12.))
            .font_weight(FontWeight::MEDIUM)
    };
    let tags = entry.tags[..shown].iter().map(|tag| {
        let tag_id = tag.clone();
        pill()
            .border_color(palette.border)
            .text_color(rgb(palette.muted))
            .child(theme::icon(super::board::tag_icon(tag), 12., palette.muted))
            .child(super::board::tag_label(tag))
            .child(
                div()
                    .id(SharedString::from(format!("widget-tag-remove-{key}-{tag}")))
                    .relative()
                    .cursor_pointer()
                    .child(theme::icon("x", 12., palette.muted))
                    .child(theme::hover_tooltip(
                        SharedString::from(format!("widget-tag-remove-tooltip-{key}-{tag}")),
                        "Remove Tag",
                    ))
                    .on_click(on_app(app, move |app, cx| {
                        let tag = tag_id.clone();
                        app.edit_widget_game(
                            widget_id,
                            game_id,
                            |game| game.tags.retain(|current| *current != tag),
                            cx,
                        );
                    })),
            )
            .into_any_element()
    });
    let hidden = entry.tags.len() - shown;
    let more = (hidden > 0).then(|| {
        pill()
            .id(("widget-tags-more", key))
            .border_color(palette.border)
            .text_color(rgb(palette.text))
            .cursor_pointer()
            .child(format!("+{hidden}"))
            .on_click(on_app(app, move |app, cx| {
                app.update_widgets_ui(
                    |ui| {
                        ui.expanded_tags.insert((widget_id, game_id));
                    },
                    cx,
                )
            }))
            .into_any_element()
    });
    let add_bounds = Rc::new(Cell::new(None));
    let add = pill()
        .id(("widget-tags-add", key))
        .relative()
        .border_dashed()
        .border_color(palette.border)
        .text_color(rgb(palette.text))
        .cursor_pointer()
        .child(super::styles_panel::measure_into(add_bounds.clone()))
        .child("+ Tags")
        .on_click(on_app(app, move |app, cx| {
            let anchor = add_bounds.get();
            app.update_widgets_ui(|ui| ui.tag_menu = Some((widget_id, game_id, anchor)), cx)
        }))
        .into_any_element();
    let remove = div()
        .id(("widget-game-remove", key))
        .absolute()
        .top(rpx(0.))
        .right(rpx(0.))
        .size(rpx(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgba(0x0000007A))
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.))
        .hover(|style| style.bg(rgba(0x000000CC)))
        .cursor_pointer()
        .child(theme::icon("trash-2", 16., 0xFFFFFF))
        .on_click(on_app(app, move |app, cx| {
            app.remove_widget_game(widget_id, game_id, cx)
        }))
        .child(theme::hover_tooltip(
            ("widget-game-remove-tooltip", key),
            "Remove Game",
        ));
    div()
        .group(group)
        .relative()
        .flex()
        .gap(rpx(16.))
        .child(cover)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .pr(rpx(40.))
                .child(name)
                .child(comment)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(rpx(4.))
                        .mt(rpx(4.))
                        .children(tags)
                        .children(more)
                        .child(add),
                ),
        )
        .child(remove)
        .into_any_element()
}
