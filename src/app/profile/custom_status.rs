use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};
use gpui_component::ElementExt as _;
use gpui_component::input::{Input, InputEvent, InputState};

use super::popout::{ShownStatus, status_emoji_source, status_preview};
use super::*;
use crate::{
    app::accounts::modal::{self, Dialog, MEDIUM},
    app::expressions::emoji_picker::PickerTarget,
    discord::{Error, settings_proto},
    store::{CustomStatus, EmojiChoice, StatusEmoji},
};

const MAX_LENGTH: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClearAfter {
    Day,
    FourHours,
    Hour,
    HalfHour,
    Never,
    Existing(SystemTime),
}

impl ClearAfter {
    const CHOICES: [Self; 5] = [
        Self::Day,
        Self::FourHours,
        Self::Hour,
        Self::HalfHour,
        Self::Never,
    ];

    fn expires_at(self, now: SystemTime) -> Option<SystemTime> {
        let minutes = match self {
            Self::Day => 24 * 60,
            Self::FourHours => 4 * 60,
            Self::Hour => 60,
            Self::HalfHour => 30,
            Self::Never => return None,
            Self::Existing(at) => return Some(at),
        };
        now.checked_add(Duration::from_secs(minutes * 60))
    }

    fn label(self) -> &'static str {
        match self {
            Self::Day => "24 hours",
            Self::FourHours => "4 hours",
            Self::Hour => "1 hour",
            Self::HalfHour => "30 minutes",
            Self::Never | Self::Existing(_) => "Don't clear",
        }
    }

    fn choice_label(self, now: SystemTime) -> String {
        match self.expires_at(now) {
            Some(at) => format!("{} ({})", self.label(), clock_hint(at)),
            None => self.label().to_owned(),
        }
    }

    fn trigger_label(self, now: SystemTime) -> String {
        match self.expires_at(now) {
            Some(at) => {
                let hint = clock_hint(at);
                if hint.contains(" at ") {
                    format!("Clear {hint}")
                } else {
                    format!("Clear at {hint}")
                }
            }
            None => "Don't clear".to_owned(),
        }
    }
}

fn clock_hint(at: SystemTime) -> String {
    let at: DateTime<Local> = at.into();
    let today = Local::now().date_naive();
    let time = at.format("%-I:%M %p");
    if at.date_naive() == today {
        time.to_string()
    } else if today.succ_opt() == Some(at.date_naive()) {
        format!("tomorrow at {time}")
    } else {
        format!("{} at {time}", at.format("%b %-d"))
    }
}

pub(crate) struct StatusEditor {
    popout_card: Option<ProfileCard>,
    placeholder: SharedString,
    input: Entity<InputState>,
    emoji: Option<StatusEmoji>,
    clear_after: ClearAfter,
    clear_menu: bool,
    saving: bool,
    error: Option<SharedString>,
    _events: Subscription,
}

impl StatusEditor {
    fn draft(&self, cx: &App) -> Option<CustomStatus> {
        let text: String = self
            .input
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(MAX_LENGTH)
            .collect();
        (!text.is_empty() || self.emoji.is_some()).then(|| CustomStatus {
            text: (!text.is_empty()).then_some(text),
            emoji: self.emoji.clone(),
            expires_at: None,
        })
    }
}

fn wire_status(
    status: &CustomStatus,
    expires_at: Option<SystemTime>,
) -> settings_proto::CustomStatus {
    let (emoji_id, emoji_name) = match &status.emoji {
        Some(StatusEmoji::Unicode(sequence)) => (0, sequence.clone()),
        Some(StatusEmoji::Custom { id, name, .. }) => (*id, name.clone()),
        None => (0, String::new()),
    };
    settings_proto::CustomStatus {
        text: status.text.clone().unwrap_or_default(),
        emoji_id,
        emoji_name,
        expires_at_ms: expires_at
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_millis() as u64),
    }
}

impl DiscordApp {
    pub(crate) fn open_status_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let own = self.store.current_user_id;
        let over_full_profile = self
            .profile
            .as_ref()
            .is_some_and(|card| card.modal && card.user_id == own);
        let card = if over_full_profile {
            None
        } else {
            self.drop_styles_editor();
            self.profile.take().filter(|card| card.user_id == own)
        };
        let placeholder: SharedString = card
            .as_ref()
            .or(self.profile.as_ref())
            .map_or(STATUS_PROMPTS[1], |card| STATUS_PROMPTS[card.status_prompt])
            .into();
        let current = self
            .store
            .user(self.store.current_user_id)
            .and_then(|user| user.custom_status.clone())
            .filter(|status| !status.is_expired(SystemTime::now()));
        let text = current
            .as_ref()
            .and_then(|status| status.text.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder.clone());
            state.set_value(text, window, cx);
            state
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let events = cx.subscribe_in(
            &input,
            window,
            |app, input, event, window, cx| match event {
                InputEvent::Change => {
                    let value = input.read(cx).value();
                    if value.chars().count() > MAX_LENGTH {
                        let kept: String = value.chars().take(MAX_LENGTH).collect();
                        input.update(cx, |input, cx| input.set_value(kept, window, cx));
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => app.save_status_editor(cx),
                _ => {}
            },
        );
        let clear_after = match &current {
            None => ClearAfter::Day,
            Some(status) => status
                .expires_at
                .map_or(ClearAfter::Never, ClearAfter::Existing),
        };
        self.status_editor = Some(StatusEditor {
            popout_card: card,
            placeholder,
            input,
            emoji: current.and_then(|status| status.emoji),
            clear_after,
            clear_menu: false,
            saving: false,
            error: None,
            _events: events,
        });
        cx.notify();
    }

    pub(crate) fn close_status_editor(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = &mut self.status_editor else {
            return false;
        };
        if editor.clear_menu {
            editor.clear_menu = false;
        } else {
            self.status_editor = None;
        }
        cx.notify();
        true
    }

    pub(crate) fn set_status_emoji(&mut self, choice: &EmojiChoice, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.status_editor {
            editor.emoji = Some(match choice {
                EmojiChoice::Unicode { emoji, .. } => StatusEmoji::Unicode(emoji.clone()),
                EmojiChoice::Custom(emoji) => StatusEmoji::Custom {
                    id: emoji.id,
                    name: emoji.name.clone(),
                    animated: emoji.animated,
                },
            });
            cx.notify();
        }
    }

    fn clear_status_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.status_editor else {
            return;
        };
        editor.emoji = None;
        editor.error = None;
        let input = editor.input.clone();
        input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn toggle_clear_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.status_editor {
            editor.clear_menu = !editor.clear_menu;
            cx.notify();
        }
    }

    fn choose_clear_after(&mut self, choice: ClearAfter, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.status_editor {
            editor.clear_after = choice;
            editor.clear_menu = false;
            cx.notify();
        }
    }

    fn save_status_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.status_editor else {
            return;
        };
        if editor.saving {
            return;
        }
        let expires_at = editor.clear_after.expires_at(SystemTime::now());
        let custom = editor
            .draft(cx)
            .map(|status| wire_status(&status, expires_at));
        editor.saving = true;
        editor.error = None;
        cx.notify();
        self.write_custom_status(custom, true, cx);
    }

    pub(crate) fn clear_custom_status(&mut self, cx: &mut Context<Self>) {
        self.write_custom_status(None, false, cx);
    }

    fn write_custom_status(
        &mut self,
        custom: Option<settings_proto::CustomStatus>,
        from_editor: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            if let Some(editor) = self.status_editor.as_mut().filter(|_| from_editor) {
                editor.saving = false;
                editor.error = Some("You're offline right now.".into());
                cx.notify();
            }
            return;
        };
        let request = Tokio::spawn(cx, async move { session.set_custom_status(custom).await });
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or(Err(Error::Closed));
            this.update(cx, |app, cx| {
                match result {
                    Ok(stored) => {
                        app.store.apply_own_status(Some(&stored), false);
                        if from_editor {
                            app.status_editor = None;
                        }
                    }
                    Err(error) => {
                        log::warn!("Could not update custom status: {error}");
                        if let Some(editor) = app.status_editor.as_mut().filter(|_| from_editor) {
                            editor.saving = false;
                            editor.error = Some("Couldn't save your status. Try again.".into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn emoji_button(app: &DiscordApp, emoji: Option<&StatusEmoji>, cx: &App) -> AnyElement {
    let entity = app.entity.clone();
    let label: SharedString = match emoji {
        Some(StatusEmoji::Unicode(sequence)) => format!("Status emoji: {sequence}").into(),
        Some(StatusEmoji::Custom { name, .. }) => format!("Status emoji: {name}").into(),
        None => "Choose an emoji".into(),
    };
    div()
        .id("status-editor-emoji")
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(46.))
        .pl(rpx(4.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            let position = event.position;
            entity
                .update(cx, |app, cx| {
                    app.toggle_emoji_picker(PickerTarget::CustomStatus, position, window, cx)
                })
                .ok();
        })
        .child(
            match emoji.and_then(|emoji| status_emoji_source(emoji, 24., cx)) {
                Some(source) => img(source)
                    .size(rpx(24.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => theme::icon("smile", 22., theme::muted()).into_any_element(),
            },
        )
        .into_any_element()
}

fn clear_button(app: &DiscordApp) -> AnyElement {
    let entity = app.entity.clone();
    div()
        .id("status-editor-clear")
        .role(Role::Button)
        .aria_label("Clear custom status")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(rpx(40.))
        .h(rpx(46.))
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.clear_status_draft(window, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(16.))
                .rounded_full()
                .bg(rgb(theme::muted()))
                .child(theme::icon("x", 11., theme::bg())),
        )
        .into_any_element()
}

fn clear_after_menu(
    app: &DiscordApp,
    selected: ClearAfter,
    trigger: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> AnyElement {
    let close = app.entity.clone();
    let now = SystemTime::now();
    let menu = div()
        .id("status-editor-clear-menu")
        .role(Role::ListBox)
        .flex()
        .flex_col()
        .w(rpx(260.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |event, _, cx| {
            if trigger
                .get()
                .is_some_and(|bounds| bounds.contains(&event.position))
            {
                return;
            }
            close.update(cx, |app, cx| app.toggle_clear_menu(cx)).ok();
        })
        .child(
            div()
                .px(rpx(8.))
                .pt(rpx(4.))
                .pb(rpx(8.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::muted()))
                .child("Clear after"),
        )
        .children(
            ClearAfter::CHOICES
                .into_iter()
                .enumerate()
                .map(|(index, choice)| {
                    let entity = app.entity.clone();
                    let chosen = choice == selected;
                    div()
                        .id(("status-editor-clear-choice", index))
                        .role(Role::ListBoxOption)
                        .aria_selected(chosen)
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .p(rpx(8.))
                        .rounded(rpx(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::selected())))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if chosen {
                            theme::strong()
                        } else {
                            theme::text()
                        }))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            entity
                                .update(cx, |app, cx| app.choose_clear_after(choice, cx))
                                .ok();
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(choice.choice_label(now)),
                        )
                        .when(chosen, |this| {
                            this.child(theme::icon("check", 16., theme::BLURPLE))
                        })
                }),
        );
    div()
        .absolute()
        .left_0()
        .bottom_full()
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(3),
        )
        .into_any_element()
}

pub(crate) fn status_editor_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let editor = app.status_editor.as_ref()?;
    let draft = editor.draft(cx);
    let has_draft = draft.is_some();
    let shown = match draft {
        Some(status) => ShownStatus {
            status,
            prompt: false,
        },
        None => ShownStatus {
            status: CustomStatus {
                text: Some(editor.placeholder.to_string()),
                emoji: None,
                expires_at: None,
            },
            prompt: true,
        },
    };
    let own = app.store.current_user_id;
    let preview = editor
        .popout_card
        .as_ref()
        .or(app.profile.as_ref().filter(|card| card.user_id == own))
        .map(|card| status_preview(app, card, shown, cx));
    let field = div()
        .flex()
        .items_center()
        .h(rpx(48.))
        .rounded(rpx(8.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(rgba(theme::border()))
        .child(emoji_button(app, editor.emoji.as_ref(), cx))
        .child(
            div().flex_1().min_w_0().pl(rpx(2.)).child(
                Input::new(&editor.input)
                    .appearance(false)
                    .px_0()
                    .h(rpx(46.))
                    .text_size(rpx(16.)),
            ),
        )
        .when(has_draft, |this| this.child(clear_button(app)));
    let body = div()
        .flex()
        .flex_col()
        .children(preview.map(|preview| div().flex().justify_center().mb(rpx(36.)).child(preview)))
        .child(
            div()
                .mb(rpx(4.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Status"),
        )
        .child(field)
        .children(editor.error.clone().map(|error| {
            div()
                .mt(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .into_any_element();
    let trigger: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let paint = trigger.clone();
    let toggle = app.entity.clone();
    let save = app.entity.clone();
    let enabled = !editor.saving;
    let trigger_label = editor.clear_after.trigger_label(SystemTime::now());
    let footer = div()
        .flex()
        .flex_1()
        .items_center()
        .child(
            div()
                .relative()
                .child(
                    div()
                        .id("status-editor-clear-after")
                        .role(Role::Button)
                        .aria_label(format!("Clear after, {trigger_label}"))
                        .aria_expanded(editor.clear_menu)
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .h(rpx(40.))
                        .cursor_pointer()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .text_color(rgb(theme::text()))
                        .hover(|style| style.text_color(rgb(theme::strong())))
                        .on_prepaint(move |bounds, _, _| paint.set(Some(bounds)))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            toggle.update(cx, |app, cx| app.toggle_clear_menu(cx)).ok();
                        })
                        .child(trigger_label)
                        .child(theme::icon(
                            if editor.clear_menu {
                                "chevron-up"
                            } else {
                                "chevron-down"
                            },
                            16.,
                            theme::muted(),
                        )),
                )
                .when(editor.clear_menu, |this| {
                    this.child(clear_after_menu(app, editor.clear_after, trigger))
                }),
        )
        .child(div().flex_1())
        .child(
            modal::button("status-editor-save", "Save", true, enabled)
                .w(rpx(100.))
                .when(enabled, |this| {
                    this.on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        save.update(cx, |app, cx| app.save_status_editor(cx)).ok();
                    })
                }),
        )
        .into_any_element();
    let close = app.entity.clone();
    Some(modal::dialog(
        Dialog {
            id: "status-editor",
            width: MEDIUM,
            title: "Set your status",
            subtitle: None,
            body,
            footer,
        },
        move |_, cx| {
            close
                .update(cx, |app, cx| {
                    app.status_editor = None;
                    cx.notify();
                })
                .ok();
        },
    ))
}
