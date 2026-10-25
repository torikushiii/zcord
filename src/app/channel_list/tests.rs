use std::{cell::RefCell, rc::Rc, time::Instant};

use gpui::{
    Context, IntoElement, ListOffset, Render, TestAppContext, Window, div, list, prelude::*, px,
};

use super::{
    BANNER_HEIGHT, BANNER_HOVER_FADE, BannerHover, FOLD, HEADER_HEIGHT, Row, Rows, WIDTH, row_content,
};
use crate::{
    images::ImageSpec,
    store::{Category, Channel, ChannelKind, Guild},
    theme::rpx,
};

fn guild(id: u64, channels: usize) -> Guild {
    Guild {
        id,
        name: "Large server".into(),
        initials: "LS".into(),
        icon_color: 0,
        unread: false,
        mentions: 0,
        categories: vec![Category {
            id: id + 1,
            name: "Channels".into(),
            channels: (0..channels)
                .map(|index| Channel {
                    id: index as u64 + 100,
                    name: format!("channel-{index}"),
                    kind: ChannelKind::Text,
                    parent_id: Some(id + 1),
                    unread: index == 1,
                    mentions: 0,
                    new_posts: 0,
                    messages: Vec::new(),
                    messages_loaded: false,
                })
                .collect(),
        }],
        threads: Vec::new(),
        member_ids: Vec::new(),
        icon: None,
        banner: None,
        emojis: Vec::new(),
    }
}

#[test]
fn hierarchy_updates_without_resetting_scroll_for_the_same_server() {
    let mut guild = guild(1, 4);
    let mut rows = Rows::new();
    rows.sync(Some(&guild), 101);
    assert_eq!(
        rows.rows,
        [
            Row::Shortcuts,
            Row::Category(0),
            Row::Channel {
                category: 0,
                channel: 0
            },
            Row::Channel {
                category: 0,
                channel: 1
            },
            Row::Channel {
                category: 0,
                channel: 2
            },
            Row::Channel {
                category: 0,
                channel: 3
            },
        ]
    );
    let position = ListOffset {
        item_ix: 3,
        offset_in_item: px(5.),
    };
    rows.state.scroll_to(position);
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.state.logical_scroll_top().item_ix, position.item_ix);
    assert_eq!(
        rows.state.logical_scroll_top().offset_in_item,
        position.offset_in_item
    );
    guild.categories[0].channels.pop();
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.rows.len(), 5);
    assert_eq!(rows.state.item_count(), 5);
    assert_eq!(rows.state.logical_scroll_top().item_ix, position.item_ix);
    guild.id = 2;
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.state.logical_scroll_top().item_ix, 0);
    rows.sync(None, 0);
    assert_eq!(rows.rows, [Row::Shortcuts]);
    assert_eq!(rows.state.item_count(), 1);
}

#[test]
fn collapsed_categories_hide_channels_but_keep_the_selected_one_across_syncs() {
    let first = guild(1, 4);
    let mut rows = Rows::new();
    rows.sync(Some(&first), 101);
    let category = first.categories[0].id;
    rows.collapsed.insert(category);
    rows.sync(Some(&first), 101);
    assert_eq!(
        rows.rows,
        [
            Row::Shortcuts,
            Row::Category(0),
            Row::Channel {
                category: 0,
                channel: 1
            },
        ]
    );
    assert_eq!(rows.state.item_count(), 3);

    rows.sync(Some(&first), 100);
    assert_eq!(
        rows.rows,
        [
            Row::Shortcuts,
            Row::Category(0),
            Row::Channel {
                category: 0,
                channel: 0
            },
        ]
    );

    rows.collapsed.remove(&category);
    rows.sync(Some(&first), 100);
    assert_eq!(rows.rows.len(), 6);
    assert_eq!(rows.state.item_count(), 6);

    rows.collapsed.insert(category);
    rows.sync(Some(&first), 101);
    assert_eq!(rows.rows.len(), 3);
    rows.sync(Some(&guild(2, 2)), 100);
    assert_eq!(rows.rows.len(), 4);
    assert!(rows.collapsed.contains(&category));
    rows.sync(Some(&first), 101);
    assert_eq!(rows.rows.len(), 3);
    assert_eq!(rows.state.item_count(), 3);
}

#[test]
fn collapsing_keeps_channels_listed_until_the_fold_ends() {
    let guild = guild(1, 4);
    let mut rows = Rows::new();
    rows.sync(Some(&guild), 101);
    let category = guild.categories[0].id;
    assert!(rows.toggle_collapsed(category));
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.rows.len(), 6);
    rows.folds.get_mut(&category).unwrap().started -= FOLD;
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.rows.len(), 3);
    assert_eq!(rows.state.item_count(), 3);
    assert!(!rows.toggle_collapsed(category));
    rows.sync(Some(&guild), 101);
    assert_eq!(rows.rows.len(), 6);
}

#[test]
fn banner_hover_fades_and_reverses_without_jumping() {
    let mut hover = BannerHover::default();
    let now = Instant::now();
    assert_eq!(hover.opacity(now), 0.);
    assert!(!hover.animating(now));

    assert!(hover.set(true, now));
    assert_eq!(hover.opacity(now), 0.);
    let halfway = now + BANNER_HOVER_FADE / 2;
    let opacity = hover.opacity(halfway);
    assert!((opacity - 0.5).abs() < 0.001);
    assert!(hover.animating(halfway));

    assert!(hover.set(false, halfway));
    assert_eq!(hover.opacity(halfway), opacity);
    assert!(hover.opacity(halfway + BANNER_HOVER_FADE / 2) < opacity);
    assert_eq!(hover.opacity(halfway + BANNER_HOVER_FADE), 0.);
    assert!(!hover.animating(halfway + BANNER_HOVER_FADE));

    let later = halfway + BANNER_HOVER_FADE;
    assert!(hover.set(true, later));
    assert_eq!(hover.opacity(later + BANNER_HOVER_FADE), 1.);
}

#[test]
fn repeated_header_hover_does_not_restart_the_fade() {
    let mut hover = BannerHover::default();
    let now = Instant::now();
    hover.set(true, now);
    let halfway = now + BANNER_HOVER_FADE / 2;
    let opacity = hover.opacity(halfway);
    assert!(!hover.set(true, halfway));
    assert_eq!(hover.opacity(halfway), opacity);
    let later = now + BANNER_HOVER_FADE;
    assert_eq!(hover.opacity(later), 1.);
    assert!(!hover.animating(later));
    assert!(hover.set(false, later));
    assert_eq!(hover.opacity(later + BANNER_HOVER_FADE), 0.);
}

#[test]
fn banner_hover_survives_sync_but_resets_on_server_change_or_banner_removal() {
    let mut rows = Rows::new();
    let first = guild(1, 1);
    let banner = ImageSpec::GuildBanner {
        guild_id: first.id,
        hash: "banner".into(),
    };
    let now = Instant::now();
    rows.sync_with_banner(Some(&first), Some(banner.clone()), 100);
    rows.banner_hover.set(true, now);
    rows.sync_with_banner(Some(&first), Some(banner.clone()), 100);
    assert_eq!(rows.banner_hover.opacity(now + BANNER_HOVER_FADE), 1.);

    let second = guild(2, 1);
    let second_banner = ImageSpec::GuildBanner {
        guild_id: second.id,
        hash: "other_banner".into(),
    };
    rows.sync_with_banner(Some(&second), Some(second_banner), 100);
    assert_eq!(rows.banner_hover.opacity(now + BANNER_HOVER_FADE), 0.);
    rows.banner_hover.set(true, now);
    rows.sync_with_banner(Some(&second), None, 100);
    assert_eq!(rows.banner_hover.opacity(now + BANNER_HOVER_FADE), 0.);
}

#[test]
fn banner_header_becomes_solid_at_the_same_scroll_position_at_every_zoom() {
    let guild = guild(1, 1);
    let mut rows = Rows::new();
    rows.sync_with_banner(
        Some(&guild),
        Some(ImageSpec::GuildBanner {
            guild_id: guild.id,
            hash: "banner".into(),
        }),
        100,
    );
    assert_eq!(rows.rows[0], Row::Banner);
    for zoom in [1., 1.25, 1.5] {
        let rem_size = px(16. * zoom);
        let range = rpx(BANNER_HEIGHT - HEADER_HEIGHT).to_pixels(rem_size);
        for fraction in [0., 0.5, 1.] {
            rows.state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: range * fraction,
            });
            assert_eq!(rows.header_solidity(rem_size), Some(fraction));
        }
        rows.state.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(0.),
        });
        assert_eq!(rows.header_solidity(rem_size), Some(1.));
    }
    rows.sync_with_banner(Some(&guild), None, 100);
    assert_eq!(rows.header_solidity(px(16.)), None);
}

struct Sidebar {
    guild: Guild,
    rows: Rows,
    rendered: Rc<RefCell<Vec<usize>>>,
}

impl Sidebar {
    fn new(channels: usize, rendered: Rc<RefCell<Vec<usize>>>) -> Self {
        let guild = guild(1, channels);
        let mut rows = Rows::new();
        rows.sync(Some(&guild), 101);
        Self {
            guild,
            rows,
            rendered,
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div().w(rpx(WIDTH)).h(rpx(500.)).child(
            list(self.rows.state.clone(), move |index, _, cx| {
                let sidebar = entity.read(cx);
                sidebar.rendered.borrow_mut().push(index);
                row_content(Some(&sidebar.guild), sidebar.rows.rows[index], 101, 1.)
                    .id(("row", index))
                    .debug_selector(move || format!("row-{index}"))
                    .into_any_element()
            })
            .size_full()
            .py_2(),
        )
    }
}

#[gpui::test]
fn hundreds_of_channels_only_build_visible_rows_and_can_reach_the_end(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_, _| Sidebar::new(500, rendered.clone()));
    for zoom in [1., 1.25, 1.5] {
        rendered.borrow_mut().clear();
        view.update(cx, |sidebar, cx| {
            sidebar.rows.state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            window.set_rem_size(px(16. * zoom));
            window.refresh();
            let _ = window.draw(cx);
        });
        assert!(
            rendered.borrow().len() < 80,
            "off-screen channels should not be built"
        );
        let category = cx.debug_bounds("row-1").unwrap();
        let channel = cx.debug_bounds("row-2").unwrap();
        assert!((category.size.height - px(40. * zoom)).abs() <= px(1.));
        assert!((channel.size.height - px(34. * zoom)).abs() <= px(1.));
        assert!(
            (channel.size.width - px(WIDTH * zoom)).abs() <= px(1.),
            "zoom={zoom} channel={channel:?}",
        );

        rendered.borrow_mut().clear();
        view.update(cx, |sidebar, cx| {
            sidebar.rows.state.scroll_to(ListOffset {
                item_ix: 501,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(rendered.borrow().len() < 80);
        assert!(rendered.borrow().contains(&501));
        assert!(cx.debug_bounds("row-501").is_some());
    }
}
