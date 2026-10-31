use std::sync::Arc;

use gpui::{
    Context, Entity, ImageSource, IntoElement, Render, RenderImage, TestAppContext, Window, div,
    img, prelude::*, px, rgb,
};

use super::{
    ACTIVITY_TOOLTIP_MAX_WIDTH, ICON, TOOLTIP_AVATAR_LIMIT, TOOLTIP_AVATAR_OVERLAP,
    TOOLTIP_AVATAR_SIZE, WIDTH, activity_row, activity_tooltip_bubble, positioned_tooltip,
};
use crate::theme::{self, rpx};

struct ActivityPreview {
    count: usize,
    name: &'static str,
    image: Arc<RenderImage>,
    hovered: bool,
}

struct PreviewHost {
    preview: Entity<ActivityPreview>,
}

impl Render for PreviewHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(
                self.preview
                    .clone()
                    .cached(crate::app::views::guild_rail_style()),
            )
            .child(div().flex_1().h_full())
    }
}

impl Render for ActivityPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let image = self.image.clone();
        let row = |icon: &'static str, count| {
            let image = image.clone();
            let avatars = (0..count).map(move |index| {
                div()
                    .size(rpx(TOOLTIP_AVATAR_SIZE - 4.))
                    .debug_selector(move || format!("preview-avatar-{icon}-{index}"))
                    .child(theme::avatar(
                        "A",
                        rgb(theme::BG).into(),
                        TOOLTIP_AVATAR_SIZE - 4.,
                        None,
                        Some(
                            img(ImageSource::Render(image.clone()))
                                .size_full()
                                .rounded_full()
                                .into_any_element(),
                        ),
                        None,
                    ))
                    .into_any_element()
            });
            activity_row(icon, count, avatars)
                .debug_selector(move || format!("activity-row-{icon}"))
        };
        let bubble = activity_tooltip_bubble(1, self.name)
            .debug_selector(|| "activity-tooltip".into())
            .child(row("volume-2", self.count))
            .child(row("monitor-filled", self.count.min(6)));
        div()
            .id("preview-rail")
            .flex()
            .flex_col()
            .items_center()
            .w(rpx(WIDTH))
            .h_full()
            .overflow_y_scroll()
            .child(div().h(rpx(96.)).flex_none())
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .h(rpx(ICON))
                    .debug_selector(|| "preview-guild".into())
                    .when(self.hovered, |this| this.child(positioned_tooltip(bubble)))
                    .child(div().size(rpx(ICON)).flex_none()),
            )
    }
}

#[gpui::test]
fn voice_preview_keeps_rows_inside_anchored_bubble_at_each_zoom(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (host, cx) = cx.add_window_view(|_, cx| PreviewHost {
        preview: cx.new(|_| ActivityPreview {
            count: 1,
            name: "Sample",
            hovered: true,
            image: Arc::new(RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4])),
            )])),
        }),
    });
    let view = host.read_with(cx, |host, _| host.preview.clone());
    for name in [
        "Sample",
        "logchannel",
        "Example Server With A Long Name That Must Wrap Inside The Tooltip",
    ] {
        for count in [1, 5, 6, 15, 42, 10_000] {
            view.update(cx, |view, cx| {
                view.count = count;
                view.name = name;
                cx.notify();
            });
            for zoom in [1., 1.125, 1.25, 1.5] {
                cx.update(|window, cx| {
                    window.set_rem_size(px(16. * zoom));
                    window.refresh();
                    let _ = window.draw(cx);
                });
                let tooltip = cx.debug_bounds("activity-tooltip").unwrap();
                assert!(tooltip.size.width <= px(ACTIVITY_TOOLTIP_MAX_WIDTH * zoom + 1.));
                if count == 1 && matches!(name, "Sample" | "logchannel") {
                    assert!(
                        tooltip.size.width <= px(144. * zoom),
                        "one-person preview should fit its title and avatar: {tooltip:?}"
                    );
                }
                if name == "Sample" && count <= 42 {
                    assert!(tooltip.size.width <= px(208. * zoom));
                }
                let guild = cx.debug_bounds("preview-guild").unwrap();
                assert!(tooltip.left() >= guild.right());
                for (row_selector, avatar_selectors, overflow_selector, count) in [
                    (
                        "activity-row-volume-2",
                        [
                            "preview-avatar-volume-2-0",
                            "preview-avatar-volume-2-1",
                            "preview-avatar-volume-2-2",
                            "preview-avatar-volume-2-3",
                            "preview-avatar-volume-2-4",
                            "preview-avatar-volume-2-5",
                        ],
                        "rail-activity-overflow-volume-2",
                        count,
                    ),
                    (
                        "activity-row-monitor-filled",
                        [
                            "preview-avatar-monitor-filled-0",
                            "preview-avatar-monitor-filled-1",
                            "preview-avatar-monitor-filled-2",
                            "preview-avatar-monitor-filled-3",
                            "preview-avatar-monitor-filled-4",
                            "preview-avatar-monitor-filled-5",
                        ],
                        "rail-activity-overflow-monitor-filled",
                        count.min(6),
                    ),
                ] {
                    let row = cx.debug_bounds(row_selector).unwrap();
                    let horizontal_inset = px(12. * zoom - 1.);
                    let vertical_inset = px(8. * zoom - 1.);
                    assert!(row.left() >= tooltip.left() + horizontal_inset);
                    assert!(row.right() <= tooltip.right() - horizontal_inset);
                    assert!(row.top() >= tooltip.top() + vertical_inset);
                    assert!(row.bottom() <= tooltip.bottom() - vertical_inset);
                    assert!((row.size.height - px(TOOLTIP_AVATAR_SIZE * zoom)).abs() <= px(1.));
                    let mut previous = None;
                    for selector in avatar_selectors
                        .into_iter()
                        .take(count.min(TOOLTIP_AVATAR_LIMIT))
                    {
                        let avatar = cx.debug_bounds(selector).unwrap();
                        assert!(
                            (avatar.size.width - px((TOOLTIP_AVATAR_SIZE - 4.) * zoom)).abs()
                                <= px(1.)
                        );
                        assert!(avatar.left() >= row.left());
                        assert!(avatar.right() <= row.right());
                        if let Some(left) = previous {
                            assert!(
                                (avatar.left()
                                    - left
                                    - px((TOOLTIP_AVATAR_SIZE - TOOLTIP_AVATAR_OVERLAP) * zoom))
                                .abs()
                                    <= px(1.)
                            );
                        }
                        previous = Some(avatar.left());
                    }
                    assert!(cx.debug_bounds(avatar_selectors[5]).is_none());
                    let overflow = cx.debug_bounds(overflow_selector);
                    assert_eq!(overflow.is_some(), count > TOOLTIP_AVATAR_LIMIT);
                    if let Some(overflow) = overflow {
                        assert!(overflow.left() >= row.left());
                        assert!(
                            overflow.right() <= row.right(),
                            "name={name:?} count={count} zoom={zoom} tooltip={tooltip:?} row={row:?} overflow={overflow:?}"
                        );
                        assert!(overflow.size.width >= px(TOOLTIP_AVATAR_SIZE * zoom - 1.));
                    }
                }
            }
        }
    }
    view.update(cx, |view, cx| {
        view.hovered = false;
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("activity-tooltip").is_none());
    assert!(cx.debug_bounds("preview-avatar-volume-2-0").is_none());
    assert!(cx.debug_bounds("rail-activity-overflow-volume-2").is_none());
}
