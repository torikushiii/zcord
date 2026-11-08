use super::activity::{bool_field, rebuild};
use super::*;

const TEXT_AND_IMAGES: u32 = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpoilerMode {
    #[default]
    OnClick,
    IfModerator,
    Always,
}

impl SpoilerMode {
    fn from_wire(value: &str) -> Self {
        match value {
            "IF_MODERATOR" => Self::IfModerator,
            "ALWAYS" => Self::Always,
            _ => Self::OnClick,
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Self::OnClick => "ON_CLICK",
            Self::IfModerator => "IF_MODERATOR",
            Self::Always => "ALWAYS",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickerAnimation {
    #[default]
    Always,
    OnInteraction,
    Never,
}

impl StickerAnimation {
    fn from_wire(value: u64) -> Self {
        match value {
            1 => Self::OnInteraction,
            2 => Self::Never,
            _ => Self::Always,
        }
    }

    fn wire(self) -> u64 {
        match self {
            Self::Always => 0,
            Self::OnInteraction => 1,
            Self::Never => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextImages {
    pub use_rich_chat_input: bool,
    pub use_thread_sidebar: bool,
    pub render_spoilers: SpoilerMode,
    pub view_image_descriptions: bool,
    pub inline_attachment_media: bool,
    pub inline_embed_media: bool,
    pub gif_auto_play: bool,
    pub render_embeds: bool,
    pub render_reactions: bool,
    pub animate_emoji: bool,
    pub animate_stickers: StickerAnimation,
    pub message_display_compact: bool,
    pub convert_emoticons: bool,
    pub include_stickers_in_autocomplete: bool,
    pub is_cross_dm_search_enabled: bool,
    pub include_game_mentions_in_autocomplete: bool,
    pub display_compact_avatars: bool,
}

impl Default for TextImages {
    fn default() -> Self {
        Self {
            use_rich_chat_input: true,
            use_thread_sidebar: true,
            render_spoilers: SpoilerMode::OnClick,
            view_image_descriptions: false,
            inline_attachment_media: true,
            inline_embed_media: true,
            gif_auto_play: true,
            render_embeds: true,
            render_reactions: true,
            animate_emoji: true,
            animate_stickers: StickerAnimation::Always,
            message_display_compact: false,
            convert_emoticons: true,
            include_stickers_in_autocomplete: false,
            is_cross_dm_search_enabled: false,
            include_game_mentions_in_autocomplete: true,
            display_compact_avatars: false,
        }
    }
}

impl TextImages {
    pub fn rebase(&self, base: &Self, onto: &Self) -> Self {
        let mut merged = onto.clone();
        macro_rules! pick {
            ($($field:ident),+ $(,)?) => {
                $(if self.$field != base.$field {
                    merged.$field = self.$field;
                })+
            };
        }
        pick!(
            use_rich_chat_input,
            use_thread_sidebar,
            render_spoilers,
            view_image_descriptions,
            inline_attachment_media,
            inline_embed_media,
            gif_auto_play,
            render_embeds,
            render_reactions,
            animate_emoji,
            animate_stickers,
            message_display_compact,
            convert_emoticons,
            include_stickers_in_autocomplete,
            is_cross_dm_search_enabled,
            include_game_mentions_in_autocomplete,
            display_compact_avatars,
        );
        merged
    }
}

pub(super) fn decode_message(bytes: &[u8], settings: &mut TextImages) -> Result<()> {
    for field in fields(bytes)? {
        let target = match field.number {
            2 => &mut settings.use_rich_chat_input,
            3 => &mut settings.use_thread_sidebar,
            4 => {
                settings.render_spoilers =
                    SpoilerMode::from_wire(&string_wrapper(field.value.ok_or(DecodeError)?)?);
                continue;
            }
            7 => &mut settings.view_image_descriptions,
            9 => &mut settings.inline_attachment_media,
            10 => &mut settings.inline_embed_media,
            11 => &mut settings.gif_auto_play,
            12 => &mut settings.render_embeds,
            13 => &mut settings.render_reactions,
            14 => &mut settings.animate_emoji,
            15 => {
                settings.animate_stickers =
                    StickerAnimation::from_wire(number_wrapper(field.value.ok_or(DecodeError)?)?);
                continue;
            }
            17 => &mut settings.message_display_compact,
            21 => &mut settings.convert_emoticons,
            28 => &mut settings.include_stickers_in_autocomplete,
            36 => &mut settings.is_cross_dm_search_enabled,
            39 => &mut settings.include_game_mentions_in_autocomplete,
            41 => &mut settings.display_compact_avatars,
            _ => continue,
        };
        *target = number_wrapper(field.value.ok_or(DecodeError)?)? != 0;
    }
    Ok(())
}

pub fn text_images(preloaded: &[u8]) -> Result<TextImages> {
    let mut settings = TextImages::default();
    for top in fields(preloaded)? {
        if top.number == TEXT_AND_IMAGES {
            decode_message(top.value.ok_or(DecodeError)?, &mut settings)?;
        }
    }
    Ok(settings)
}

pub fn text_images_patch(preloaded: &[u8], desired: &TextImages) -> Result<Option<Vec<u8>>> {
    let current = text_images(preloaded)?;
    let mut replaced = Vec::new();
    let mut encoded = Vec::new();
    for (number, current, desired) in [
        (2, current.use_rich_chat_input, desired.use_rich_chat_input),
        (3, current.use_thread_sidebar, desired.use_thread_sidebar),
        (
            7,
            current.view_image_descriptions,
            desired.view_image_descriptions,
        ),
        (
            9,
            current.inline_attachment_media,
            desired.inline_attachment_media,
        ),
        (10, current.inline_embed_media, desired.inline_embed_media),
        (11, current.gif_auto_play, desired.gif_auto_play),
        (12, current.render_embeds, desired.render_embeds),
        (13, current.render_reactions, desired.render_reactions),
        (14, current.animate_emoji, desired.animate_emoji),
        (
            17,
            current.message_display_compact,
            desired.message_display_compact,
        ),
        (21, current.convert_emoticons, desired.convert_emoticons),
        (
            28,
            current.include_stickers_in_autocomplete,
            desired.include_stickers_in_autocomplete,
        ),
        (
            36,
            current.is_cross_dm_search_enabled,
            desired.is_cross_dm_search_enabled,
        ),
        (
            39,
            current.include_game_mentions_in_autocomplete,
            desired.include_game_mentions_in_autocomplete,
        ),
        (
            41,
            current.display_compact_avatars,
            desired.display_compact_avatars,
        ),
    ] {
        if current != desired {
            replaced.push(number);
            bool_field(&mut encoded, number, desired);
        }
    }
    if current.animate_stickers != desired.animate_stickers {
        replaced.push(15);
        let mut wrapper = Vec::new();
        let wire = desired.animate_stickers.wire();
        if wire != 0 {
            push_uint(&mut wrapper, 1, wire);
        }
        push_bytes(&mut encoded, 15, &wrapper);
    }
    if current.render_spoilers != desired.render_spoilers {
        replaced.push(4);
        let mut wrapper = Vec::new();
        push_bytes(&mut wrapper, 1, desired.render_spoilers.wire().as_bytes());
        push_bytes(&mut encoded, 4, &wrapper);
    }
    if replaced.is_empty() {
        Ok(None)
    } else {
        rebuild(preloaded, TEXT_AND_IMAGES, &replaced, &encoded).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fixtures::{field, uint};

    #[test]
    fn defaults_and_values() {
        assert_eq!(text_images(&[]).unwrap(), TextImages::default());
        assert_eq!(text_images(&field(6, &[])).unwrap(), TextImages::default());
        let mut message = Vec::new();
        for number in [2, 3, 9, 10, 11, 12, 13, 14, 21, 39] {
            message.extend(field(number, &[]));
        }
        for number in [7, 17, 28, 36, 41] {
            message.extend(field(number, &uint(1, 1)));
        }
        message.extend(field(15, &uint(1, 2)));
        message.extend(field(4, &field(1, b"ALWAYS")));
        let settings = text_images(&field(6, &message)).unwrap();
        assert_eq!(
            settings,
            TextImages {
                use_rich_chat_input: false,
                use_thread_sidebar: false,
                render_spoilers: SpoilerMode::Always,
                view_image_descriptions: true,
                inline_attachment_media: false,
                inline_embed_media: false,
                gif_auto_play: false,
                render_embeds: false,
                render_reactions: false,
                animate_emoji: false,
                animate_stickers: StickerAnimation::Never,
                message_display_compact: true,
                convert_emoticons: false,
                include_stickers_in_autocomplete: true,
                is_cross_dm_search_enabled: true,
                include_game_mentions_in_autocomplete: false,
                display_compact_avatars: true,
            }
        );
        let patch = text_images_patch(&[], &settings).unwrap().unwrap();
        assert_eq!(text_images(&patch).unwrap(), settings);
        let reset = text_images_patch(&patch, &TextImages::default())
            .unwrap()
            .unwrap();
        assert_eq!(text_images(&reset).unwrap(), TextImages::default());
        for (wire, expected) in [
            ("ON_CLICK", SpoilerMode::OnClick),
            ("IF_MODERATOR", SpoilerMode::IfModerator),
            ("future", SpoilerMode::OnClick),
        ] {
            assert_eq!(
                text_images(&field(6, &field(4, &field(1, wire.as_bytes()))))
                    .unwrap()
                    .render_spoilers,
                expected
            );
        }
        for (wire, expected) in [
            (0, StickerAnimation::Always),
            (1, StickerAnimation::OnInteraction),
            (9, StickerAnimation::Always),
        ] {
            assert_eq!(
                text_images(&field(6, &field(15, &uint(1, wire))))
                    .unwrap()
                    .animate_stickers,
                expected
            );
        }
    }

    #[test]
    fn patches_only_changes_preserving_opaque_and_unchanged_wrappers() {
        let opaque = field(99, b"opaque");
        let wrapper = field(2, &[uint(1, 1), uint(9, 42)].concat());
        let spoiler = field(4, &field(1, b"future"));
        let base = [
            field(1, &uint(3, 42)),
            field(
                6,
                &[opaque.clone(), wrapper.clone(), spoiler.clone()].concat(),
            ),
        ]
        .concat();
        let mut desired = text_images(&base).unwrap();
        assert_eq!(text_images_patch(&base, &desired).unwrap(), None);
        desired.gif_auto_play = false;
        let patch = text_images_patch(&base, &desired).unwrap().unwrap();
        assert_eq!(
            patch,
            field(6, &[opaque, wrapper, spoiler, field(11, &[])].concat())
        );
        assert_eq!(
            text_images(&merge_partial(&base, &patch).unwrap()).unwrap(),
            desired
        );
        desired.render_spoilers = SpoilerMode::IfModerator;
        assert_eq!(
            text_images(&text_images_patch(&base, &desired).unwrap().unwrap()).unwrap(),
            desired
        );
    }

    #[test]
    fn rebase_keeps_remote_edits_and_prefers_local_conflicts() {
        let base = TextImages::default();
        let edited = TextImages {
            gif_auto_play: false,
            render_spoilers: SpoilerMode::Always,
            ..base.clone()
        };
        let onto = TextImages {
            animate_emoji: false,
            render_spoilers: SpoilerMode::IfModerator,
            ..base.clone()
        };
        let merged = edited.rebase(&base, &onto);
        assert!(!merged.gif_auto_play);
        assert!(!merged.animate_emoji);
        assert_eq!(merged.render_spoilers, SpoilerMode::Always);
        assert_eq!(base.rebase(&base, &onto), onto);
    }

    #[test]
    fn malformed_fields_fail_closed() {
        for bytes in [
            field(6, &[18, 9]),
            field(6, &uint(11, 1)),
            field(6, &field(4, &field(1, &[255]))),
        ] {
            assert!(text_images(&bytes).is_err());
            assert!(text_images_patch(&bytes, &TextImages::default()).is_err());
        }
    }
}
