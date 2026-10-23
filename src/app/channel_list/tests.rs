use std::{cell::RefCell, rc::Rc};

use gpui::{
    Context, IntoElement, ListOffset, Render, TestAppContext, Window, div, list, prelude::*, px,
};

use super::{Row, Rows, WIDTH, row_content};
use crate::{
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
                    unread: index == 1,
                    messages: Vec::new(),
                    messages_loaded: false,
                })
                .collect(),
        }],
        member_ids: Vec::new(),
        icon: None,
        emojis: Vec::new(),
    }
}

#[test]
fn hierarchy_updates_without_resetting_scroll_for_the_same_server() {
    let mut guild = guild(1, 4);
    let mut rows = Rows::new();
    rows.sync(Some(&guild));
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
    rows.sync(Some(&guild));
    assert_eq!(rows.state.logical_scroll_top().item_ix, position.item_ix);
    assert_eq!(
        rows.state.logical_scroll_top().offset_in_item,
        position.offset_in_item
    );
    guild.categories[0].channels.pop();
    rows.sync(Some(&guild));
    assert_eq!(rows.rows.len(), 5);
    assert_eq!(rows.state.item_count(), 5);
    assert_eq!(rows.state.logical_scroll_top().item_ix, position.item_ix);
    guild.id = 2;
    rows.sync(Some(&guild));
    assert_eq!(rows.state.logical_scroll_top().item_ix, 0);
    rows.sync(None);
    assert_eq!(rows.rows, [Row::Shortcuts]);
    assert_eq!(rows.state.item_count(), 1);
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
        rows.sync(Some(&guild));
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
                row_content(Some(&sidebar.guild), sidebar.rows.rows[index], 101)
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
