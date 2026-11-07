use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use super::{ArtSlot, ProfileCard, ProfilePalette, Stack, StackHover, Tab};
use crate::{
    app::{DISCORD_EPOCH_MS, DiscordApp, message_body::label},
    discord::model::{
        Game, IdentityProfile, ListenedTrack, OutboxEntry, OutboxKind, OutboxTrait, PersonalImage,
        PersonalSection, PublicApplication, SkuPrice, StatValue, WidgetAsset, WidgetData,
        WidgetGame, WidgetImage, WidgetText, Wishlist, WishlistItem,
    },
    media::animated,
    media::images::{self, ImageSpec},
    store::GameActivity,
    theme::{self, rpx},
};

mod actions;
mod activity;
mod art;
mod stack;
#[cfg(test)]
mod tests;
mod voice;
mod widgets;
mod wishlist;
use super::{widget_application_ids, widget_game_ids};
pub(super) use activity::{activity, listening, popout_playing};
pub(crate) use art::presence_art;
pub(super) use stack::{card_stack, hover_card};
pub(super) use voice::popout_voice;
pub(super) use widgets::{
    board_panel, popout_widget, prefetch_images, tag_icon, tag_label, widget_element,
};
pub(super) use wishlist::wishlist_panel;

const COVER_WIDTH: f32 = 88.;
const COVER_HEIGHT: f32 = COVER_WIDTH * 4. / 3.;
const COVER_GAP: f32 = 16.;
const ACTIVITY_ART: f32 = 64.;
const ACTIVITY_RADIUS: f32 = 8.;
const CURRENT_ART: f32 = 100.;
const CURRENT_BADGE: f32 = 32.;
const POPOUT_ART: f32 = 60.;
const POPOUT_BADGE: f32 = 24.;
const POPOUT_HERO: f32 = 65.;
const COLLECTION_ICON: f32 = 24.;
const POPOUT_COVER_WIDTH: f32 = 104.;
const POPOUT_COVER_HEIGHT: f32 = 42.;
const COLLECTION_SHOWN: usize = 3;
const HERO_WIDTH: f32 = 240.;
const HERO_HEIGHT: f32 = 160.;
const TILE_IMAGE: f32 = 48.;
const CONTAINED_IMAGE: f32 = 96.;
const PERSONAL_COVER: f32 = 480.;
/// Share of the hero's width that fades in from transparent at its left edge.
const HERO_FADE: f32 = 0.6;
/// Detailed game lists collapse to two entries; cover grids show the full collection.
const COLLAPSED_GAMES: usize = 2;
/// Two rows of four, as Discord collapses its cover grids.
const COLLAPSED_COVERS: usize = 8;
const COVERS_PER_ROW: usize = 4;
/// How long a widget's "Show more"/"Show less" takes to grow or shrink.
pub(super) const WIDGET_TOGGLE: Duration = Duration::from_millis(250);
const WISHLIST_GAP: f32 = 16.;
const WISHLIST_BADGE: f32 = 26.;
const WISHLIST_AVATAR: f32 = 64.;
/// Discord draws avatar decorations 1.2x the avatar's size.
const DECORATION_SCALE: f32 = 1.2;
const SHOP_MENU_ENTER: Duration = Duration::from_millis(150);
const MARATHON_SECONDS: u64 = 3 * 60 * 60;

pub(super) fn has_board(card: &ProfileCard) -> bool {
    card.profile.as_ref().is_some_and(|profile| {
        profile
            .widgets
            .iter()
            .any(|widget| !matches!(widget.data, WidgetData::Unknown))
    })
}

pub(super) fn has_wishlist(card: &ProfileCard) -> bool {
    card.wishlist
        .as_ref()
        .is_some_and(|wishlist| !wishlist.items.is_empty())
}
