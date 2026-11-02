use super::profile::{
    ProfileCard,
    panel::{self, PANEL_INSET},
};
use super::*;
use crate::discord::ProfileKind;
use gpui_component::scroll::ScrollableElement as _;

const WIDTH: f32 = panel::CARD_WIDTH + PANEL_INSET * 2.;

impl DiscordApp {
    fn dm_recipient(&self) -> Option<u64> {
        let dm = self.store.private_channel(self.selected_channel_id)?;
        dm.recipient_ids.first().copied().filter(|_| !dm.group)
    }

    /// `None` when the selected page has no panel beside it.
    pub(crate) fn dm_panel_width(&self) -> Option<f32> {
        if !self.is_home() {
            return None;
        }
        let dm = self.store.private_channel(self.selected_channel_id)?;
        Some(if dm.group { member_list::WIDTH } else { WIDTH })
    }

    pub(super) fn load_dm_profile(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = self.dm_recipient() else {
            self.dm_profile = None;
            return;
        };
        if self
            .dm_profile
            .as_ref()
            .is_some_and(|card| card.user_id() == user_id)
        {
            return;
        }
        self.dm_profile = Some(ProfileCard::panel(user_id));
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        let request = Tokio::spawn(cx, async move {
            session
                // Only the modal response carries the mutual servers and friends.
                .profile(user_id, None, Some(channel_id), ProfileKind::Modal)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(profile)) => app.dm_profile_loaded(user_id, profile, cx),
                Ok(Err(error)) => log::warn!("Could not load the DM profile: {error}"),
                Err(error) => log::warn!("DM profile task failed: {error}"),
            })
            .ok();
        })
        .detach();
    }
}

pub(super) fn render(app: &DiscordApp, cx: &App) -> AnyElement {
    let Some(dm) = app.store.private_channel(app.selected_channel_id) else {
        return div().into_any_element();
    };
    if dm.group {
        return div()
            .id("dm-panel")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scrollbar()
            .border_l_1()
            .border_color(rgba(theme::border()))
            .child(group_members(app, dm, cx))
            .into_any_element();
    }
    div()
        .size_full()
        .p(rpx(PANEL_INSET))
        .children(
            app.dm_profile
                .as_ref()
                .map(|card| panel::render(app, card, cx)),
        )
        .into_any_element()
}

fn group_members(app: &DiscordApp, dm: &crate::store::PrivateChannel, cx: &App) -> AnyElement {
    let mut users: Vec<_> = dm
        .recipient_ids
        .iter()
        .copied()
        .chain([app.store.current_user_id])
        .filter_map(|id| app.store.user(id))
        .collect();
    users.sort_by_key(|user| user.name.to_lowercase());
    div()
        .w_full()
        .px_2()
        .pb_4()
        .child(member_list::group_row(0, "Members", users.len() as u32))
        .children(
            users
                .into_iter()
                .map(|user| member_list::member_row(app, 0, user, cx)),
        )
        .into_any_element()
}
