use super::super::profile::{
    ProfileCard,
    panel::{self, PANEL_INSET},
};
use super::super::*;
use crate::discord::ProfileKind;
use gpui_component::scroll::ScrollableElement as _;

const WIDTH: f32 = panel::CARD_WIDTH + PANEL_INSET * 2.;

impl DiscordApp {
    fn dm_recipient(&self) -> Option<u64> {
        let dm = self.store.private_channel(self.selected_channel_id)?;
        dm.recipient_ids.first().copied().filter(|_| !dm.group)
    }

    pub(crate) fn dm_panel_width(&self) -> Option<f32> {
        if !self.is_home() {
            return None;
        }
        let dm = self.store.private_channel(self.selected_channel_id)?;
        Some(if dm.group {
            members::member_list::WIDTH
        } else {
            WIDTH
        })
    }

    pub(crate) fn load_dm_profile(&mut self, cx: &mut Context<Self>) {
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

pub(crate) fn render(app: &DiscordApp, cx: &App) -> AnyElement {
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
    let own_group = dm.owner_id == Some(app.store.current_user_id);
    let channel_id = dm.channel.id;
    div()
        .w_full()
        .px_2()
        .pb_4()
        .child(members::member_list::group_row(
            0,
            "Members",
            users.len() as u32,
        ))
        .children(users.into_iter().map(|user| {
            let user_id = user.id;
            let removable = own_group && user_id != app.store.current_user_id;
            let remove = app.entity.clone();
            div()
                .relative()
                .group("group-member")
                .child(members::member_list::member_row(app, 0, user, cx))
                .when(removable, |this| {
                    this.child(
                        div()
                            .id(("group-member-remove", user_id))
                            .absolute()
                            .right(rpx(8.))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .opacity(0.)
                            .group_hover("group-member", |style| style.opacity(1.))
                            .cursor_pointer()
                            .child(theme::icon("x", 16., theme::muted()))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                remove
                                    .update(cx, |app, cx| {
                                        app.remove_group_recipient(channel_id, user_id, cx)
                                    })
                                    .ok();
                            }),
                    )
                })
        }))
        .into_any_element()
}
