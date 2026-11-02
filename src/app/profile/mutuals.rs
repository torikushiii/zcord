use super::*;

pub(super) fn mutuals_line(
    app: &DiscordApp,
    profile: &model::Profile,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let friend_count = profile
        .mutual_friends_count
        .filter(|count| *count > 0)
        .map(|count| count as usize)
        .or_else(|| {
            profile
                .mutual_friends
                .as_ref()
                .map(Vec::len)
                .filter(|len| *len > 0)
        });
    let friend_list = profile.mutual_friends.as_deref().unwrap_or_default();
    let friends = friend_count.map(|count| {
        let avatars = friend_list.iter().take(3).map(|friend| {
            theme::user_avatar(
                app.store.user(friend.id),
                friend.id,
                MUTUAL_ICON,
                None,
                false,
                cx,
            )
            .into_any_element()
        });
        // Only a returned list has a Friends tab to open.
        let tab = (!friend_list.is_empty()).then_some(Tab::Friends);
        mutual_part(
            app,
            "profile-mutual-friends",
            avatars,
            plural(count, "Mutual Friend"),
            tab,
        )
    });
    let guilds = profile.mutual_guilds.as_deref().unwrap_or_default();
    let servers = (!guilds.is_empty()).then(|| {
        let icons = guilds
            .iter()
            .filter_map(|mutual| app.store.guild_by_id(mutual.id))
            .take(if friend_count.is_some() { 0 } else { 3 })
            .map(|guild| guild_icon(guild, MUTUAL_ICON, cx));
        mutual_part(
            app,
            "profile-mutual-servers",
            icons,
            plural(guilds.len(), "Mutual Server"),
            Some(Tab::Servers),
        )
    });
    let parts: Vec<_> = friends.into_iter().chain(servers).collect();
    mutuals_row(parts, palette).map(|row| row.into_any_element())
}

pub(super) fn mutuals_row(parts: Vec<AnyElement>, palette: ProfilePalette) -> Option<Div> {
    if parts.is_empty() {
        return None;
    }
    let compact = parts.len() > 1;
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(rpx(6.))
            .gap_y(rpx(2.))
            .text_size(rpx(if compact { 12. } else { 14. }))
            .line_height(rpx(if compact { 16. } else { 18. }))
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(palette.muted))
            .children(parts.into_iter().enumerate().flat_map(|(index, part)| {
                let spacer = (index > 0).then(|| {
                    div()
                        .size(rpx(4.))
                        .rounded_full()
                        .bg(rgb(palette.muted))
                        .into_any_element()
                });
                spacer.into_iter().chain(std::iter::once(part))
            })),
    )
}

/// One "N Mutual …" entry: up to three overlapping icons, then the count, opening `tab` on click.
fn mutual_part(
    app: &DiscordApp,
    id: &'static str,
    icons: impl Iterator<Item = AnyElement>,
    text: String,
    tab: Option<Tab>,
) -> AnyElement {
    let entity = app.entity.clone();
    mutual_entry(id, icons, text)
        .when_some(tab, |this, tab| {
            this.cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.open_profile_tab(tab, cx))
                        .ok();
                })
        })
        .into_any_element()
}

pub(super) fn mutual_entry(
    id: &'static str,
    icons: impl Iterator<Item = AnyElement>,
    text: String,
) -> Stateful<Div> {
    // A negative margin collapses the stack's width to zero in gpui's layout, so the icons are
    // placed absolutely inside an explicitly sized box instead.
    let icons: Vec<_> = icons
        .enumerate()
        .map(|(index, icon)| {
            div()
                .absolute()
                .top_0()
                .left(rpx(index as f32 * MUTUAL_STEP))
                .size(rpx(MUTUAL_ICON))
                .rounded_full()
                .child(icon)
        })
        .collect();
    let width = MUTUAL_ICON + icons.len().saturating_sub(1) as f32 * MUTUAL_STEP;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(4.))
        .py(rpx(4.))
        .when(!icons.is_empty(), |this| {
            this.child(
                div()
                    .debug_selector(|| "mutual-icons".into())
                    .relative()
                    .flex_none()
                    .w(rpx(width))
                    .h(rpx(MUTUAL_ICON))
                    .children(icons),
            )
        })
        .child(div().debug_selector(|| "mutual-text".into()).child(text))
}

fn guild_icon(guild: &crate::store::Guild, size: f32, cx: &App) -> AnyElement {
    let icon = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx));
    match icon {
        Some(path) => img(path).size(rpx(size)).rounded_full().into_any_element(),
        None => div()
            .size(rpx(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rgb(guild.icon_color))
            .text_size(rpx(size * 0.4))
            .text_color(gpui::black())
            .child(guild.initials.clone())
            .into_any_element(),
    }
}
