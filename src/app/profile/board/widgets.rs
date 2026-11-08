mod application;
mod games;
mod personal;
mod popout_widgets;

use application::application_widget;
pub(in crate::app::profile) use application::{tag_icon, tag_label};
pub(in crate::app::profile) use games::prefetch_images;
use games::{covers_widget, games_widget};
use personal::personal_widget;
#[cfg(test)]
pub(super) use personal::stat_name;
pub(super) use personal::{stat_value, widget_image_art};
pub(in crate::app::profile) use popout_widgets::popout_widget;

use super::art::{
    activity_art, app_asset, current_art, media, personal_art, presence_art, widget_art,
};
use super::*;
use crate::app::profile::popout::markdown;

pub(in crate::app::profile) fn board_panel(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> AnyElement {
    let widgets = card.profile.iter().flat_map(|profile| &profile.widgets);
    div()
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .children(widgets.filter_map(|widget| widget_element(app, card, widget, false, cx)))
        .into_any_element()
}

pub(in crate::app::profile) fn widget_element(
    app: &DiscordApp,
    card: &ProfileCard,
    widget: &crate::discord::model::ProfileWidget,
    editing: bool,
    cx: &App,
) -> Option<AnyElement> {
    let palette = card.palette();
    let (title, games) = match &widget.data {
        WidgetData::Application { application_id } => {
            return Some(application_widget(app, card, *application_id, palette, cx));
        }
        WidgetData::FavoriteGames { games } => ("Favorite Game", games),
        WidgetData::CurrentGames { games } => ("Games in Rotation", games),
        WidgetData::PlayedGames { games } => {
            return (!games.is_empty())
                .then(|| covers_widget(app, card, widget.id, "Games I Like", games, palette, cx));
        }
        WidgetData::WantToPlayGames { games } => {
            return (!games.is_empty())
                .then(|| covers_widget(app, card, widget.id, "Want to Play", games, palette, cx));
        }
        WidgetData::Personal { header, sections } => {
            return Some(personal_widget(
                card,
                widget.id,
                header.as_deref(),
                sections,
                palette,
                cx,
            ));
        }
        WidgetData::Unknown => return None,
    };
    let featured = matches!(widget.data, WidgetData::FavoriteGames { .. });
    (!games.is_empty()).then(|| {
        games_widget(
            app, card, widget.id, title, games, featured, editing, palette, cx,
        )
    })
}

fn widget_card(palette: ProfilePalette) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(rpx(12.))
        .bg(palette.panel)
        .overflow_hidden()
}
