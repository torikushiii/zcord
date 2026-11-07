use gpui::{
    Context, IntoElement, ListAlignment, ListOffset, ListState, Render, TestAppContext, Window,
    div, list, prelude::*, px,
};

use super::{Layout, ROW_HEIGHT, WIDTH, group_row, member_activity, placeholder_row};
use crate::{
    store::{CustomStatus, GameActivity, MemberList, MemberRow, Status, User},
    theme::rpx,
};

fn members(count: usize) -> MemberList {
    let mut rows = vec![None; count];
    if let Some(first) = rows.first_mut() {
        *first = Some(MemberRow::Group {
            name: "Online".into(),
            count: count as u32,
        });
    }
    MemberList {
        id: "everyone".into(),
        rows,
        ..MemberList::default()
    }
}

fn assert_position(state: &ListState, index: usize, offset: f32) {
    let top = state.logical_scroll_top();
    assert_eq!(top.item_ix, index);
    assert!((top.offset_in_item - px(offset)).abs() < px(0.01));
}

#[test]
fn range_sync_and_presence_updates_keep_the_scroll_anchor() {
    let state = ListState::new(0, ListAlignment::Top, px(400.));
    let mut layout = Layout::default();
    let mut list = members(500);
    layout.sync(&state, Some(&list), 0);
    state.scroll_to(ListOffset {
        item_ix: 150,
        offset_in_item: px(13.),
    });
    for index in 200..300 {
        list.rows[index] = Some(MemberRow::Member {
            user_id: index as u64,
        });
    }
    layout.sync(&state, Some(&list), 0);
    assert_position(&state, 150, 13.);
    layout.sync(&state, Some(&list), 0);
    assert_position(&state, 150, 13.);
    assert_eq!(state.item_count(), 500);

    list.rows.resize(550, None);
    layout.sync(&state, Some(&list), 0);
    assert_position(&state, 150, 13.);
    list.rows.truncate(450);
    layout.sync(&state, Some(&list), 0);
    assert_position(&state, 150, 13.);
    layout.sync(&state, None, 0);
    assert_eq!(state.item_count(), 0);
    layout.sync(&state, Some(&list), 0);
    assert_position(&state, 0, 0.);
}

struct MemberSidebar {
    list: MemberList,
    state: ListState,
    layout: Layout,
}

impl MemberSidebar {
    fn new() -> Self {
        let list = members(500);
        let state = ListState::new(0, ListAlignment::Top, px(400.));
        let mut layout = Layout::default();
        layout.sync(&state, Some(&list), 0);
        Self {
            list,
            state,
            layout,
        }
    }

    fn sync(&mut self) {
        self.layout.sync(&self.state, Some(&self.list), 0);
    }
}

impl Render for MemberSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div().w(rpx(WIDTH)).h(rpx(500.)).child(
            list(self.state.clone(), move |index, _, cx| {
                let sidebar = entity.read(cx);
                let content = match &sidebar.list.rows[index] {
                    Some(MemberRow::Group { name, count }) => {
                        group_row(index, name, *count).into_any_element()
                    }
                    Some(MemberRow::Member { .. }) => div().h(rpx(ROW_HEIGHT)).into_any_element(),
                    None => placeholder_row(),
                };
                div()
                    .w_full()
                    .px_2()
                    .when(index + 1 == sidebar.list.rows.len(), |this| this.pb_4())
                    .debug_selector(move || format!("member-row-{index}"))
                    .child(content)
                    .into_any_element()
            })
            .size_full(),
        )
    }
}

#[gpui::test]
fn incoming_pages_and_group_changes_do_not_jump_a_scrolled_viewport(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| MemberSidebar::new());
    cx.update(|window, cx| {
        window.set_rem_size(px(16.));
        let _ = window.draw(cx);
    });
    view.update(cx, |sidebar, cx| {
        sidebar.state.scroll_to(ListOffset {
            item_ix: 150,
            offset_in_item: px(13.),
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let original = cx.debug_bounds("member-row-150").unwrap();

    for index in [220, 150, 300] {
        view.update(cx, |sidebar, cx| {
            for index in index..index + 30 {
                sidebar.list.rows[index] = Some(MemberRow::Member {
                    user_id: index as u64,
                });
            }
            sidebar.sync();
            assert_position(&sidebar.state, 150, 13.);
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let current = cx.debug_bounds("member-row-150").unwrap();
        assert_eq!(current.origin, original.origin);
        assert_eq!(current.size.height, px(ROW_HEIGHT));
    }

    view.update(cx, |sidebar, cx| {
        sidebar.list.rows[150] = Some(MemberRow::Group {
            name: "Offline".into(),
            count: 100,
        });
        sidebar.sync();
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let group = cx.debug_bounds("member-row-150").unwrap();
    assert_eq!(group.origin, original.origin);
    assert_eq!(group.size.height, px(46.));

    view.update(cx, |sidebar, cx| {
        sidebar.list.rows[150] = None;
        sidebar.sync();
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let placeholder = cx.debug_bounds("member-row-150").unwrap();
    assert_eq!(placeholder.origin, original.origin);
    assert_eq!(placeholder.size.height, px(ROW_HEIGHT));
}

#[gpui::test]
fn resizing_the_list_moves_bottom_padding_without_replacing_other_rows(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| MemberSidebar::new());
    cx.update(|window, cx| {
        window.set_rem_size(px(16.));
        let _ = window.draw(cx);
    });
    view.update(cx, |sidebar, cx| {
        sidebar.state.scroll_to(ListOffset {
            item_ix: 495,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(
        cx.debug_bounds("member-row-499").unwrap().size.height,
        px(60.)
    );
    view.update(cx, |sidebar, cx| {
        sidebar.list.rows.resize(502, None);
        sidebar.sync();
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(
        cx.debug_bounds("member-row-499").unwrap().size.height,
        px(44.)
    );
    view.update(cx, |sidebar, cx| {
        sidebar.state.scroll_to(ListOffset {
            item_ix: 501,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(
        cx.debug_bounds("member-row-501").unwrap().size.height,
        px(60.)
    );
    view.update(cx, |sidebar, cx| {
        sidebar.list.rows.truncate(500);
        sidebar.sync();
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(
        cx.debug_bounds("member-row-499").unwrap().size.height,
        px(60.)
    );
}

fn user(
    activity: Option<&str>,
    activity_kind: Option<u8>,
    custom_status: Option<&str>,
    listening: Option<(&str, Option<&str>, Option<&str>)>,
) -> User {
    User {
        id: 1,
        username: "listener".into(),
        name: "Listener".into(),
        initials: "L".into(),
        status: Status::Online,
        bot: false,
        verified_bot: false,
        role_color: None,
        tag: None,
        playing: false,
        activity: activity.map(str::to_owned),
        activity_kind,
        game: None,
        live_stream: None,
        on_mobile: false,
        listening: listening.map(|(name, details, state)| GameActivity {
            name: name.into(),
            application_id: None,
            kind: 2,
            platform: None,
            embedded: false,
            playable: false,
            started_at: None,
            ends_at: None,
            details: details.map(str::to_owned),
            state: state.map(str::to_owned),
            large_image: None,
            small_image: None,
        }),
        custom_status: custom_status.map(|text| CustomStatus {
            text: Some(text.into()),
            emoji: None,
            expires_at: None,
        }),
        avatar: None,
        discriminator: None,
        decoration: None,
        nameplate: None,
        name_style: None,
    }
}

#[test]
fn listening_activity_shows_the_artist_and_custom_status() {
    let youtube = Some(("YouTube Music", Some("Colorful Universe"), Some("Laur")));

    assert_eq!(
        member_activity(&user(
            Some("Listening to YouTube Music"),
            Some(2),
            None,
            youtube
        )),
        Some(("Laur".into(), Some("music")))
    );

    assert_eq!(
        member_activity(&user(Some("chilling"), None, Some("chilling"), youtube)),
        Some(("Laur · chilling".into(), Some("music")))
    );

    assert_eq!(
        member_activity(&user(Some("🎮 chilling"), None, Some("chilling"), youtube)),
        Some(("Laur · 🎮 chilling".into(), Some("music")))
    );

    assert_eq!(
        member_activity(&user(
            Some("chilling"),
            None,
            Some("chilling"),
            Some(("YouTube Music", Some("Colorful Universe"), None))
        )),
        Some(("chilling".into(), Some("music")))
    );

    let nameless = Some(("YouTube", None, None));
    assert_eq!(
        member_activity(&user(Some("Listening to YouTube"), Some(2), None, nameless)),
        Some(("Listening to YouTube".into(), Some("music")))
    );

    let mut game = user(
        Some("Example Game"),
        Some(0),
        None,
        Some(("YouTube Music", Some("Song"), None)),
    );
    game.playing = true;
    assert_eq!(
        member_activity(&game),
        Some(("Example Game".into(), Some("gamepad-2")))
    );

    let mut custom_over_game = user(Some("chilling"), None, Some("chilling"), youtube);
    custom_over_game.game = custom_over_game.listening.clone();
    assert_eq!(
        member_activity(&custom_over_game),
        Some(("chilling".into(), None))
    );

    assert_eq!(
        member_activity(&user(
            Some("Laur"),
            None,
            Some("Laur"),
            Some(("YouTube Music", Some("Colorful Universe"), Some("Laur")))
        )),
        Some(("Laur".into(), Some("music")))
    );

    assert_eq!(
        member_activity(&user(
            Some("Listening to YouTube Music"),
            Some(2),
            Some("guild emoji only"),
            youtube
        )),
        Some(("Laur".into(), Some("music")))
    );
}
