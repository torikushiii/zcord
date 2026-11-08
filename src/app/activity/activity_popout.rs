use std::time::Duration;

use gpui::*;
use gpui_component::input::{InputEvent, InputState};

use super::super::DiscordApp;
use crate::store::{Component, EmojiChoice};

const CLOSE_DELAY: Duration = Duration::from_millis(150);

pub(crate) struct ActivityPopout {
    pub entry_id: u64,
    pub top: Pixels,
    pub menu: bool,
    pub input: Entity<InputState>,
    over_card: bool,
    over_popout: bool,
    closing: Option<Task<()>>,
    _events: Subscription,
}

impl DiscordApp {
    pub(crate) fn hover_activity_card(
        &mut self,
        entry_id: u64,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            let top = self
                .activity_card_tops
                .borrow()
                .get(&entry_id)
                .copied()
                .unwrap_or_else(|| window.mouse_position().y);
            if self
                .activity_popout
                .as_ref()
                .is_none_or(|popout| popout.entry_id != entry_id)
            {
                self.open_activity_popout(entry_id, top, window, cx);
            }
        }
        if let Some(popout) = &mut self.activity_popout
            && popout.entry_id == entry_id
        {
            popout.over_card = hovered;
        }
        self.settle_activity_popout(window, cx);
    }

    pub(crate) fn hover_activity_popout(
        &mut self,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(popout) = &mut self.activity_popout {
            popout.over_popout = hovered;
        }
        self.settle_activity_popout(window, cx);
    }

    fn open_activity_popout(
        &mut self,
        entry_id: u64,
        top: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self
            .activity_cards()
            .into_iter()
            .find(|card| card.entry_id == entry_id)
        else {
            return;
        };
        if let Some(application_id) = card.application_id {
            self.ensure_activity_colors(application_id, cx);
        }
        let name = self
            .store
            .member_name(self.selected_guild_id, card.user_id)
            .map(str::to_owned)
            .or_else(|| self.store.user(card.user_id).map(|user| user.name.clone()))
            .unwrap_or_default();
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(format!("Message @{name}")));
        let events = cx.subscribe_in(&input, window, move |app, input, event, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let text = input.read(cx).value().trim().to_owned();
                if !text.is_empty() {
                    app.send_activity_reply(entry_id, text, cx);
                    app.close_activity_popout(cx);
                }
            }
        });
        self.activity_popout = Some(ActivityPopout {
            entry_id,
            top,
            menu: false,
            input,
            over_card: true,
            over_popout: false,
            closing: None,
            _events: events,
        });
        cx.notify();
    }

    /// Keeps the popout while the pointer is on its card or on it, or while its reply box has
    /// focus; otherwise closes it after a short grace period so the pointer can cross the gap.
    fn settle_activity_popout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popout) = &mut self.activity_popout else {
            return;
        };
        let typing = popout.input.read(cx).focus_handle(cx).is_focused(window);
        if popout.over_card || popout.over_popout || popout.menu || typing {
            popout.closing = None;
            return;
        }
        if popout.closing.is_some() {
            return;
        }
        let entry_id = popout.entry_id;
        popout.closing =
            Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |app, cx| {
                    if app.activity_popout.as_ref().is_some_and(|popout| {
                        popout.entry_id == entry_id && popout.closing.is_some()
                    }) {
                        app.close_activity_popout(cx);
                    }
                })
                .ok();
            }));
    }

    pub(crate) fn close_activity_popout(&mut self, cx: &mut Context<Self>) {
        if self.activity_popout.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn toggle_activity_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(popout) = &mut self.activity_popout {
            popout.menu = !popout.menu;
            cx.notify();
        }
    }

    pub(crate) fn react_to_activity(
        &mut self,
        entry_id: u64,
        choice: &EmojiChoice,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = self.emoji_insert_text(choice) {
            self.send_activity_reply(entry_id, text, cx);
        }
        self.close_activity_popout(cx);
    }

    /// Activity replies in the open channel need the game icon and its gradient colours.
    pub(crate) fn load_activity_reply_assets(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self
            .channel()
            .into_iter()
            .flat_map(|channel| &channel.messages)
            .flat_map(|message| &message.components)
            .filter_map(|component| match component {
                Component::ActivityEntry(entry) => entry.application_id,
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        for &id in &ids {
            self.ensure_activity_colors(id, cx);
        }
        self.load_game_apps(ids, cx);
    }
}
