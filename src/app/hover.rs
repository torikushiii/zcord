use super::*;

impl DiscordApp {
    pub(crate) fn upload_tip(
        &self,
        button: messages::attachments::UploadButton,
        label: &'static str,
        element: Stateful<Div>,
    ) -> Stateful<Div> {
        let entity = self.entity.clone();
        element
            .relative()
            .on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        if *hovered {
                            app.uploads.hovered_button = Some(button);
                        } else if app.uploads.hovered_button == Some(button) {
                            app.uploads.hovered_button = None;
                        }
                        cx.notify();
                    })
                    .ok();
            })
            .when(self.uploads.hovered_button == Some(button), |this| {
                this.child(theme::tooltip_above(label))
            })
    }

    pub(crate) fn set_add_reaction_hover(
        &mut self,
        message_id: u64,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(message_id)
        } else if self.hover.add_reaction == Some(message_id) {
            None
        } else {
            return;
        };
        if self.hover.add_reaction != next {
            self.hover.add_reaction = next;
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
            &mut self.hover.actions
        } else {
            &mut self.hover.message
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
