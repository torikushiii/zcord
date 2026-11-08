use super::message::passive_reply_pieces;
use crate::{
    app::messages::rich_text::{Action, Chip, Content, Piece, PieceStyle},
    theme::{self},
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
