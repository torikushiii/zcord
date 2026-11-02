use super::message_body::{BodyColors, BodyContext, render_body};
use super::*;
use crate::discord::{ProfileKind, model};
use crate::media::images::{self, ImageSpec};
use gpui_component::scroll::ScrollableElement as _;

const WIDTH: f32 = 340.;
const BANNER_HEIGHT: f32 = 120.;
const AVATAR: f32 = 80.;
const AVATAR_RING: f32 = 6.;
const AVATAR_LEFT: f32 = 16.;

pub(crate) struct DmProfile {
    user_id: u64,
    profile: Option<Arc<model::Profile>>,
}

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
            .is_some_and(|loaded| loaded.user_id == user_id)
        {
            return;
        }
        self.dm_profile = Some(DmProfile {
            user_id,
            profile: None,
        });
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        let request = Tokio::spawn(cx, async move {
            session
                .profile(user_id, None, Some(channel_id), ProfileKind::Popout)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(profile)) => {
                    if let Some(loaded) = app
                        .dm_profile
                        .as_mut()
                        .filter(|loaded| loaded.user_id == user_id)
                    {
                        loaded.profile = Some(profile);
                        cx.notify();
                    }
                }
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
    let content = if dm.group {
        group_members(app, dm, cx)
    } else {
        profile_panel(app, cx)
    };
    div()
        .id("dm-panel")
        .size_full()
        .flex()
        .flex_col()
        .overflow_y_scrollbar()
        .border_l_1()
        .border_color(rgba(theme::BORDER))
        .child(content)
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

fn profile_panel(app: &DiscordApp, cx: &App) -> AnyElement {
    let Some(user_id) = app.dm_recipient() else {
        return div().into_any_element();
    };
    let user = app.store.user(user_id);
    let profile = app
        .dm_profile
        .as_ref()
        .filter(|loaded| loaded.user_id == user_id)
        .and_then(|loaded| loaded.profile.as_deref());
    let entity = app.entity.clone();
    let avatar = div()
        .id("dm-panel-avatar")
        .absolute()
        .left(rpx(AVATAR_LEFT))
        .top(rpx(BANNER_HEIGHT - AVATAR / 2. - AVATAR_RING))
        .p(rpx(AVATAR_RING))
        .rounded_full()
        .bg(rgb(theme::BG))
        .cursor_pointer()
        .on_click(move |event, window, cx| {
            let left =
                window.viewport_size().width - window.rem_size() * (WIDTH / theme::BASE_FONT_SIZE);
            let position = point(left - px(8.), event.position().y);
            entity
                .update(cx, |app, cx| {
                    app.open_profile(user_id, position, Anchor::TopRight, cx)
                })
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            AVATAR,
            user.map(|user| (user.status, theme::BG)),
            true,
            cx,
        ));
    let bio = profile
        .and_then(|profile| profile.user_profile.as_ref()?.bio.clone())
        .or_else(|| profile.and_then(|profile| profile.user.bio.clone()))
        .filter(|bio| !bio.trim().is_empty());
    let created = (user_id >> 22) as i64 + DISCORD_EPOCH_MS as i64;
    let created = chrono::DateTime::from_timestamp_millis(created)
        .map(|date| date.date_naive().format("%b %-d, %Y").to_string());
    let about = div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .p(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .children(bio.map(|bio| {
            section(
                "About Me",
                bio_body(app, user_id, &bio, cx).into_any_element(),
            )
        }))
        .children(created.map(|created| section("Member Since", created.into_any_element())));
    let mutual_servers = profile
        .and_then(|profile| profile.mutual_guilds.as_ref())
        .map(Vec::len)
        .filter(|count| *count > 0);
    let mutual_friends = profile
        .and_then(|profile| profile.mutual_friends_count)
        .filter(|count| *count > 0);
    div()
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .flex_none()
        .pb(rpx(16.))
        .child(banner(profile, user_id, cx))
        .child(avatar)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .pt(rpx(AVATAR / 2. + AVATAR_RING + 8.))
                .px(rpx(16.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(20.))
                                .line_height(rpx(24.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(user.map_or_else(String::new, |user| user.name.clone())),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(theme::TEXT))
                                .child(user.map_or_else(String::new, |user| user.username.clone())),
                        ),
                )
                .child(about)
                .children(mutual_servers.map(|count| mutual_row("Mutual Servers", count)))
                .children(mutual_friends.map(|count| mutual_row("Mutual Friends", count as usize))),
        )
        .into_any_element()
}

fn banner(profile: Option<&model::Profile>, user_id: u64, cx: &App) -> Div {
    let color = profile
        .and_then(|profile| profile.user.banner_color.or(profile.user.accent_color))
        .or_else(|| profile?.user_profile.as_ref()?.accent_color)
        .unwrap_or(theme::SURFACE_HIGHEST);
    let image = profile
        .and_then(|profile| profile.user.banner.clone())
        .and_then(|hash| {
            images::resolve(
                &ImageSpec::Banner { user_id, hash },
                theme::image_px(WIDTH),
                cx,
            )
        })
        .map(|path| img(path).size_full().object_fit(ObjectFit::Cover));
    div()
        .flex_none()
        .w_full()
        .h(rpx(BANNER_HEIGHT))
        .bg(rgb(color))
        .children(image)
}

fn section(title: &'static str, body: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::MUTED))
                .child(title),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::STRONG))
                .child(body),
        )
}

fn bio_body(app: &DiscordApp, user_id: u64, bio: &str, cx: &App) -> impl IntoElement {
    let parsed = app.parsed(user_id, bio);
    let context = BodyContext {
        store: &app.store,
        guild_id: 0,
        message_id: user_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: BodyColors {
            strong: theme::STRONG,
            muted: theme::MUTED,
            faint: theme::FAINT,
            link: None,
        },
    };
    render_body(&parsed.blocks, false, false, &context, cx)
}

fn mutual_row(title: &str, count: usize) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(rpx(40.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::STRONG))
        .child(title.to_owned())
        .child(div().text_color(rgb(theme::MUTED)).child(count.to_string()))
}
