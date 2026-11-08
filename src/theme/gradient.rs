use gpui::{
    Hsla, IntoElement, ParentElement, Rems, Rgba, Styled, div, hsla, linear_color_stop,
    linear_gradient, rgb,
};

use super::Palette;

pub struct Preset {
    pub id: u32,
    pub name: &'static str,
    pub light: bool,
    pub angle: f32,
    pub stops: &'static [(u32, f32)],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: 0,
        name: "Mint Apple",
        light: true,
        angle: 180.0,
        stops: &[(0x56B69F, 6.15), (0x63BC61, 48.7), (0x9ECA67, 93.07)],
    },
    Preset {
        id: 1,
        name: "Citrus Sherbert",
        light: true,
        angle: 180.0,
        stops: &[(0xF3B336, 31.1), (0xEE8558, 67.09)],
    },
    Preset {
        id: 2,
        name: "Retro Raincloud",
        light: true,
        angle: 148.71,
        stops: &[
            (0x3A7CA1, 5.64),
            (0x7F7EB9, 26.38),
            (0x7F7EB9, 49.92),
            (0x3A7CA1, 73.12),
        ],
    },
    Preset {
        id: 3,
        name: "Hanami",
        light: true,
        angle: 38.08,
        stops: &[(0xEFAAB3, 3.56), (0xEFD696, 35.49), (0xA6DAA2, 68.78)],
    },
    Preset {
        id: 4,
        name: "Sunrise",
        light: true,
        angle: 154.19,
        stops: &[(0x9F4175, 8.62), (0xC49064, 48.07), (0xA6953D, 76.04)],
    },
    Preset {
        id: 5,
        name: "Cotton Candy",
        light: true,
        angle: 180.14,
        stops: &[(0xF4ABB8, 8.5), (0xB1C2FC, 94.28)],
    },
    Preset {
        id: 6,
        name: "LoFi Vibes",
        light: true,
        angle: 179.52,
        stops: &[
            (0xA4C0F7, 7.08),
            (0xA9E4E8, 34.94),
            (0xB0E2B8, 65.12),
            (0xCFDFA2, 96.23),
        ],
    },
    Preset {
        id: 7,
        name: "Desert Khaki",
        light: true,
        angle: 38.99,
        stops: &[(0xE7DBD0, 12.92), (0xDFD0B2, 32.92), (0xE0D6A3, 52.11)],
    },
    Preset {
        id: 8,
        name: "Sunset",
        light: false,
        angle: 141.68,
        stops: &[(0x48288C, 27.57), (0xDB7F4B, 71.25)],
    },
    Preset {
        id: 9,
        name: "Chroma Glow",
        light: false,
        angle: 128.92,
        stops: &[
            (0x0EB5BF, 3.94),
            (0x4C0CE0, 26.1),
            (0xA308A7, 39.82),
            (0x9A53FF, 56.89),
            (0x218BE0, 76.45),
        ],
    },
    Preset {
        id: 10,
        name: "Forest",
        light: false,
        angle: 162.27,
        stops: &[
            (0x142215, 11.2),
            (0x2D4D39, 29.93),
            (0x454C32, 48.64),
            (0x5A7C58, 67.85),
            (0xA98E4B, 83.54),
        ],
    },
    Preset {
        id: 11,
        name: "Crimson Moon",
        light: false,
        angle: 64.92,
        stops: &[(0x950909, 16.17), (0x000000, 72.0)],
    },
    Preset {
        id: 12,
        name: "Midnight Blurple",
        light: false,
        angle: 48.17,
        stops: &[(0x5348CA, 11.21), (0x140730, 61.92)],
    },
    Preset {
        id: 13,
        name: "Mars",
        light: false,
        angle: 170.82,
        stops: &[(0x895240, 14.61), (0x8F4343, 74.62)],
    },
    Preset {
        id: 14,
        name: "Dusk",
        light: false,
        angle: 180.0,
        stops: &[(0x665069, 12.84), (0x91A3D1, 85.99)],
    },
    Preset {
        id: 15,
        name: "Under the Sea",
        light: false,
        angle: 179.14,
        stops: &[(0x647962, 1.91), (0x588575, 48.99), (0x6A8482, 96.35)],
    },
    Preset {
        id: 17,
        name: "Retro Storm",
        light: false,
        angle: 148.71,
        stops: &[
            (0x3A7CA1, 5.64),
            (0x58579A, 26.38),
            (0x58579A, 49.92),
            (0x3A7CA1, 73.12),
        ],
    },
    Preset {
        id: 18,
        name: "Neon Nights",
        light: false,
        angle: 180.0,
        stops: &[(0x01A89E, 0.0), (0x7D60BA, 50.0), (0xB43898, 100.0)],
    },
    Preset {
        id: 20,
        name: "Strawberry Lemonade",
        light: false,
        angle: 161.03,
        stops: &[(0xAF1A6C, 18.79), (0xC26B20, 49.76), (0xE7A525, 80.72)],
    },
    Preset {
        id: 21,
        name: "Aurora",
        light: false,
        angle: 239.16,
        stops: &[
            (0x062053, 10.39),
            (0x191FBB, 26.87),
            (0x13929A, 48.31),
            (0x218573, 64.98),
            (0x051A81, 92.5),
        ],
    },
    Preset {
        id: 19,
        name: "Sepia",
        light: false,
        angle: 69.98,
        stops: &[(0x857664, 14.14), (0x5B4421, 60.35)],
    },
    Preset {
        id: 22,
        name: "Blurple Twilight",
        light: false,
        angle: 47.61,
        stops: &[(0x2C3FE7, 11.18), (0x261D83, 64.54)],
    },
];

pub fn preset(id: u32) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
    Lower,
    Low,
    Chat,
    High,
    Higher,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub angle: f32,
    pub stops: Vec<(u32, f32)>,
    light: bool,
    base: u32,
    base_weight: f32,
    text: u32,
    text_weight: f32,
    opacities: [f32; 5],
}

impl Gradient {
    pub fn from_preset(preset: &Preset) -> Self {
        let first = preset.stops[0].0;
        let last = preset.stops[preset.stops.len() - 1].0;
        let (lighter, darker) = if luminance(first) > luminance(last) {
            (first, last)
        } else {
            (last, first)
        };
        let darker = if hue(darker) == 0. { lighter } else { darker };
        let (base, text) = if preset.light {
            let text = with_saturation(darker, 1.);
            let lightness = if (200. ..=300.).contains(&hue(text)) {
                0.25
            } else {
                0.15
            };
            (with_hsl(lighter, 1., 0.9), with_lightness(text, lightness))
        } else {
            let text = with_saturation(lighter, 1.);
            let lightness = if (200. ..=300.).contains(&hue(text)) {
                0.8
            } else {
                0.95
            };
            (with_hsl(darker, 1., 0.05), with_lightness(text, lightness))
        };
        Self {
            angle: preset.angle,
            stops: preset.stops.to_vec(),
            light: preset.light,
            base,
            base_weight: mix_weight(50.),
            text,
            text_weight: mix_weight(38.),
            opacities: if preset.light {
                [0.8, 0.75, 0.85, 0.7, 0.65]
            } else {
                [0.85, 0.8, 0.8, 0.75, 0.7]
            },
        }
    }

    pub fn custom(colors: &[u32], angle: f32, base_mix: f32, light: bool) -> Option<Self> {
        let (&first, rest) = colors.split_first()?;
        let darkest = rest.iter().fold(first, |darkest, &color| {
            if luminance(color) < luminance(darkest) {
                color
            } else {
                darkest
            }
        });
        let lightest = rest.iter().fold(first, |lightest, &color| {
            if luminance(color) > luminance(lightest) {
                color
            } else {
                lightest
            }
        });
        let strength = 100. - base_mix;
        let mix = (30. + 0.7 * strength) / 100.;
        let count = colors.len().max(2) - 1;
        let mut stops: Vec<(u32, f32)> = colors
            .iter()
            .enumerate()
            .map(|(index, &color)| (color, index as f32 * 100. / count as f32))
            .collect();
        if stops.len() == 1 {
            stops.push((first, 100.));
        }
        let (base, base_amount, text, text_amount, opacities) = if light {
            (
                with_hsl(lightest, 1., 0.9),
                30. - 0.2 * base_mix,
                with_lightness(lightest, 0.05 * strength / 100.),
                40.,
                [
                    0.48 + mix * 0.4 * 0.8,
                    0.45 + mix * 0.4 * 0.75,
                    0.85 * (0.6 + mix * 0.4),
                    0.42 + mix * 0.4 * 0.7,
                    0.325 + mix * 0.5 * 0.65,
                ],
            )
        } else {
            (
                with_hsl(darkest, 1., 0.05),
                60. + 0.4 * strength,
                with_lightness(darkest, (100. - 0.1 * strength) / 100.),
                30.,
                [
                    0.51 + mix * 0.4 * 0.85,
                    0.48 + mix * 0.4 * 0.8,
                    0.48 + mix * 0.4 * 0.8,
                    0.45 + mix * 0.4 * 0.75,
                    0.42 + mix * 0.4 * 0.7,
                ],
            )
        };
        Some(Self {
            angle,
            stops,
            light,
            base,
            base_weight: mix_weight(base_amount),
            text,
            text_weight: mix_weight(text_amount),
            opacities,
        })
    }

    pub fn layer_over(&self, layer: Layer, under: Option<Layer>) -> Hsla {
        let target = self.opacities[layer as usize].clamp(0., 1.);
        let below = under.map_or(0., |under| self.opacities[under as usize].clamp(0., 0.999));
        let alpha = ((target - below) / (1. - below)).max(0.);
        hsla(0., 0., if self.light { 1. } else { 0. }, alpha)
    }

    pub fn tint(&self, palette: &Palette) -> Palette {
        palette.map(|field, value| match field {
            "bright"
            | "control_thumb"
            | "spoiler_revealed_bg"
            | "timestamp_bg"
            | "menu_border"
            | "shadow"
            | "tooltip_shadow"
            | "control_shadow"
            | "recessed"
            | "connection_tile_bg"
            | "connection_tile_text"
            | "upload_sheet"
            | "upload_fold"
            | "accent_border"
            | "text_selection"
            | "mention_bg"
            | "reaction_selected_bg"
            | "reply_bg"
            | "mention_row_bg"
            | "mention_row_hover"
            | "jump_highlight" => value,
            "text" | "strong" | "muted" | "faint" | "link" | "text_secondary" | "mention_text" => {
                mix_oklab(value, self.text, self.text_weight)
            }
            _ if value > 0xFF_FFFF => {
                (mix_oklab(value >> 8, self.base, self.base_weight) << 8) | (value & 0xFF)
            }
            _ => mix_oklab(value, self.base, self.base_weight),
        })
    }

    /// Full-size layers that together paint the gradient; GPUI gradients take two stops,
    /// so each later stop fades in over the previous one.
    pub fn backdrop(&self) -> impl IntoElement + use<> {
        self.layers(None)
    }

    /// The gradient with rounded corners; GPUI doesn't clip children to rounded corners.
    pub fn tile(&self, radius: Rems) -> impl IntoElement + use<> {
        self.layers(Some(radius))
    }

    fn layers(&self, radius: Option<Rems>) -> impl IntoElement + use<> {
        let layer = |background| {
            let layer = div().absolute().inset_0().bg(background);
            match radius {
                Some(radius) => layer.rounded(radius),
                None => layer,
            }
            .into_any_element()
        };
        let first = self.stops[0];
        let second = self.stops.get(1).copied().unwrap_or(first);
        let mut layers = vec![layer(linear_gradient(
            self.angle,
            linear_color_stop(rgb(first.0), first.1 / 100.),
            linear_color_stop(rgb(second.0), second.1 / 100.),
        ))];
        for pair in self.stops.windows(2).skip(1) {
            let (from, to) = (pair[0], pair[1]);
            let color = rgb(to.0);
            layers.push(layer(linear_gradient(
                self.angle,
                linear_color_stop(
                    Hsla {
                        a: 0.,
                        ..color.into()
                    },
                    from.1 / 100.,
                ),
                linear_color_stop(color, to.1 / 100.),
            )));
        }
        div().absolute().inset_0().children(layers)
    }
}

/// `color-mix(in oklab, token 100%, other amount%)` normalizes the two percentages.
fn mix_weight(amount: f32) -> f32 {
    amount / (100. + amount)
}

fn channels(color: u32) -> [f32; 3] {
    [
        ((color >> 16) & 0xFF) as f32 / 255.,
        ((color >> 8) & 0xFF) as f32 / 255.,
        (color & 0xFF) as f32 / 255.,
    ]
}

fn pack([r, g, b]: [f32; 3]) -> u32 {
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    (channel(r) << 16) | (channel(g) << 8) | channel(b)
}

fn to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}

fn to_oklab(color: u32) -> [f32; 3] {
    let [r, g, b] = channels(color).map(to_linear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([l, a, b]: [f32; 3]) -> u32 {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    pack(
        [
            4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
            -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
            -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
        ]
        .map(from_linear),
    )
}

fn mix_oklab(color: u32, other: u32, weight: f32) -> u32 {
    let (a, b) = (to_oklab(color), to_oklab(other));
    from_oklab([0, 1, 2].map(|index| a[index] + (b[index] - a[index]) * weight))
}

fn luminance(color: u32) -> f32 {
    let [r, g, b] = channels(color).map(to_linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn to_hsl(color: u32) -> [f32; 3] {
    let [r, g, b] = channels(color);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.;
    let delta = max - min;
    if delta == 0. {
        return [0., 0., lightness];
    }
    let saturation = delta / (1. - (2. * lightness - 1.).abs());
    let hue = if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    [hue, saturation, lightness]
}

fn from_hsl([hue, saturation, lightness]: [f32; 3]) -> u32 {
    let color: Rgba = hsla(hue / 360., saturation, lightness, 1.).into();
    pack([color.r, color.g, color.b])
}

fn hue(color: u32) -> f32 {
    to_hsl(color)[0]
}

fn with_hsl(color: u32, saturation: f32, lightness: f32) -> u32 {
    from_hsl([hue(color), saturation, lightness])
}

fn with_saturation(color: u32, saturation: f32) -> u32 {
    let [hue, _, lightness] = to_hsl(color);
    from_hsl([hue, saturation, lightness])
}

fn with_lightness(color: u32, lightness: f32) -> u32 {
    let [hue, saturation, _] = to_hsl(color);
    from_hsl([hue, saturation, lightness])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnight_blurple_tint_matches_the_measured_base_and_text() {
        let gradient = Gradient::from_preset(preset(12).unwrap());
        assert_eq!(gradient.base, 0x08001A);
        assert_eq!(gradient.text, 0xA299FF);
        assert!(!gradient.light);
        assert_eq!(gradient.layer_over(Layer::Higher, None).a, 0.7);
        assert_eq!(
            gradient.layer_over(Layer::Higher, Some(Layer::Higher)).a,
            0.
        );
    }

    #[test]
    fn oklab_round_trips_and_mixes_toward_the_other_color() {
        for color in [0x000000, 0xFFFFFF, 0x5348CA, 0x1A1A1E] {
            assert_eq!(from_oklab(to_oklab(color)), color);
        }
        assert_eq!(mix_oklab(0x123456, 0xABCDEF, 0.), 0x123456);
        assert_eq!(mix_oklab(0x123456, 0xABCDEF, 1.), 0xABCDEF);
    }

    #[test]
    fn custom_themes_spread_colors_evenly() {
        let gradient = Gradient::custom(&[0x112233, 0x445566, 0x778899], 90., 74., false).unwrap();
        assert_eq!(
            gradient.stops,
            vec![(0x112233, 0.), (0x445566, 50.), (0x778899, 100.)]
        );
        assert!(Gradient::custom(&[], 0., 74., false).is_none());
    }
}
