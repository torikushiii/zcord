use super::super::calls::stream_view::{
    self, DOCK_HEIGHT, StageControls, control, controls_fade, member_name, name_chip,
};
use super::super::profile::avatar_colors;
use super::super::*;
use crate::media::images;
use crate::store::VoiceMember;
use std::collections::HashMap;

const TILE_GAP: f32 = 8.;
const TILE_RATIO: f32 = 16. / 9.;
const STAGE_INSET: f32 = 8.;
const AVATAR_SIZE: f32 = 80.;
const AVATAR_TILE_SHARE: f32 = 0.4;
const STREAM_TILE: u32 = 0x1E1F22;

pub(crate) struct CallView {
    pub(super) controls: StageControls,
    /// `None` while an avatar's color is being read, or when it can't be.
    tints: HashMap<u64, Option<u32>>,
}

enum Tile {
    Member(VoiceMember),
    Stream(u64),
}

impl DiscordApp {
    pub(crate) fn open_voice_channel(
        &mut self,
        guild_id: u64,
        channel_id: u64,
        cx: &mut Context<Self>,
    ) {
        // The first click only joins; clicking the channel again once in it opens the call.
        if self
            .voice
            .joined_call()
            .is_some_and(|(_, joined)| joined == channel_id)
        {
            self.open_call_view(cx);
        } else {
            self.join_voice(guild_id, channel_id, cx);
        }
    }

    pub(super) fn open_call_view(&mut self, cx: &mut Context<Self>) {
        if self.voice.call.is_none() {
            return;
        }
        self.voice.call_view.get_or_insert_with(|| CallView {
            controls: StageControls::new(),
            tints: HashMap::new(),
        });
        self.refresh_call_tints(cx);
        cx.notify();
    }

    pub(crate) fn dismiss_call_view(&mut self) {
        self.voice.call_view = None;
        self.voice_lobby = None;
        if self.stream.view.is_none() {
            self.stream.stage_chat = None;
        }
    }

    pub(crate) fn highlighted_channel(&self) -> u64 {
        self.voice
            .call_view
            .as_ref()
            .and(self.voice.joined_call())
            .or(self.voice_lobby)
            .map_or(self.selected_channel_id, |(_, channel_id)| channel_id)
    }

    pub(crate) fn open_voice_chat(
        &mut self,
        guild_id: u64,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Switching stages keeps the way back to the text channel the chat first opened over.
        let back = self.stream.stage_chat.take();
        if self.voice.joined_call() == Some((guild_id, channel_id)) {
            self.voice_lobby = None;
            self.open_call_view(cx);
        } else {
            // A stream stage outranks a lobby, so it has to go for this channel to show.
            self.close_stream(Some(window), cx);
            self.dismiss_call_view();
            self.voice_lobby = Some((guild_id, channel_id));
        }
        self.open_stream_chat(window, cx);
        if back.is_some() {
            self.stream.stage_chat = back;
        }
        cx.notify();
    }

    pub(crate) fn tick_call_view(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self
            .voice
            .call_view
            .as_ref()
            .filter(|_| self.stream.view.is_none())
        else {
            return;
        };
        if view.controls.idle() {
            self.hide_stage_controls(cx);
        }
        self.refresh_call_tints(cx);
    }

    /// Avatars load on their own schedule, so this runs on the call's clock until each is read.
    fn refresh_call_tints(&mut self, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.voice.joined_call() else {
            return;
        };
        for member in self.store.voice_members(guild_id, channel_id) {
            let user_id = member.user_id;
            let spec = self
                .store
                .member_avatar_spec(guild_id, user_id)
                .or_else(|| self.store.user(user_id).map(|user| user.avatar_spec()));
            let Some(view) = self
                .voice
                .call_view
                .as_mut()
                .filter(|view| !view.tints.contains_key(&user_id))
            else {
                continue;
            };
            let Some(path) =
                spec.and_then(|spec| images::resolve(&spec, theme::image_px(AVATAR_SIZE), cx))
            else {
                continue;
            };
            view.tints.insert(user_id, None);
            cx.spawn(async move |this, cx| {
                let colors = cx
                    .background_spawn(async move { avatar_colors::load(&path) })
                    .await;
                this.update(cx, |app, cx| {
                    if let Some(view) = &mut app.voice.call_view {
                        view.tints
                            .insert(user_id, colors.map(|[primary, _]| primary));
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                })
                .ok();
            })
            .detach();
        }
    }
}

/// How many columns give the largest 16:9 tiles inside `width` by `height`, and their width.
fn grid(count: usize, width: f32, height: f32) -> (usize, f32) {
    (1..=count.max(1))
        .map(|columns| {
            let rows = count.div_ceil(columns).max(1) as f32;
            let gaps = |lines: f32| TILE_GAP * (lines - 1.);
            let wide = (width - gaps(columns as f32)) / columns as f32;
            let tall = (height - gaps(rows)) / rows;
            (columns, wide.min(tall * TILE_RATIO).max(0.))
        })
        .fold(
            (1, 0.),
            |best, next| if next.1 > best.1 { next } else { best },
        )
}

fn tile(width: f32) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(rpx(width))
        .h(rpx(width / TILE_RATIO))
        .rounded(rpx(8.))
        .overflow_hidden()
}

fn member_tile(
    app: &DiscordApp,
    view: &CallView,
    guild_id: u64,
    member: &VoiceMember,
    width: f32,
    cx: &App,
) -> Div {
    let user_id = member.user_id;
    let color = view
        .tints
        .get(&user_id)
        .copied()
        .flatten()
        .unwrap_or(theme::surface_highest());
    let avatar = AVATAR_SIZE.min(width / TILE_RATIO * AVATAR_TILE_SHARE);
    tile(width)
        .bg(rgb(color))
        .when(app.voice.speaking.contains(&user_id), |this| {
            this.border_2().border_color(rgb(theme::GREEN))
        })
        .child(theme::undecorated_member_avatar(
            &app.store, guild_id, user_id, avatar, cx,
        ))
        .child(name_chip(
            (member.mute || member.self_mute).then_some("mic-off"),
            member_name(app, guild_id, user_id),
        ))
}

fn stream_tile(
    app: &DiscordApp,
    (guild_id, channel_id): (u64, u64),
    user_id: u64,
    width: f32,
) -> Div {
    let watch = app.entity.clone();
    tile(width)
        .bg(rgb(STREAM_TILE))
        .child(
            div()
                .id(("call-watch-stream", user_id))
                .flex()
                .items_center()
                .h(rpx(36.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .cursor_pointer()
                .bg(rgba(0x00000099))
                .hover(|style| style.opacity(0.85))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white())
                .on_click(move |_, window, cx| {
                    watch
                        .update(cx, |app, cx| {
                            app.watch_or_join((guild_id, channel_id, user_id), window, cx)
                        })
                        .ok();
                })
                .child("Watch Stream"),
        )
        .child(
            div()
                .absolute()
                .top(rpx(8.))
                .right(rpx(8.))
                .child(calls::voice::live_badge()),
        )
        .child(name_chip(
            Some("monitor-filled"),
            member_name(app, guild_id, user_id),
        ))
}

pub(crate) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    if app.stream.view.is_some() {
        return None;
    }
    let view = app.voice.call_view.as_ref()?;
    let (guild_id, channel_id) = app.voice.joined_call()?;
    let chat_open = app.stream_chat_open();
    let channel = app
        .store
        .channel_name(channel_id)
        .unwrap_or_default()
        .to_owned();
    let mut tiles = Vec::new();
    for member in app.store.voice_members(guild_id, channel_id) {
        if member.streaming && member.user_id != app.store.current_user_id {
            tiles.push(Tile::Stream(member.user_id));
        }
        tiles.push(Tile::Member(member));
    }
    let unit = rpx(1.).to_pixels(window.rem_size());
    let viewport = window.viewport_size();
    let width = viewport.width / unit
        - guilds::guild_rail::WIDTH
        - guilds::channel_list::WIDTH
        - if chat_open {
            stream_view::CHAT_WIDTH + stream_view::CHAT_GAP
        } else {
            0.
        }
        - STAGE_INSET * 2.;
    let height = viewport.height / unit - titlebar::HEIGHT - chat::HEADER_HEIGHT - DOCK_HEIGHT;
    let (columns, tile_width) = grid(tiles.len(), width, height);
    let rows = tiles.chunks(columns).map(|row| {
        div()
            .flex()
            .flex_none()
            .justify_center()
            .gap(rpx(TILE_GAP))
            .children(row.iter().map(|tile| match tile {
                Tile::Member(member) => member_tile(app, view, guild_id, member, tile_width, cx),
                Tile::Stream(user_id) => {
                    stream_tile(app, (guild_id, channel_id), *user_id, tile_width)
                }
            }))
    });
    let connecting = tiles.is_empty().then(|| {
        div()
            .text_size(rpx(15.))
            .text_color(rgb(theme::muted()))
            .child("Connecting\u{2026}")
    });
    let grid = div()
        .absolute()
        .top(rpx(chat::HEADER_HEIGHT))
        .bottom(rpx(DOCK_HEIGHT))
        .left(rpx(STAGE_INSET))
        .right(rpx(STAGE_INSET))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(TILE_GAP))
        .children(connecting)
        .children(rows);
    let header = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(chat::HEADER_HEIGHT))
        .px(rpx(16.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgba(0x000000B3), 0.),
            linear_color_stop(rgba(0x00000000), 1.),
        ))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white())
        .child(theme::icon("volume-2", 20., 0xFFFFFF))
        .child(div().flex_1().min_w_0().truncate().child(channel.clone()))
        .when(!chat_open, |this| {
            this.child(stream_view::show_chat_button(app))
        });
    let controls = div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .h(rpx(DOCK_HEIGHT))
        .child(stream_view::voice_controls(app, false))
        .child(stream_view::share_control(app, false))
        .child(stream_view::disconnect_control(app))
        .child(
            div().absolute().right(rpx(16.)).child(
                control(
                    app,
                    "stage-fullscreen",
                    "Full Screen",
                    "video-fullscreen",
                    0xFFFFFF,
                    0,
                )
                .on_click(|_, window, _| window.toggle_fullscreen()),
            ),
        );
    let area = stream_view::stage_area(app, titlebar::HEIGHT, DOCK_HEIGHT, chat_open)
        .child(grid)
        .child(controls_fade(header, "stage-header", &view.controls, cx))
        .child(controls_fade(
            controls,
            "stage-controls",
            &view.controls,
            cx,
        ));
    Some(stream_view::stage_frame(app, area, channel, chat_open))
}

fn in_voice(names: &[String]) -> String {
    match names {
        [] => "No one is currently in voice".to_owned(),
        [a] => format!("{a} is currently in voice"),
        [a, b] => format!("{a} and {b} are currently in voice"),
        [a, b, rest @ ..] => format!(
            "{a}, {b}, and {} {} are currently in voice",
            rest.len(),
            if rest.len() == 1 { "other" } else { "others" }
        ),
    }
}

const LOBBY_BANDS: [(f32, u32, u32); 3] = [
    (0.27, 0x0F0F17, 0x16163D),
    (0.33, 0x16163D, 0x2A2D80),
    (0.40, 0x2A2D80, 0x7A88E4),
];

pub(crate) fn render_lobby(app: &DiscordApp) -> Option<AnyElement> {
    if app.stream.view.is_some() || app.voice.call_view.is_some() {
        return None;
    }
    let (guild_id, channel_id) = app.voice_lobby?;
    let chat_open = app.stream_chat_open();
    let channel = app
        .store
        .channel_name(channel_id)
        .unwrap_or_default()
        .to_owned();
    let names: Vec<String> = app
        .store
        .voice_members(guild_id, channel_id)
        .iter()
        .map(|member| member_name(app, guild_id, member.user_id))
        .collect();
    let backdrop = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .children(LOBBY_BANDS.map(|(share, from, to)| {
            div().w_full().h(relative(share)).bg(linear_gradient(
                180.,
                linear_color_stop(rgb(from), 0.),
                linear_color_stop(rgb(to), 1.),
            ))
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(relative(0.5))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgba(0x00000080), 0.),
                    linear_color_stop(rgba(0x00000000), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(relative(0.5))
                .bg(linear_gradient(
                    270.,
                    linear_color_stop(rgba(0x00000080), 0.),
                    linear_color_stop(rgba(0x00000000), 1.),
                )),
        );
    let join = app.store.can_connect_voice(guild_id, channel_id).then(|| {
        let entity = app.entity.clone();
        div()
            .id("lobby-join-voice")
            .flex()
            .items_center()
            .h(rpx(38.))
            .mt(rpx(24.))
            .px(rpx(16.))
            .rounded(rpx(8.))
            .bg(white())
            .hover(|style| style.bg(rgb(0xE6E6E8)))
            .cursor_pointer()
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(black())
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.join_voice(guild_id, channel_id, cx))
                    .ok();
            })
            .child("Join Voice")
    });
    let body = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(rpx(16.))
        .child(
            div()
                .max_w_full()
                .truncate()
                .text_size(rpx(32.))
                .line_height(rpx(40.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(channel.clone()),
        )
        .child(
            div()
                .mt(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::text()))
                .child(in_voice(&names)),
        )
        .children(join);
    let header = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(chat::HEADER_HEIGHT))
        .px(rpx(16.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white())
        .child(theme::icon("volume-2", 20., 0xFFFFFF))
        .child(div().flex_1().min_w_0().truncate().child(channel.clone()))
        .when(!chat_open, |this| {
            this.child(stream_view::show_chat_button(app))
        });
    let area = stream_view::stage_area(app, titlebar::HEIGHT, 0., chat_open)
        .child(backdrop)
        .child(body)
        .child(header);
    Some(stream_view::stage_frame(app, area, channel, chat_open))
}

#[cfg(test)]
mod tests {
    use super::{grid, in_voice};

    #[test]
    fn lobby_line_names_up_to_two_members() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(in_voice(&[]), "No one is currently in voice");
        assert_eq!(in_voice(&names(&["Ann"])), "Ann is currently in voice");
        assert_eq!(
            in_voice(&names(&["Ann", "Bo"])),
            "Ann and Bo are currently in voice"
        );
        assert_eq!(
            in_voice(&names(&["Ann", "Bo", "Cy"])),
            "Ann, Bo, and 1 other are currently in voice"
        );
        assert_eq!(
            in_voice(&names(&["Ann", "Bo", "Cy", "Di"])),
            "Ann, Bo, and 2 others are currently in voice"
        );
    }

    #[test]
    fn a_tall_stage_stacks_tiles() {
        assert_eq!(grid(2, 400., 900.).0, 1);
    }

    #[test]
    fn an_empty_stage_has_one_column() {
        assert_eq!(grid(0, 800., 600.).0, 1);
    }
}
