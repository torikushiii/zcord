use super::*;

impl DiscordApp {
    pub(crate) fn set_add_reaction_hover(
        &mut self,
        message_id: u64,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(message_id)
        } else if self.hovered_add_reaction == Some(message_id) {
            None
        } else {
            return;
        };
        if self.hovered_add_reaction != next {
            self.hovered_add_reaction = next;
            cx.notify();
        }
    }

    pub(crate) fn set_sticker_hover(
        &mut self,
        message_id: u64,
        sticker_id: u64,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (message_id, sticker_id);
        let next = if hovered {
            Some(key)
        } else if self.hovered_sticker == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_sticker != next {
            self.hovered_sticker = next;
            cx.notify();
        }
    }

    pub(crate) fn set_message_hover(
        &mut self,
        message_id: u64,
        hovered: bool,
        actions: bool,
        cx: &mut Context<Self>,
    ) {
        let slot = if actions {
            &mut self.hovered_actions
        } else {
            &mut self.hovered_message
        };
        let next = if hovered {
            Some(message_id)
        } else if *slot == Some(message_id) {
            None
        } else {
            return;
        };
        if *slot != next {
            *slot = next;
            cx.notify();
        }
    }
}
