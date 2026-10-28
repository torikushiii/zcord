use gpui::{prelude::*, *};

use crate::{
    app::{
        DiscordApp,
        display_name::{self, Surface},
        message_body::label,
    },
    media::animated,
    media::images::{self, ImageSpec},
    store::{MemberList, MemberRow, NameStyle, Nameplate, User},
    theme::{self, WithoutTextSelection, rpx},
};

pub const WIDTH: f32 = 264.;
const ROW_HEIGHT: f32 = 44.;
const ROW_PADDING: f32 = 8.;
const AVATAR: f32 = 32.;
const AVATAR_GAP: f32 = 12.;
const NAMEPLATE_OPACITY: f32 = 0.6;
const NAMEPLATE_DIM: u32 = 0xB3;
const NAMEPLATE_FADE: f32 = 50.;
const NAMEPLATE_TEXT_MARGIN: f32 = 10.;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct Layout {
    groups: Vec<usize>,
}

impl Layout {
    pub fn sync(&mut self, state: &ListState, list: Option<&MemberList>) {
        let count = list.map_or(0, |list| list.rows.len());
        let old = state.item_count();
        if old == 0 && count > 0 {
            state.reset(count);
        } else if count > old {
            state.splice(old..old, count - old);
        } else if count < old {
            state.splice(count..old, 0);
        }
        if count != old {
            // Only the final row has bottom padding; keep the retained rows measured.
            if let Some(last) = count.min(old).checked_sub(1) {
                state.remeasure_items(last..last + 1);
            }
        }

        // Members and placeholders share a fixed height. Only becoming or ceasing to
        // be a group header changes an existing row's layout, not presence or SYNC data.
        if count == old {
            let mut same = true;
            let mut positions = self.groups.iter().copied();
            for (index, row) in list.into_iter().flat_map(|list| &list.rows).enumerate() {
                if matches!(row, Some(MemberRow::Group { .. })) && positions.next() != Some(index) {
                    same = false;
                    break;
                }
            }
            if same && positions.next().is_none() {
                return;
            }
        }

        let groups: Vec<usize> = list
            .into_iter()
            .flat_map(|list| &list.rows)
            .enumerate()
            .filter_map(|(index, row)| {
                matches!(row, Some(MemberRow::Group { .. })).then_some(index)
            })
            .collect();
        for &index in &self.groups {
            if groups.binary_search(&index).is_err() && index < count {
                state.remeasure_items(index..index + 1);
            }
        }
        for &index in &groups {
            if self.groups.binary_search(&index).is_err() && index < count {
                state.remeasure_items(index..index + 1);
            }
        }
        self.groups = groups;
    }
}

pub fn render(app: &DiscordApp) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    div()
        .id("members")
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(rgba(theme::BORDER))
        .child(
            list(app.member_rows.clone(), move |index, _, cx| {
                let Some(app) = entity.upgrade() else {
                    return div().into_any_element();
                };
                render_row(app.read(cx), index, cx)
            })
            .size_full(),
        )
}

fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let last = app
        .store
        .member_list(app.selected_guild_id)
        .is_some_and(|list| index + 1 == list.rows.len());
    div()
        .w_full()
        .px_2()
        .when(last, |this| this.pb_4())
        .child(row_content(app, index, cx))
        .into_any_element()
}

fn row_content(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let guild_id = app.selected_guild_id;
    let row = app
        .store
        .member_list(guild_id)
        .and_then(|list| list.rows.get(index))
        .and_then(Option::as_ref);
    match row {
        Some(MemberRow::Group { name, count }) => group_row(index, name, *count).into_any_element(),
        Some(MemberRow::Member { user_id }) => match app.store.user(*user_id) {
            Some(user) => member_row(app, guild_id, user, cx).into_any_element(),
            None => placeholder_row(),
        },
        None => placeholder_row(),
    }
}

fn group_row(index: usize, name: &str, count: u32) -> Div {
    div()
        .pt(rpx(if index == 0 { 16. } else { 24. }))
        .pb(rpx(4.))
        .px_2()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::MUTED))
        .truncate()
        .child(label(
            ("member-group", index as u64),
            &format!("{name} — {count}"),
        ))
}

fn placeholder_row() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(rpx(44.))
        .px_2()
        .child(
            div()
                .size(rpx(32.))
                .flex_none()
                .rounded_full()
                .bg(rgba(theme::SELECTED)),
        )
        .child(
            div()
                .h(rpx(12.))
                .w(rpx(110.))
                .rounded(rpx(6.))
                .bg(rgba(theme::SELECTED)),
        )
        .into_any_element()
}

fn member_row(app: &DiscordApp, guild_id: u64, user: &User, cx: &App) -> impl IntoElement + use<> {
    let online = user.status.is_online();
    let name = app
        .store
        .member_name(guild_id, user.id)
        .unwrap_or(&user.name)
        .to_owned();
    let role_color = app.store.member_color(guild_id, user.id);
    let hovered = app.hovered_member == Some(user.id);
    let nameplate = app
        .store
        .nameplate(Some(guild_id), user.id)
        .and_then(|nameplate| nameplate_layer(nameplate, user.id, ROW_HEIGHT, hovered, cx));
    let plated = nameplate.is_some();
    let row_background = if hovered {
        theme::composite(theme::HOVER, theme::BG)
    } else {
        theme::BG
    };
    let dim = rgba(row_background << 8 | NAMEPLATE_DIM);
    let entity = app.entity.clone();
    let id = user.id;
    div()
        .id(("member", user.id))
        .relative()
        .flex()
        .items_center()
        .gap(rpx(AVATAR_GAP))
        .w_full()
        .h(rpx(ROW_HEIGHT))
        .px(rpx(ROW_PADDING))
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .when(!online, |this| this.opacity(0.35))
        .hover(|style| style.bg(rgba(theme::HOVER)).opacity(1.))
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    let value = hovered.then_some(id);
                    if *hovered || app.hovered_member == Some(id) {
                        app.hovered_member = value;
                        cx.notify();
                    }
                })
                .ok();
        })
        .without_text_selection()
        .on_click({
            let entity = app.entity.clone();
            move |event, window, cx| {
                cx.stop_propagation();
                let list_left = window.viewport_size().width
                    - window.rem_size() * (WIDTH / theme::BASE_FONT_SIZE);
                let position = point(list_left - px(8.), event.position().y - px(40.));
                entity
                    .update(cx, |app, cx| {
                        app.open_profile(id, position, Anchor::TopRight, cx)
                    })
                    .ok();
            }
        })
        .children(nameplate)
        .when(plated, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(rpx(ROW_PADDING + AVATAR + AVATAR_GAP))
                    .bg(dim),
            )
        })
        .child(theme::member_avatar(
            &app.store,
            guild_id,
            user.id,
            AVATAR,
            online.then_some((user.status, theme::BG)),
            hovered,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .max_w_full()
                        .min_w_0()
                        .when(plated, |this| this.children(nameplate_mask(dim)))
                        .child(name_line(
                            user,
                            &name,
                            role_color,
                            app.store.member_name_style(guild_id, user.id),
                            cx,
                        ))
                        .children(activity_line(user, online)),
                ),
        )
}

fn name_line(
    user: &User,
    name: &str,
    role_color: Option<u32>,
    name_style: Option<&NameStyle>,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(role_color.unwrap_or(theme::TEXT)))
                .child(display_name::render(
                    ("member-name", user.id),
                    name,
                    name_style,
                    Surface::FontOnly,
                    cx,
                )),
        )
        .when(user.bot, |this| this.child(theme::app_tag()))
        .children(user.tag.as_ref().map(|tag| theme::guild_tag(tag, cx)))
}

fn activity_line(user: &User, online: bool) -> Option<impl IntoElement> {
    let activity = user.activity.as_ref().filter(|_| online)?;
    Some(
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .min_w_0()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::MUTED))
            .children(theme::activity_icon(user).map(|icon| theme::icon(icon, 14., theme::GREEN)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(label(("member-activity", user.id), activity)),
            ),
    )
}

fn palette_color(palette: &str) -> Option<u32> {
    Some(match palette {
        "crimson" => 0x900007,
        "berry" => 0x893A99,
        "sky" => 0x0080B7,
        "teal" => 0x086460,
        "forest" => 0x2D5401,
        "bubble_gum" => 0xDC3E97,
        "violet" => 0x730BC8,
        "cobalt" => 0x0131C2,
        "clover" => 0x047B20,
        "lemon" => 0xF6CD12,
        "white" => 0xFFFFFF,
        "black" => 0x000000,
        _ => return None,
    })
}

pub(crate) fn nameplate_layer(
    nameplate: &Nameplate,
    user_id: u64,
    height: f32,
    hovered: bool,
    cx: &App,
) -> Option<AnyElement> {
    let image = nameplate_image(&nameplate.asset, user_id, height, hovered, cx)?;
    let wash = nameplate
        .palette
        .as_deref()
        .and_then(palette_color)
        .map(|color| palette_wash(color, hovered));
    Some(
        div()
            .absolute()
            .inset_0()
            .children(wash)
            .child(image)
            .into_any_element(),
    )
}

fn palette_wash(color: u32, hovered: bool) -> impl IntoElement {
    let tint = |alpha: u32| rgba(color << 8 | alpha);
    let flat = tint(0x14);
    let end = tint(if hovered { 0x4D } else { 0x33 });
    div()
        .absolute()
        .inset_0()
        .flex()
        .child(div().h_full().w(relative(0.2)).bg(linear_gradient(
            90.,
            linear_color_stop(tint(0), 0.),
            linear_color_stop(flat, 1.),
        )))
        .child(div().h_full().w(relative(0.3)).bg(flat))
        .child(div().h_full().flex_1().bg(linear_gradient(
            90.,
            linear_color_stop(flat, 0.),
            linear_color_stop(end, 1.),
        )))
}

fn nameplate_mask(dim: Rgba) -> [AnyElement; 2] {
    let full_row = rpx(ROW_HEIGHT);
    [
        div()
            .absolute()
            .top(-full_row)
            .bottom(-full_row)
            .left_0()
            .right(rpx(-NAMEPLATE_TEXT_MARGIN))
            .bg(dim)
            .into_any_element(),
        div()
            .absolute()
            .top(-full_row)
            .bottom(-full_row)
            .right(rpx(-NAMEPLATE_TEXT_MARGIN - NAMEPLATE_FADE))
            .w(rpx(NAMEPLATE_FADE))
            .bg(linear_gradient(
                90.,
                linear_color_stop(dim, 0.),
                linear_color_stop(rgba(0), 1.),
            ))
            .into_any_element(),
    ]
}

fn nameplate_image(
    asset: &str,
    user_id: u64,
    height: f32,
    animate: bool,
    cx: &App,
) -> Option<AnyElement> {
    let spec = |animated| ImageSpec::Nameplate {
        asset: asset.to_owned(),
        animated,
    };
    let pixels = theme::image_px(WIDTH);
    let image = match animate
        .then(|| animated::resolve(&spec(true), pixels, cx))
        .flatten()
    {
        Some(image) => animated::image(image)
            .h(rpx(height))
            .id(("nameplate", user_id))
            .into_any_element(),
        None => img(images::resolve(&spec(false), pixels, cx)?)
            .h(rpx(height))
            .into_any_element(),
    };
    Some(
        div()
            .absolute()
            .top_0()
            .right_0()
            .opacity(NAMEPLATE_OPACITY)
            .child(image)
            .into_any_element(),
    )
}
