use gpui::{App, Global};

/// `letter_spacing_em` is Discord's CSS tracking for this face. The face's
/// binary weight is intentionally left alone; callers choose the shaping
/// weight for their context.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DisplayFont {
    pub family: &'static str,
    pub letter_spacing_em: f32,
}

#[derive(Clone, Copy)]
struct Preset {
    id: u32,
    family: &'static str,
    letter_spacing_em: f32,
}

// Keyed by Discord's protocol ids; order is mirrored by `FONTS` and the availability
// bitmask. Absent, default (11), deprecated (1, 2, 5, 9), and unknown ids have no preset,
// so callers inherit the UI font.
const PRESETS: [Preset; 11] = [
    Preset {
        id: 3,
        family: "Sakura",
        letter_spacing_em: 0.04,
    },
    Preset {
        id: 4,
        family: "Jellybean",
        letter_spacing_em: 0.0,
    },
    Preset {
        id: 6,
        family: "Modern",
        letter_spacing_em: 0.01,
    },
    Preset {
        id: 7,
        family: "Medieval",
        letter_spacing_em: 0.02,
    },
    Preset {
        id: 8,
        family: "8Bit",
        letter_spacing_em: 0.02,
    },
    Preset {
        id: 10,
        family: "Vampyre",
        letter_spacing_em: 0.01,
    },
    Preset {
        id: 12,
        family: "Tempo",
        letter_spacing_em: 0.03,
    },
    Preset {
        id: 13,
        family: "Monkey Bars",
        letter_spacing_em: 0.0,
    },
    Preset {
        id: 14,
        family: "Mainframe",
        letter_spacing_em: 0.03,
    },
    Preset {
        id: 15,
        family: "Headbang",
        letter_spacing_em: 0.0,
    },
    Preset {
        id: 16,
        family: "Journal",
        letter_spacing_em: 0.0,
    },
];

// SFNT conversions of the supplied WOFF2 builds; see assets/fonts/display/NOTICE.md.
pub(super) const FONTS: [&[u8]; 11] = [
    include_bytes!("../../assets/fonts/display/Sakura.ttf"),
    include_bytes!("../../assets/fonts/display/Jellybean.ttf"),
    include_bytes!("../../assets/fonts/display/Modern.otf"),
    include_bytes!("../../assets/fonts/display/Medieval.otf"),
    include_bytes!("../../assets/fonts/display/8Bit.ttf"),
    include_bytes!("../../assets/fonts/display/Vampyre.ttf"),
    include_bytes!("../../assets/fonts/display/Tempo.ttf"),
    include_bytes!("../../assets/fonts/display/Monkey-Bars.ttf"),
    include_bytes!("../../assets/fonts/display/Mainframe.ttf"),
    include_bytes!("../../assets/fonts/display/Headbang.ttf"),
    include_bytes!("../../assets/fonts/display/Journal.ttf"),
];

#[derive(Default)]
struct AvailableDisplayFonts(u16);

impl Global for AvailableDisplayFonts {}

/// Runs once at startup; render paths only read the cached bitmask.
pub(super) fn cache_availability(cx: &mut App, font_names: &[String]) {
    let mut available = 0u16;
    for (index, preset) in PRESETS.iter().enumerate() {
        if font_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(preset.family))
        {
            available |= 1u16 << index;
        } else {
            log::warn!(
                "Bundled display font {} is unavailable; falling back to the UI font",
                preset.family
            );
        }
    }
    cx.set_global(AvailableDisplayFonts(available));
}

pub(crate) fn display_font(id: Option<u32>, cx: &App) -> Option<DisplayFont> {
    let index = preset_index(id?)?;
    let available = cx.try_global::<AvailableDisplayFonts>()?;
    if available.0 & (1u16 << index) == 0 {
        return None;
    }
    let preset = &PRESETS[index];
    Some(DisplayFont {
        family: preset.family,
        letter_spacing_em: preset.letter_spacing_em,
    })
}

fn preset_index(id: u32) -> Option<usize> {
    PRESETS.iter().position(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_protocol_ids_tracking_and_families() {
        let expected: [(u32, &str, f32); 11] = [
            (3, "Sakura", 0.04),
            (4, "Jellybean", 0.0),
            (6, "Modern", 0.01),
            (7, "Medieval", 0.02),
            (8, "8Bit", 0.02),
            (10, "Vampyre", 0.01),
            (12, "Tempo", 0.03),
            (13, "Monkey Bars", 0.0),
            (14, "Mainframe", 0.03),
            (15, "Headbang", 0.0),
            (16, "Journal", 0.0),
        ];

        assert_eq!(PRESETS.len(), expected.len());
        assert_eq!(FONTS.len(), PRESETS.len());
        for (id, family, letter_spacing_em) in expected {
            let index = preset_index(id).unwrap_or_else(|| panic!("missing display font id {id}"));
            let preset = &PRESETS[index];
            assert_eq!(preset.family, family);
            assert_eq!(
                preset.letter_spacing_em.to_bits(),
                letter_spacing_em.to_bits()
            );
        }
    }

    #[test]
    fn default_deprecated_unknown_and_missing_ids_fall_back() {
        let fallback_ids: [Option<u32>; 9] = [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(5),
            Some(9),
            Some(11),
            Some(17),
            Some(u32::MAX),
        ];
        for id in fallback_ids {
            assert!(
                id.and_then(preset_index).is_none(),
                "{id:?} should inherit the UI font"
            );
        }
    }

    #[test]
    fn protocol_ids_are_unique() {
        for (index, preset) in PRESETS.iter().enumerate() {
            assert!(
                PRESETS[index + 1..]
                    .iter()
                    .all(|other| other.id != preset.id),
                "duplicate display font id {}",
                preset.id
            );
        }
    }

    #[gpui::test]
    fn availability_lookup_handles_partial_and_missing_catalogs(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(display_font(Some(14), cx), None);
            cache_availability(cx, &["mainframe".to_owned(), "Tempo".to_owned()]);
            assert_eq!(
                display_font(Some(14), cx),
                Some(DisplayFont {
                    family: "Mainframe",
                    letter_spacing_em: 0.03
                })
            );
            assert_eq!(display_font(Some(12), cx).unwrap().family, "Tempo");
            assert_eq!(display_font(Some(3), cx), None);
            assert_eq!(display_font(Some(11), cx), None);
            assert_eq!(display_font(None, cx), None);
            cache_availability(cx, &[]);
            assert_eq!(display_font(Some(14), cx), None);
        });
    }
}
