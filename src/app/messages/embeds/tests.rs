use std::borrow::Cow;

use super::{EMBED_MEDIA, description_text, fields::field_bases, inline_media, media_max};
use crate::store::{Embed, EmbedField, EmbedKind, Media};

#[test]
fn only_orphaned_selectors_are_removed() {
    let source = "Beach skin! \u{1F3D6}\u{FE0F}\n\u{FE00}\u{FE00}\n\u{FE00}\u{FE00}[#ZZZ](https://example.com)\n  indented\n**bold**\n> \u{FE00}\u{FE00}quoted";
    assert_eq!(
        description_text(source),
        "Beach skin! \u{1F3D6}\u{FE0F}\n\n[#ZZZ](https://example.com)\n  indented\n**bold**\n> quoted"
    );
    for source in [
        "",
        "normal\n  indentation",
        "\u{1F3D6}\u{FE0F}",
        "\u{4E00}\u{FE00}",
    ] {
        assert!(matches!(description_text(source), Cow::Borrowed(_)));
    }
}

fn inline_field_embed() -> Embed {
    Embed {
        kind: EmbedKind::Rich,
        url: None,
        color: Some(0xED4245),
        provider: None,
        author: None,
        title: None,
        description: None,
        fields: [
            ("Current Stamina", "360 / 360", true),
            ("UID", "123456789", true),
            ("Rank", "60", true),
            ("Region", "Example", false),
        ]
        .into_iter()
        .map(|(name, value, inline)| EmbedField {
            name: name.into(),
            value: value.into(),
            inline,
        })
        .collect(),
        components: Vec::new(),
        thumbnail: None,
        images: Vec::new(),
        video: None,
        footer: None,
        timestamp: None,
    }
}

fn media(width: u32, height: u32) -> Media {
    Media {
        url: "https://cdn.discordapp.com/attachments/1/2/image.png".into(),
        source: "https://example.com/image.png".into(),
        width,
        height,
        animated: false,
    }
}

#[test]
fn bare_media_and_tall_boxes_follow_discord() {
    let mut embed = inline_field_embed();
    embed.fields.clear();
    embed.kind = EmbedKind::Link;
    embed.images = vec![media(800, 600)];
    assert!(inline_media(&embed));
    embed.description = Some("descriptions don't frame media".into());
    assert!(inline_media(&embed));
    embed.title = Some(crate::store::EmbedText {
        text: "Title".into(),
        url: None,
        icon: None,
    });
    assert!(!inline_media(&embed));
    embed.kind = EmbedKind::Rich;
    embed.title = None;
    assert!(!inline_media(&embed));

    embed.kind = EmbedKind::Video;
    embed.video = Some(Media {
        source: "https://www.youtube.com/embed/abc".into(),
        ..media(1080, 1920)
    });
    assert_eq!(media_max(&embed), (400., 450.));
    embed.video = Some(Media {
        source: "https://www.youtube.com/embed/abc".into(),
        ..media(1920, 1080)
    });
    assert_eq!(media_max(&embed), EMBED_MEDIA);
}

#[test]
fn field_columns_line_up_across_rows_like_css_grid() {
    let bases = field_bases(&[vec![75.3, 62.2, 41.7], vec![51.8, 89.9]]);
    let expected = [vec![75.3, 66.567, 50.433], vec![110.4, 89.9]];
    for (row, expected) in bases.iter().zip(expected) {
        for (basis, expected) in row.iter().zip(expected) {
            assert!((basis - expected).abs() < 0.01, "{basis} != {expected}");
        }
    }
}
