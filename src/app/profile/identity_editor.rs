//! Your full profile's name lines while you style it, as Discord's editor has them: the display
//! name boxed with an Edit Style button, pronouns edited in place, and a picker for the server
//! tag shown beside your name.

use gpui_component::input::{Input, InputEvent, InputState, RopeExt, Textarea, TextareaState};
use gpui_component::scroll::ScrollableElement as _;

use super::styles::StylesEditor;
use super::styles_panel::{HoverFade, measure_into, on_app};
use super::*;
use crate::app::chat::{TEXTAREA_BUILTIN_INSET_X, TEXTAREA_BUILTIN_INSET_Y};
use crate::store::{Guild, GuildTag, NameStyle};

/// The longest display name and pronouns Discord takes, in characters.
const NAME_LIMIT: usize = 32;
const PRONOUNS_LIMIT: usize = 40;
const TAG_MENU_WIDTH: f32 = 280.;
const TAG_LIST_HEIGHT: f32 = 204.;
const TAG_ROW_HEIGHT: f32 = 40.;

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Name,
    Pronouns,
}

impl Field {
    fn limit(self) -> usize {
        match self {
            Self::Name => NAME_LIMIT,
            Self::Pronouns => PRONOUNS_LIMIT,
        }
    }
}

#[derive(Clone, Copy)]
enum Hover {
    Name,
    EditStyle,
    Pronouns,
    Tag,
}

/// The name wraps like Discord's, so it edits in a growing text area; pronouns stay one line.
enum EditInput {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

struct Edit {
    field: Field,
    input: EditInput,
    /// What the field opened with; finishing on it unchanged leaves the draft as it was, so
    /// reopening a cleared field doesn't turn the inherited text it shows into an edit.
    opened_with: String,
    _events: Subscription,
}

impl EditInput {
    fn value(&self, cx: &App) -> SharedString {
        match self {
            Self::Line(input) => input.read(cx).value(),
            Self::Area(input) => input.read(cx).value(),
        }
    }
}

/// `text` as one line of at most `limit` characters, or `None` when it already is.
fn clamped(text: &str, limit: usize) -> Option<String> {
    let line = text.replace(['\n', '\r'], "");
    let clamped: String = line.chars().take(limit).collect();
    (clamped != text).then_some(clamped)
}

impl IdentityUi {
    /// Forgets the name and pronoun edits, which belong to one profile scope; the tag is
    /// account-wide and stays.
    pub(super) fn leave_scope(&mut self) {
        self.editing = None;
        self.display_name = None;
        self.pronouns = None;
    }
}

#[derive(Default)]
pub(crate) struct IdentityUi {
    hovers: [HoverFade; 4],
    /// The open tag picker, below the pill it hangs from.
    tag_menu: Option<Option<Bounds<Pixels>>>,
    tag_search: Option<Entity<InputState>>,
    editing: Option<Edit>,
    /// Edits shown in the preview; saving them comes with the profile save.
    pub(crate) display_name: Option<String>,
    pub(crate) pronouns: Option<String>,
    /// `Some(None)` takes the tag off.
    pub(crate) tag_guild: Option<Option<u64>>,
}

impl DiscordApp {
    fn update_identity_ui(&mut self, update: impl FnOnce(&mut IdentityUi), cx: &mut Context<Self>) {
        self.update_styles_ui(|ui| update(&mut ui.identity), cx);
    }

    fn start_identity_edit(
        &mut self,
        field: Field,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_identity_edit(cx);
        let limit = field.limit();
        let opened_with = match field {
            Field::Name => String::new(),
            Field::Pronouns => current.clone(),
        };
        let (input, events) = match field {
            Field::Name => {
                // Discord's field opens empty, with the current name faded behind it.
                let area = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 3));
                area.update(cx, |area, cx| area.focus(window, cx));
                let events =
                    cx.subscribe_in(
                        &area,
                        window,
                        move |app, area, event, window, cx| match event {
                            InputEvent::PressEnter { shift: false, .. } | InputEvent::Blur => {
                                app.finish_identity_edit(cx);
                            }
                            InputEvent::Change => {
                                let value = area.read(cx).value();
                                if let Some(text) = clamped(&value, limit) {
                                    area.update(cx, |area, cx| {
                                        area.set_value(text, window, cx);
                                        let end = area.text().offset_to_position(area.text().len());
                                        area.set_cursor_position(end, window, cx);
                                    });
                                }
                                // The field hugs its text, so it resizes as you type.
                                cx.notify();
                            }
                            _ => {}
                        },
                    );
                (EditInput::Area(area), events)
            }
            Field::Pronouns => {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
                input.update(cx, |input, cx| input.focus(window, cx));
                let events =
                    cx.subscribe_in(&input, window, move |app, input, event, window, cx| {
                        match event {
                            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                                app.finish_identity_edit(cx);
                            }
                            InputEvent::Change => {
                                let value = input.read(cx).value();
                                if let Some(text) = clamped(&value, limit) {
                                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                                }
                                // The field hugs its text, so it resizes as you type.
                                cx.notify();
                            }
                            _ => {}
                        }
                    });
                (EditInput::Line(input), events)
            }
        };
        self.update_identity_ui(
            |ui| {
                ui.editing = Some(Edit {
                    field,
                    input,
                    opened_with,
                    _events: events,
                })
            },
            cx,
        );
    }

    /// Keeps what's typed in the field being edited; false when none was.
    pub(crate) fn finish_identity_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.profile_styles.as_mut() else {
            return false;
        };
        let ui = &mut editor.ui.identity;
        let Some(Edit {
            field,
            input,
            opened_with,
            ..
        }) = ui.editing.take()
        else {
            return false;
        };
        let text: String = input
            .value(cx)
            .replace(['\n', '\r'], "")
            .trim()
            .chars()
            .take(field.limit())
            .collect();
        match field {
            // The name field opens empty, so leaving it empty keeps the name it opened over.
            _ if text == opened_with.trim() => {}
            Field::Name => ui.display_name = Some(text),
            Field::Pronouns => ui.pronouns = Some(text),
        }
        cx.notify();
        true
    }

    fn open_tag_menu(
        &mut self,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        search.update(cx, |input, cx| input.focus(window, cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        self.update_identity_ui(
            |ui| {
                ui.tag_menu = Some(anchor);
                ui.tag_search = Some(search);
            },
            cx,
        );
    }

    pub(crate) fn close_tag_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.profile_styles.as_mut() else {
            return false;
        };
        let open = editor.ui.identity.tag_menu.take().is_some();
        editor.ui.identity.tag_search = None;
        if open {
            cx.notify();
        }
        open
    }

    fn pick_server_tag(&mut self, guild_id: Option<u64>, cx: &mut Context<Self>) {
        self.update_identity_ui(|ui| ui.tag_guild = Some(guild_id), cx);
        self.close_tag_menu(cx);
    }
}

/// Whether the name, pronouns or server tag differ from what's saved, which the save bar asks
/// about.
pub(super) fn edited(app: &DiscordApp, editor: &StylesEditor) -> bool {
    changes(app, editor) != Changes::default()
}

/// The name, pronouns and tag edits that differ from what's saved, as the save sends them.
#[derive(Default, PartialEq)]
pub(super) struct Changes {
    pub(super) name: Option<String>,
    pub(super) pronouns: Option<String>,
    pub(super) tag_guild: Option<Option<u64>>,
}

pub(super) fn changes(app: &DiscordApp, editor: &StylesEditor) -> Changes {
    let ui = &editor.ui.identity;
    let me = app.store.current_user_id;
    let guild_id = editor.scope.guild_id();
    let saved_name = app
        .store
        .member_name(guild_id.unwrap_or(0), me)
        .map(str::to_owned)
        .or_else(|| app.store.user(me).map(|user| user.name.clone()))
        .unwrap_or_default();
    let (own_pronouns, shown_pronouns) =
        scope_pronouns(editor.saved_profile.as_deref(), guild_id.is_some());
    let worn = app
        .store
        .user(me)
        .and_then(|user| user.tag.as_ref())
        .map(|tag| tag.guild_id);
    Changes {
        name: ui.display_name.clone().filter(|name| *name != saved_name),
        // Matching what shows changes nothing, and clearing a server's pronouns when it has none
        // of its own leaves the main profile's showing, as before.
        pronouns: ui.pronouns.clone().filter(|pronouns| {
            *pronouns != shown_pronouns
                && !(guild_id.is_some() && pronouns.is_empty() && own_pronouns.is_empty())
        }),
        tag_guild: ui.tag_guild.filter(|guild| *guild != worn),
    }
}

/// Your pronouns as a scope stores them, and as its profile shows them: a server profile with
/// none of its own shows the main profile's.
pub(super) fn scope_pronouns(profile: Option<&model::Profile>, guild: bool) -> (String, String) {
    let main = profile
        .and_then(|profile| profile.user_profile.as_ref())
        .and_then(|profile| profile.pronouns.clone())
        .unwrap_or_default();
    if !guild {
        return (main.clone(), main);
    }
    let own = profile
        .and_then(super::loading::guild_profile)
        .and_then(|profile| profile.pronouns.clone())
        .unwrap_or_default();
    let shown = if own.trim().is_empty() {
        main
    } else {
        own.clone()
    };
    (own, shown)
}

/// Servers whose tag you could wear, in your server list's order, as Discord lists them.
fn tag_options(app: &DiscordApp) -> Vec<(&Guild, GuildTag)> {
    app.store
        .guilds
        .iter()
        .filter_map(|guild| Some((guild, guild.tag.clone()?)))
        .collect()
}

/// The tag your profile shows while styling: the picked one, else the one you wear.
pub(super) fn shown_tag(app: &DiscordApp, editor: &StylesEditor) -> Option<GuildTag> {
    let worn = || {
        app.store
            .user(app.store.current_user_id)
            .and_then(|user| user.tag.clone())
    };
    match editor.ui.identity.tag_guild {
        None => worn(),
        Some(None) => None,
        Some(Some(guild_id)) => worn().filter(|tag| tag.guild_id == guild_id).or_else(|| {
            tag_options(app)
                .into_iter()
                .find(|(guild, _)| guild.id == guild_id)
                .map(|(_, tag)| tag)
        }),
    }
}

fn track_hover(app: &DiscordApp, which: Hover) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |hovered, _, cx| {
        entity
            .update(cx, |app, cx| {
                app.update_identity_ui(|ui| ui.hovers[which as usize].set(*hovered), cx)
            })
            .ok();
    }
}

/// The rounded box behind a hovered field, fading in and out with the pointer; `held` keeps it
/// shown, as while the field is being edited.
fn hover_box(
    editor: &StylesEditor,
    which: Hover,
    id: &'static str,
    fill: Hsla,
    border: Option<Hsla>,
    held: bool,
) -> AnyElement {
    let layer = div()
        .absolute()
        .inset_0()
        .rounded(rpx(8.))
        .bg(fill)
        .when_some(border, |this, border| this.border_1().border_color(border));
    if held {
        return layer.into_any_element();
    }
    editor.ui.identity.hovers[which as usize].fade(id, layer)
}

/// While `field` is being edited, a text field the size of what's typed in it: an invisible
/// copy of the text sizes it, and the field sits over that.
fn editing_input(
    editor: &StylesEditor,
    field: Field,
    size: f32,
    line_height: f32,
    cx: &App,
) -> Option<AnyElement> {
    let Edit {
        input: EditInput::Line(input),
        ..
    } = editor
        .ui
        .identity
        .editing
        .as_ref()
        .filter(|edit| edit.field == field)?
    else {
        return None;
    };
    let text = input.read(cx).value().to_string();
    Some(
        div()
            .relative()
            .min_w(rpx(24.))
            .child(
                div()
                    .invisible()
                    .whitespace_nowrap()
                    .text_size(rpx(size))
                    .line_height(rpx(line_height))
                    .pr(rpx(2.))
                    .child(if text.is_empty() {
                        " ".to_owned()
                    } else {
                        text
                    }),
            )
            .child(
                // gpui-component scrolls a single-line input to keep the caret 10px from its
                // right edge; reaching that far past the text keeps it from scrolling.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .right(px(-10.))
                    .child(
                        // `Input::h` only sizes multi-line inputs; a single line needs the
                        // style's own height, or it stays its default 32px and centers the text
                        // below the bubble.
                        Styled::h(
                            Input::new(input)
                                .appearance(false)
                                .p_0()
                                .line_height(rpx(line_height))
                                .text_size(rpx(size)),
                            rpx(line_height),
                        ),
                    ),
            )
            .into_any_element(),
    )
}

/// The name field while it's edited, as Discord's: it opens empty over your name, faded, and
/// what you type takes the name's font and effect. The styled text underneath also sizes the
/// field, so it hugs the name and wraps at the card's edge; the text area over it only shows
/// its caret and selection.
fn name_input(
    editor: &StylesEditor,
    current: &str,
    style: Option<&NameStyle>,
    cx: &App,
) -> Option<AnyElement> {
    let Edit {
        input: EditInput::Area(area),
        ..
    } = editor
        .ui
        .identity
        .editing
        .as_ref()
        .filter(|edit| edit.field == Field::Name)?
    else {
        return None;
    };
    let font = crate::fonts::display_font(style.and_then(|style| style.font_id), cx);
    let text = area.read(cx).value().to_string();
    let empty = text.is_empty();
    // Discord's placeholder is the name washed toward white, not just dimmed.
    let faded = style.map(|style| NameStyle {
        colors: style
            .colors
            .iter()
            .map(|color| super::palette::blend(color & 0xFFFFFF, 0xFFFFFF, 0.45))
            .collect(),
        ..style.clone()
    });
    Some(
        div()
            .relative()
            .min_w(rpx(24.))
            .max_w_full()
            .when_some(font, |this, font| this.font_family(font.family))
            .child(div().when(empty, |this| this.opacity(0.7)).child(
                crate::app::display_name::render(
                    "identity-name-preview",
                    if empty { current } else { &text },
                    if empty { faded.as_ref() } else { style },
                    crate::app::display_name::Surface::Field,
                    cx,
                ),
            ))
            .child(
                // Stretched over the text area's own padding, so it wraps where the name does.
                div()
                    .absolute()
                    .top(px(-TEXTAREA_BUILTIN_INSET_Y))
                    .left(px(-TEXTAREA_BUILTIN_INSET_X))
                    .right(px(-TEXTAREA_BUILTIN_INSET_X))
                    .child(
                        Textarea::new(area)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(rpx(24.))
                            .line_height(rpx(30.))
                            .text_color(transparent_black()),
                    ),
            )
            .into_any_element(),
    )
}

/// The display name in a box that shows on hover: click it to rename yourself, or use
/// Discord's Edit Style button for its look.
pub(super) fn name_field(
    app: &DiscordApp,
    editor: &StylesEditor,
    name: AnyElement,
    current: String,
    style: Option<&NameStyle>,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let editing = name_input(editor, &current, style, cx);
    let is_editing = editing.is_some();
    let entity = app.entity.clone();
    let edit_style = div()
        .id("identity-edit-style")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(28.))
        .cursor_pointer()
        .on_hover(track_hover(app, Hover::EditStyle))
        .on_click(on_app(app, |app, cx| app.open_name_style_dialog(cx)))
        .child(hover_box(
            editor,
            Hover::EditStyle,
            "identity-edit-style-box",
            palette.panel,
            None,
            false,
        ))
        .child(theme::icon("paintbrush", 16., palette.text))
        .child(theme::hover_tooltip(
            "identity-edit-style-tip",
            "Edit Style",
        ));
    div()
        .id("identity-name")
        .relative()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .max_w_full()
        .ml(rpx(-9.))
        .pl(rpx(9.))
        .pr(rpx(5.))
        .py(rpx(3.))
        .self_start()
        .on_hover(track_hover(app, Hover::Name))
        .map(|this| {
            if is_editing {
                // Clicking anywhere else ends the edit, keeping what was typed.
                let finish = app.entity.clone();
                this.on_mouse_down_out(move |_, _, cx| {
                    finish
                        .update(cx, |app, cx| {
                            app.finish_identity_edit(cx);
                        })
                        .ok();
                })
                .child(hover_box(
                    editor,
                    Hover::Name,
                    "identity-name-box",
                    rgba(0x0003081F).into(),
                    Some(rgb(theme::BLURPLE).into()),
                    true,
                ))
            } else {
                this.cursor_text()
                    .child(hover_box(
                        editor,
                        Hover::Name,
                        "identity-name-box",
                        palette.hover,
                        Some(palette.border),
                        false,
                    ))
                    .on_click(move |_, window, cx| {
                        let current = current.clone();
                        entity
                            .update(cx, |app, cx| {
                                app.start_identity_edit(Field::Name, current, window, cx)
                            })
                            .ok();
                    })
            }
        })
        .child(
            div()
                .flex_initial()
                .min_w_0()
                .text_size(rpx(24.))
                .line_height(rpx(30.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.strong))
                .child(editing.unwrap_or(name)),
        )
        .child(if is_editing {
            div().child(edit_style).into_any_element()
        } else {
            editor.ui.identity.hovers[Hover::Name as usize]
                .fade("identity-edit-style-fade", div().child(edit_style))
        })
        .into_any_element()
}

/// Pronouns that turn into a text field when clicked, boxed on hover like Discord's.
pub(super) fn pronouns_field(
    app: &DiscordApp,
    editor: &StylesEditor,
    pronouns: Option<String>,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    // `pronouns` already holds any edit, with cleared server pronouns falling back to the main
    // profile's.
    let shown = pronouns.unwrap_or_default();
    let editing = editing_input(editor, Field::Pronouns, 14., 18., cx);
    let field = div()
        .id("identity-pronouns")
        .relative()
        .flex()
        .items_center()
        .h(rpx(24.))
        .px(rpx(7.))
        .text_color(rgb(palette.strong))
        .on_hover(track_hover(app, Hover::Pronouns))
        .child(hover_box(
            editor,
            Hover::Pronouns,
            "identity-pronouns-box",
            palette.hover,
            Some(palette.border),
            editing.is_some(),
        ));
    if let Some(input) = editing {
        let finish = app.entity.clone();
        return field
            .on_mouse_down_out(move |_, _, cx| {
                finish
                    .update(cx, |app, cx| {
                        app.finish_identity_edit(cx);
                    })
                    .ok();
            })
            .child(input)
            .into_any_element();
    }
    let entity = app.entity.clone();
    let current = shown.clone();
    field
        .cursor_text()
        .on_click(move |_, window, cx| {
            let current = current.clone();
            entity
                .update(cx, |app, cx| {
                    app.start_identity_edit(Field::Pronouns, current, window, cx)
                })
                .ok();
        })
        .map(|this| {
            if shown.is_empty() {
                this.text_color(rgb(palette.faint)).child("Pronouns")
            } else {
                this.child(shown)
            }
        })
        .into_any_element()
}

/// The worn server tag with a chevron that opens the tag picker.
pub(super) fn tag_pill(
    app: &DiscordApp,
    editor: &StylesEditor,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let open = editor.ui.identity.tag_menu.is_some();
    let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let anchor = bounds.clone();
    let entity = app.entity.clone();
    div()
        .id("identity-tag")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(24.))
        .pl(rpx(8.))
        .pr(rpx(6.))
        .rounded(rpx(8.))
        .cursor_pointer()
        // Discord's pill is a fixed grey whatever the profile theme; hovered and open are a
        // guess one step stronger.
        .bg(rgba(0x97979F1F))
        .on_hover(track_hover(app, Hover::Tag))
        .child(hover_box(
            editor,
            Hover::Tag,
            "identity-tag-box",
            rgba(0x97979F1F).into(),
            None,
            open,
        ))
        .child(measure_into(bounds))
        .map(|this| match shown_tag(app, editor) {
            Some(tag) => this.child(tag_label(&tag, 12., palette.strong, cx)),
            None => this
                .text_size(rpx(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child("No Server Tag"),
        })
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            16.,
            palette.text,
        ))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| {
                    if !app.close_tag_menu(cx) {
                        app.open_tag_menu(anchor.get(), window, cx);
                    }
                })
                .ok();
        })
        .into_any_element()
}

fn tag_label(tag: &GuildTag, size: f32, color: u32, cx: &App) -> Div {
    let badge = tag
        .badge_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx));
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .text_size(rpx(size))
        .line_height(rpx(size + 4.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(color))
        .children(badge.map(|path| img(path).size(rpx(size)).flex_none()))
        .child(tag.text.clone())
}

fn guild_icon(guild: &Guild, cx: &App) -> AnyElement {
    let size = 20.;
    match guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx))
    {
        Some(path) => img(path)
            .size(rpx(size))
            .rounded_full()
            .flex_none()
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(guild.icon_color))
            .text_size(rpx(10.))
            .text_color(gpui::black())
            .child(guild.initials.clone())
            .into_any_element(),
    }
}

fn tag_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(TAG_ROW_HEIGHT))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.bg(rgba(theme::selected()))
            } else {
                this.hover(|style| style.bg(rgba(theme::hover())))
            }
        })
}

/// Discord's server tag picker: a search over the servers whose tag you could wear.
pub(super) fn tag_menu(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> Option<AnyElement> {
    let ui = &editor.ui.identity;
    let anchor = ui.tag_menu?;
    let search = ui.tag_search.as_ref()?;
    let query = search.read(cx).value().trim().to_lowercase();
    let current = shown_tag(app, editor).map(|tag| tag.guild_id);
    let options: Vec<_> = tag_options(app)
        .into_iter()
        .filter(|(guild, tag)| {
            query.is_empty()
                || guild.name.to_lowercase().contains(&query)
                || tag.text.to_lowercase().contains(&query)
        })
        .collect();
    // The scrollbar wrapper only scrolls within a definite height, not a max height.
    let list_height = ((options.len() + 1) as f32 * TAG_ROW_HEIGHT).min(TAG_LIST_HEIGHT);
    let rows = options.into_iter().map(|(guild, tag)| {
        let guild_id = guild.id;
        tag_row(("identity-tag-option", guild_id), current == Some(guild_id))
            .on_click(on_app(app, move |app, cx| {
                app.pick_server_tag(Some(guild_id), cx)
            }))
            .child(guild_icon(guild, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::text()))
                    .child(guild.name.clone()),
            )
            .child(
                tag_label(&tag, 12., theme::strong(), cx)
                    .flex_none()
                    .px(rpx(4.))
                    .rounded(rpx(4.))
                    .bg(rgba(theme::selected())),
            )
    });
    let none = tag_row("identity-tag-none", current.is_none())
        .on_click(on_app(app, |app, cx| app.pick_server_tag(None, cx)))
        .text_size(rpx(16.))
        .italic()
        .text_color(rgb(theme::muted()))
        .child("No Server Tag");
    let content = div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(Input::new(search))
        .child(
            div()
                .id("identity-tag-list")
                .flex()
                .flex_col()
                .h(rpx(list_height))
                .overflow_y_scrollbar()
                .child(none)
                .children(rows),
        );
    let panel = super::widget_editor::popover_panel("identity-tag-menu", TAG_MENU_WIDTH, None)
        .p(rpx(8.))
        .bg(rgb(theme::elevated()))
        .child(content);
    Some(super::widget_editor::anchored_popover(
        app,
        "identity-tag-menu",
        anchor,
        |app, cx| {
            app.close_tag_menu(cx);
        },
        panel,
    ))
}
