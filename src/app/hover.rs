use super::*;

impl DiscordApp {
    pub(crate) fn upload_tip(
        &self,
        button: attachments::UploadButton,
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
                            app.hovered_upload_button = Some(button);
                        } else if app.hovered_upload_button == Some(button) {
                            app.hovered_upload_button = None;
                        }
                        cx.notify();
                    })
                    .ok();
            })
            .when(self.hovered_upload_button == Some(button), |this| {
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
