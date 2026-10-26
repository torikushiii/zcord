use gpui::{Context, IntoElement, Render, TestAppContext, Window, div, prelude::*, px};

use super::message::attachment_reply_preview;
use crate::theme::rpx;

struct AttachmentReply {
    width: f32,
}

impl Render for AttachmentReply {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(rpx(self.width))
            .h(rpx(22.))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .debug_selector(|| "reply-preview".into())
            .child(attachment_reply_preview().w_full())
    }
}

#[gpui::test]
fn attachment_label_and_icon_share_a_single_line_at_every_zoom(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| AttachmentReply { width: 300. });
    for width in [300., 140., 70.] {
        for zoom in [1., 1.25, 1.5] {
            view.update(cx, |view, cx| {
                view.width = width;
                cx.notify();
            });
            cx.update(|window, cx| {
                window.set_rem_size(px(16. * zoom));
                window.refresh();
                let _ = window.draw(cx);
            });
            let preview = cx.debug_bounds("reply-preview").unwrap();
            let label = cx.debug_bounds("reply-attachment-label").unwrap();
            let icon = cx.debug_bounds("reply-attachment-icon").unwrap();
            assert!((label.size.height - px(18. * zoom)).abs() <= px(1.));
            assert!((icon.size.width - px(18. * zoom)).abs() <= px(1.));
            assert!(icon.left() >= label.right() + px(4. * zoom - 1.));
            assert!(icon.right() <= preview.right() + px(1.));
            assert!((icon.center().y - label.center().y).abs() <= px(1.));
        }
    }
}
