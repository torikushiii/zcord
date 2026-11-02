use gpui::{Context, IntoElement, Render, TestAppContext, Window, div, prelude::*, px};

use super::message::{attachment_reply_preview, passive_reply_pieces};
use crate::{
    app::rich_text::{Action, Chip, Content, Piece, PieceStyle},
    theme::{self, rpx},
};

#[test]
fn local_first_unread_matches_the_full_scan_with_placeholders() {
    use crate::store::{Delivery, Store};

    let mut store = Store::seed();
    let channel = store.guilds[0].categories[0].channels.first_mut().unwrap();
    channel.messages.truncate(6);
    assert_eq!(channel.messages.len(), 6);
    for pattern in 0..3u32.pow(6) {
        let mut remaining = pattern;
        for (index, message) in channel.messages.iter_mut().enumerate() {
            message.delivery = match remaining % 3 {
                0 => Delivery::Sent,
                1 => Delivery::Pending,
                _ => Delivery::Failed,
            };
            message.id = if message.delivery == Delivery::Sent {
                (index as u64 + 1) * 10
            } else {
                1000 - index as u64
            };
            remaining /= 3;
        }
        for since in [0, 10, 15, 20, 30, 40, 50, 60, 1000] {
            for complete in [false, true] {
                for index in 0..channel.messages.len() {
                    let start = index == 0 && complete;
                    let expected = (index > 0 || start)
                        && channel.first_unread_index(since, 9999) == Some(index);
                    assert_eq!(
                        super::rows::is_first_unread(&channel.messages, index, since, start, 9999),
                        expected,
                        "pattern={pattern}, since={since}, complete={complete}, index={index}",
                    );
                }
            }
        }
    }
}

#[test]
fn reply_links_keep_their_colors_and_formatting_without_hover_actions() {
    for color in [None, Some(gpui::rgb(0x123456).into())] {
        for underline in [false, true] {
            let style = PieceStyle {
                color,
                underline,
                italic: true,
                action: Some(Action::OpenUrl("https://example.test".into())),
                ..Default::default()
            };
            let pieces = passive_reply_pieces(vec![Piece::text("example", style.clone())]);
            let preview = &pieces[0];
            assert!(matches!(&preview.content, Content::Text(text) if &**text == "example"));
            assert_eq!(
                preview.style.color,
                Some(color.unwrap_or_else(|| gpui::rgb(theme::link()).into()))
            );
            assert_eq!(preview.style.underline, underline);
            assert!(preview.style.italic);
            assert!(preview.style.action.is_none());
            assert!(matches!(style.action, Some(Action::OpenUrl(_))));
        }
    }
}

#[test]
fn reply_mentions_spoilers_and_emoji_stay_passive_without_losing_their_appearance() {
    let pieces = passive_reply_pieces(vec![
        Piece::text(
            "@ExampleUser",
            PieceStyle {
                chip: Some(Chip::Mention { color: None }),
                group: 1,
                action: Some(Action::OpenUserProfile(42)),
                ..Default::default()
            },
        ),
        Piece::text(
            "hidden",
            PieceStyle {
                chip: Some(Chip::Spoiler { revealed: false }),
                group: 2,
                action: Some(Action::RevealSpoiler(2)),
                ..Default::default()
            },
        ),
        Piece {
            content: Content::Emoji(None, ":wave:".into()),
            style: PieceStyle {
                action: Some(Action::InspectEmoji {
                    id: 43,
                    name: "wave".into(),
                    animated: true,
                }),
                ..Default::default()
            },
        },
    ]);
    assert!(pieces.iter().all(|piece| piece.style.action.is_none()));
    assert!(matches!(
        pieces[0].style.chip,
        Some(Chip::Mention { color: None })
    ));
    assert_eq!(pieces[0].style.group, 1);
    assert!(matches!(
        pieces[1].style.chip,
        Some(Chip::Spoiler { revealed: false })
    ));
    assert_eq!(pieces[1].style.group, 2);
    assert!(matches!(&pieces[2].content, Content::Emoji(None, text) if &**text == ":wave:"));
}

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
