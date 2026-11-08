use super::super::*;

impl DiscordApp {
    pub(crate) fn invite(&self, code: &str) -> InviteState {
        self.invites
            .get(code)
            .cloned()
            .unwrap_or(InviteState::Loading)
    }

    pub(crate) fn load_invite(&mut self, code: &str, cx: &mut Context<Self>) {
        if self.invites.contains_key(code) {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.invites.insert(code.to_owned(), InviteState::Loading);
        let code_owned = code.to_owned();
        let task_code = code_owned.clone();
        let request = Tokio::spawn(cx, async move {
            let invite = match session.invite(&task_code).await {
                Ok(invite) => invite,
                Err(error) => return Err(error),
            };
            let Some(invite) = invite else {
                return Ok(None);
            };
            let game_ids = invite
                .profile
                .as_ref()
                .map(|profile| profile.game_application_ids.as_slice())
                .unwrap_or_default();
            let applications = session
                .public_applications(game_ids)
                .await
                .unwrap_or_default();
            Ok(Invite::from_model(&invite, &applications))
        });
        cx.spawn(async move |this, cx| {
            let state = match request.await {
                Ok(Ok(Some(invite))) => InviteState::Ready(Arc::new(invite)),
                Ok(Ok(None)) => InviteState::Invalid,
                Ok(Err(error)) => {
                    log::warn!("Could not load invite {code_owned}: {error}");
                    InviteState::Invalid
                }
                Err(error) => {
                    log::warn!("Invite task failed for {code_owned}: {error}");
                    InviteState::Invalid
                }
            };
            this.update(cx, |app, cx| {
                app.invites.insert(code_owned, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn prefetch_invites(&mut self, content: &str, cx: &mut Context<Self>) {
        for code in discord::model::invite_codes(content) {
            self.load_invite(&code, cx);
        }
    }
}
