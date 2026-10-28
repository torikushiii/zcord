use gpui::*;
use palette::{FromColor, Hsl, Lab, Mix, Oklab, Srgb};

use super::THEME_OVERLAY;
use super::loading::guild_profile;
use crate::{discord::model, theme};

#[derive(Clone, Copy)]
pub(super) struct ProfileTheme {
    pub(super) primary: u32,
    pub(super) secondary: u32,
}

impl ProfileTheme {
    pub(super) fn needs_avatar(profile: Option<&model::Profile>) -> bool {
        profile.is_some_and(|profile| profile.premium_type == Some(2))
            && Self::of(profile).is_none()
    }

    pub(super) fn resolve(profile: Option<&model::Profile>, avatar: Option<Self>) -> Option<Self> {
        Self::of(profile).or_else(|| avatar.filter(|_| Self::needs_avatar(profile)))
    }

    pub(super) fn of(profile: Option<&model::Profile>) -> Option<Self> {
        let profile = profile?;
        let colors = |profile: &model::UserProfile| {
            let colors = profile.theme_colors.as_deref()?;
            Some((*colors.first()?, *colors.get(1)?))
        };
        let (primary, secondary) = guild_profile(profile)
            .and_then(colors)
            .or_else(|| profile.user_profile.as_ref().and_then(colors))?;
        Some(Self { primary, secondary })
    }

    pub(super) fn outer(self) -> Background {
        linear_gradient(
            180.,
            linear_color_stop(rgb(self.primary), 0.),
            linear_color_stop(rgb(self.secondary), 1.),
        )
    }

    pub(super) fn surface(self) -> u32 {
        self.surface_color(self.primary)
    }

    pub(super) fn is_light(self) -> bool {
        // Discord selects the profile mode from the primary color's perceived brightness.
        let channel = |shift: u32| ((self.primary >> shift) & 0xFF) as f64;
        0.299 * channel(16) + 0.587 * channel(8) + 0.114 * channel(0) >= 127.5
    }

    pub(super) fn overlay(self) -> Hsla {
        hsla(0., 0., if self.is_light() { 1. } else { 0. }, THEME_OVERLAY)
    }

    pub(super) fn surface_color(self, color: u32) -> u32 {
        blend(
            color,
            if self.is_light() { 0xFFFFFF } else { 0 },
            THEME_OVERLAY,
        )
    }

    pub(super) fn modal_background(self) -> Background {
        let to_lab = |color: u32| -> Lab<_, f64> {
            let channel = |shift: u32| ((color >> shift) & 0xFF) as f64 / 255.;
            Lab::from_color(Srgb::new(channel(16), channel(8), channel(0)))
        };
        // The dialog uses a Lab midpoint with constrained lightness, not the card gradient.
        let midpoint: Srgb<f64> =
            Srgb::from_color(to_lab(self.primary).mix(to_lab(self.secondary), 0.5));
        let mut background = palette::Hsl::from_color(midpoint);
        let lightness = (background.lightness * 100.).round() / 100.;
        background.lightness = if self.is_light() {
            lightness.clamp(0.8, 1.)
        } else {
            lightness.clamp(0., 0.1)
        };
        let background: Srgb<u8> = Srgb::from_color(background).into_format();
        rgb((u32::from(background.red) << 16)
            | (u32::from(background.green) << 8)
            | u32::from(background.blue))
        .into()
    }
}

pub(super) fn blend(color: u32, overlay: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((color >> shift) & 0xFF) as f32;
        let overlay = ((overlay >> shift) & 0xFF) as f32;
        (base * (1. - amount) + overlay * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xFF) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

pub(super) fn contrast(foreground: u32, background: u32) -> f32 {
    let foreground = luminance(foreground);
    let background = luminance(background);
    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
}

#[derive(Clone, Copy)]
pub(super) struct ProfilePalette {
    pub(super) light: bool,
    pub(super) strong: u32,
    pub(super) text: u32,
    pub(super) muted: u32,
    pub(super) faint: u32,
    pub(super) panel: Hsla,
    /// Filled buttons inside a panel (Play, Get on Steam, Browse the Shop).
    pub(super) button: Hsla,
    /// Strips that sit a step above a panel, like the activity accessory footer.
    pub(super) raised: Hsla,
    pub(super) border: Hsla,
    pub(super) hover: Hsla,
    pub(super) status_surface: u32,
    pub(super) status_border: Hsla,
    pub(super) status_text: Hsla,
}

impl ProfilePalette {
    pub(super) fn of(theme: Option<ProfileTheme>) -> Self {
        let light = theme.is_some_and(ProfileTheme::is_light);
        let to_srgb = |color: u32| -> Srgb<f32> {
            Srgb::new(
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
            )
            .into_format()
        };
        let status_tint = theme.map(|theme| {
            let source = if light {
                theme.secondary
            } else {
                theme.primary
            };
            let mut tint = Hsl::from_color(to_srgb(source));
            tint.saturation = 1.;
            tint.lightness = if light { 0.9 } else { 0.05 };
            let tint: Srgb<u8> = Srgb::from_color(tint).into_format();
            Oklab::from_color(tint.into_format::<f32>())
        });
        let status_color = |color: u32| {
            let Some(tint) = status_tint else {
                return color;
            };
            // Discord mixes the original token at 100% with its profile tint at 50%.
            let mixed: Srgb<u8> =
                Srgb::from_color(Oklab::from_color(to_srgb(color)).mix(tint, 1. / 3.))
                    .into_format();
            (u32::from(mixed.red) << 16) | (u32::from(mixed.green) << 8) | u32::from(mixed.blue)
        };
        let status_border = status_color(if light { 0x9696A0 } else { 0x97979F });
        let status_border_alpha = if light {
            0x33
        } else if theme.is_some() {
            0x70
        } else {
            0x29
        };
        let mut palette = Self {
            light,
            strong: if light { 0x18141F } else { theme::STRONG },
            text: if light { 0x2E2938 } else { theme::TEXT },
            muted: if light { 0x51495E } else { theme::MUTED },
            faint: if light { 0x62596F } else { theme::FAINT },
            // Discord's dark profile surfaces are a faintly blue white over the card: #18181a,
            // #2a2a2c and #2c2c30 on a black theme.
            panel: if light {
                hsla(0., 0., 1., 0.2)
            } else {
                rgba(0xEBEBFF1A).into()
            },
            button: if light {
                hsla(0., 0., 1., 0.35)
            } else {
                rgba(0xEBEBFF2E).into()
            },
            raised: if light {
                hsla(0., 0., 1., 0.3)
            } else {
                rgba(0xEBEBFF30).into()
            },
            border: hsla(0., 0., if light { 0. } else { 1. }, 0.08),
            hover: hsla(0., 0., if light { 0. } else { 1. }, 0.04),
            status_surface: status_color(if light { 0xFFFFFF } else { 0x17181B }),
            status_border: rgba((status_border << 8) | status_border_alpha).into(),
            status_text: match (light, theme.is_some()) {
                (false, false) => rgb(0xD4D5D8).into(),
                (true, false) => rgb(0x2E2E34).into(),
                (false, true) => rgba(0xFFFFFFF5).into(),
                (true, true) => rgba(0x000000F5).into(),
            },
        };
        if let Some(theme) = theme {
            let backgrounds = [
                theme.surface_color(theme.primary),
                theme.surface_color(theme.secondary),
            ];
            // Keep metadata readable even when the two custom stops have very different brightness.
            for color in [&mut palette.muted, &mut palette.faint] {
                if backgrounds
                    .iter()
                    .any(|background| contrast(*color, *background) < 4.5)
                {
                    *color = palette.text;
                }
            }
        }
        palette
    }
}

#[cfg(test)]
mod tests {
    use super::{ProfilePalette, ProfileTheme};

    #[test]
    fn status_bubble_uses_discord_surface_and_border_tokens() {
        let plain = ProfilePalette::of(None);
        assert_eq!(plain.status_surface, 0x17181B);
        assert!((plain.status_border.a - 41. / 255.).abs() < 0.001);

        // Reference colors extracted from Discord's dark and light profile bubble CSS.
        for (primary, expected, border_alpha) in [
            (0xB2563E, 0x191211, 112. / 255.),
            (0xD99DA3, 0xFFEEEF, 51. / 255.),
        ] {
            let palette = ProfilePalette::of(Some(ProfileTheme {
                primary,
                secondary: 0xD99DA3,
            }));
            for shift in [0, 8, 16] {
                let actual = (palette.status_surface >> shift) & 0xFF;
                let expected = (expected >> shift) & 0xFF;
                assert!(actual.abs_diff(expected) <= 1);
            }
            assert!((palette.status_border.a - border_alpha).abs() < 0.001);
            assert!((palette.status_text.a - 245. / 255.).abs() < 0.001);
        }
    }
}
