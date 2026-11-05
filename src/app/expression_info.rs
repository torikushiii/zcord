use gpui_component::scroll::ScrollableElement as _;
use std::collections::HashMap;

use gpui::{prelude::*, *};

use crate::{
    discord::model,
    media::{
        animated,
        images::{self, ImageSpec},
    },
    store::{EmojiAccess, Guild, Sticker},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

use super::{DiscordApp, Tokio};

mod stickers;
use stickers::source;

const CACHE_LIMIT: usize = 128;
const DEFAULT_EMOJI: &str = "A default emoji. You can use this emoji everywhere on Discord.";
const STICKER_COLUMNS: usize = 6;
const STICKER_CELL: f32 = 44.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expression {
    Emoji {
        id: u64,
        name: String,
        animated: bool,
    },
    Sticker(Sticker),
    /// A standard emoji, by its character sequence.
    Default(String),
}

impl Expression {
    fn key(&self) -> (bool, u64) {
        match self {
            Self::Emoji { id, .. } => (false, *id),
            Self::Sticker(sticker) => (true, sticker.id),
            Self::Default(sequence) => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                sequence.hash(&mut hasher);
                (false, hasher.finish())
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Default(sequence) => match emoji::shortcode_for_emoji(sequence) {
                Some(name) => format!(":{name}:"),
                None => sequence.clone(),
            },
            Self::Emoji { name, .. } if !name.is_empty() => format!(":{name}:"),
            Self::Emoji { .. } => "Custom Emoji".to_owned(),
            Self::Sticker(sticker) if !sticker.name.is_empty() => sticker.name.clone(),
            Self::Sticker(_) => "Sticker".to_owned(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverOwner {
    Emoji(u64, u32, usize),
    Sticker(u64, u64),
}

struct Hover {
    owner: HoverOwner,
    expression: Expression,
    bounds: Bounds<Pixels>,
}

struct Popout {
    expression: Expression,
    bounds: Bounds<Pixels>,
    source_sticker: Option<u64>,
    expanded: bool,
    hovered_sticker: Option<u64>,
    scroll: UniformListScrollHandle,
}

impl Popout {
    fn new(expression: Expression, bounds: Bounds<Pixels>) -> Self {
        let source_sticker = match &expression {
            Expression::Sticker(sticker) => Some(sticker.id),
            _ => None,
        };
        Self {
            expression,
            bounds,
            source_sticker,
            expanded: false,
            hovered_sticker: None,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn toggle_stickers(&mut self) -> bool {
        if self.source_sticker.is_none() {
            return false;
        }
        self.expanded = !self.expanded;
        self.hovered_sticker = None;
        true
    }

    fn inspect_sticker(&mut self, sticker: Sticker) -> bool {
        if self.source_sticker.is_none() {
            return false;
        }
        // Browsing previews never sends a sticker or changes the card's source server.
        self.expression = Expression::Sticker(sticker);
        true
    }
}

enum Load<T> {
    Pending,
    Ready(T),
    Failed,
}

fn reserve_load<T>(cache: &mut HashMap<u64, Load<T>>, id: u64) -> bool {
    if matches!(cache.get(&id), Some(Load::Pending | Load::Ready(_))) {
        return false;
    }
    if cache.len() >= CACHE_LIMIT {
        cache.retain(|_, load| matches!(load, Load::Pending));
        if cache.len() >= CACHE_LIMIT {
            return false;
        }
    }
    cache.insert(id, Load::Pending);
    true
}

#[derive(Default)]
pub(crate) struct State {
    hover: Option<Hover>,
    popout: Option<Popout>,
    stickers: HashMap<u64, Load<model::StickerDetails>>,
    guilds: HashMap<u64, Load<model::StickerGuild>>,
    catalogs: HashMap<u64, Load<model::StickerGuild>>,
    generation: u64,
}

impl State {
    fn sticker_guild(&self, id: u64) -> Option<&model::StickerGuild> {
        if let Some(Load::Ready(guild)) = self.guilds.get(&id) {
            return Some(guild);
        }
        self.guilds
            .values()
            .chain(self.catalogs.values())
            .find_map(|load| match load {
                Load::Ready(guild)
                    if guild
                        .stickers
                        .iter()
                        .flatten()
                        .any(|sticker| sticker.id == id) =>
                {
                    Some(guild)
                }
                _ => None,
            })
    }

    fn sticker_details(&self, id: u64) -> Option<&model::StickerDetails> {
        if let Some(Load::Ready(details)) = self.stickers.get(&id) {
            return Some(details);
        }
        self.sticker_guild(id)?
            .stickers
            .as_deref()?
            .iter()
            .find(|sticker| sticker.id == id)
    }

    fn set_hover(
        &mut self,
        owner: HoverOwner,
        expression: Expression,
        bounds: Bounds<Pixels>,
        hovered: bool,
    ) -> bool {
        if hovered {
            if self.popout.is_some()
                || self.hover.as_ref().is_some_and(|hover| {
                    hover.owner == owner && hover.expression == expression && hover.bounds == bounds
                })
            {
                return false;
            }
            self.hover = Some(Hover {
                owner,
                expression,
                bounds,
            });
        } else if self
            .hover
            .as_ref()
            .is_some_and(|hover| hover.owner == owner)
        {
            self.hover = None;
        } else {
            return false;
        }
        true
    }

    pub(super) fn clear_open(&mut self) -> bool {
        let changed = self.hover.take().is_some();
        self.popout.take().is_some() || changed
    }

    pub(super) fn reset(&mut self) {
        self.clear_open();
        self.stickers.clear();
        self.guilds.clear();
        self.catalogs.clear();
        self.generation = self.generation.wrapping_add(1);
    }
}

impl DiscordApp {
    pub(crate) fn set_expression_hover(
        &mut self,
        owner: HoverOwner,
        expression: Expression,
        bounds: Bounds<Pixels>,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .expression_info
            .set_hover(owner, expression, bounds, hovered)
        {
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn open_expression_info(
        &mut self,
        expression: Expression,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let bounds = self
            .expression_info
            .hover
            .take()
            .filter(|hover| hover.expression.key() == expression.key())
            .map_or_else(
                || Bounds::new(position, size(px(0.), px(0.))),
                |hover| hover.bounds,
            );
        let sticker_id = match &expression {
            Expression::Sticker(sticker) => Some(sticker.id),
            _ => None,
        };
        self.expression_info.popout = Some(Popout::new(expression, bounds));
        cx.notify();

        let Some(id) = sticker_id.filter(|id| self.store.sticker_origin(*id).is_none()) else {
            return;
        };
        if self.expression_info.sticker_guild(id).is_some() {
            return;
        }
        if self.expression_info.sticker_details(id).is_some() {
            self.load_sticker_guild(id, cx);
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        // Metadata is fetched only on click, never while moving over the message list.
        if !reserve_load(&mut self.expression_info.stickers, id) {
            return;
        }
        let generation = self.expression_info.generation;
        let request = Tokio::spawn(cx, async move { session.sticker_details(id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if app.expression_info.generation != generation {
                    return;
                }
                let entry = match result {
                    Ok(Ok(details)) if details.id == id => Load::Ready(details),
                    _ => Load::Failed,
                };
                app.expression_info.stickers.insert(id, entry);
                app.load_sticker_guild(id, cx);
                app.views.overlays.update(cx, |_, cx| cx.notify());
            })
            .ok();
        })
        .detach();
    }

    fn load_sticker_guild(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.expression_info.sticker_guild(id).is_some()
            || self
                .expression_info
                .sticker_details(id)
                .is_some_and(|details| details.kind == 1 || details.pack_id.is_some())
        {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !reserve_load(&mut self.expression_info.guilds, id) {
            return;
        }
        let generation = self.expression_info.generation;
        let request = Tokio::spawn(cx, async move { session.sticker_guild(id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if app.expression_info.generation != generation {
                    return;
                }
                let expected = app
                    .expression_info
                    .sticker_details(id)
                    .and_then(|details| details.guild_id);
                let entry = match result {
                    Ok(Ok(guild)) if guild.id != 0 && expected.is_none_or(|id| id == guild.id) => {
                        Load::Ready(guild)
                    }
                    _ => Load::Failed,
                };
                app.expression_info.guilds.insert(id, entry);
                app.load_open_sticker_catalog(cx);
                app.views.overlays.update(cx, |_, cx| cx.notify());
            })
            .ok();
        })
        .detach();
    }

    fn toggle_expression_stickers(&mut self, cx: &mut Context<Self>) {
        let Some(popout) = &mut self.expression_info.popout else {
            return;
        };
        if !popout.toggle_stickers() {
            return;
        }
        self.load_open_sticker_catalog(cx);
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn load_open_sticker_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(popout) = self
            .expression_info
            .popout
            .as_ref()
            .filter(|card| card.expanded)
        else {
            return;
        };
        let Some(source) = source(self, popout) else {
            return;
        };
        if self.store.guild_stickers(source.id).is_some() || source.catalog_complete {
            return;
        }
        let guild_id = source.id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !reserve_load(&mut self.expression_info.catalogs, guild_id) {
            return;
        }
        let generation = self.expression_info.generation;
        let request = Tokio::spawn(
            cx,
            async move { session.sticker_guild_preview(guild_id).await },
        );
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if app.expression_info.generation != generation {
                    return;
                }
                let entry = match result {
                    Ok(Ok(guild)) if guild.id == guild_id && guild.stickers.is_some() => {
                        Load::Ready(guild)
                    }
                    _ => Load::Failed,
                };
                app.expression_info.catalogs.insert(guild_id, entry);
                app.views.overlays.update(cx, |_, cx| cx.notify());
            })
            .ok();
        })
        .detach();
    }

    fn inspect_catalog_sticker(&mut self, sticker: Sticker, cx: &mut Context<Self>) {
        let Some(popout) = &mut self.expression_info.popout else {
            return;
        };
        if popout.inspect_sticker(sticker) {
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    fn hover_catalog_sticker(&mut self, id: u64, hovered: bool, cx: &mut Context<Self>) {
        let Some(popout) = &mut self.expression_info.popout else {
            return;
        };
        let next = if hovered {
            Some(id)
        } else {
            popout.hovered_sticker.filter(|current| *current != id)
        };
        if popout.hovered_sticker != next {
            popout.hovered_sticker = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn close_expression_info(&mut self, cx: &mut Context<Self>) -> bool {
        let changed = self.expression_info.clear_open();
        if changed {
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
        changed
    }
}

fn preview(expression: &Expression, extent: f32, cx: &App) -> AnyElement {
    let (spec, animate) = match expression {
        Expression::Emoji { id, animated, .. } => (
            Some(ImageSpec::Emoji {
                id: *id,
                animated: false,
            }),
            *animated && crate::settings::emoji_animation(),
        ),
        Expression::Sticker(sticker) => (sticker.image_spec(), matches!(sticker.format, 2 | 4)),
        Expression::Default(sequence) => {
            if let Some(image) = emoji::image(sequence, theme::image_px(extent)) {
                return img(image)
                    .id(("expression-preview", expression.key().1))
                    .size(rpx(extent))
                    .flex_none()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element();
            }
            (None, false)
        }
    };
    if let Some(spec) = spec {
        let animated_spec = match &spec {
            ImageSpec::Emoji { id, .. } => ImageSpec::Emoji {
                id: *id,
                animated: true,
            },
            _ => spec.clone(),
        };
        let pixels = theme::image_px(extent);
        if animate && let Some(frames) = animated::resolve(&animated_spec, pixels, cx) {
            return animated::image(frames)
                .object_fit(ObjectFit::Contain)
                .id(("expression-preview", expression.key().1))
                .size(rpx(extent))
                .flex_none()
                .into_any_element();
        }
        if let Some(path) = images::resolve(&spec, pixels, cx) {
            return img(path)
                .id(("expression-preview", expression.key().1))
                .size(rpx(extent))
                .flex_none()
                .object_fit(ObjectFit::Contain)
                .into_any_element();
        }
    }
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(extent))
        .flex_none()
        .child(theme::icon(
            if expression.key().0 {
                "sticker"
            } else {
                "smile"
            },
            extent.min(24.),
            theme::muted(),
        ))
        .into_any_element()
}

fn resolved_expression(app: &DiscordApp, expression: &Expression) -> Expression {
    match expression {
        Expression::Sticker(sticker) => {
            if let Some(details) = app.expression_info.sticker_details(sticker.id) {
                return Expression::Sticker(Sticker {
                    id: sticker.id,
                    name: if details.name.is_empty() {
                        sticker.name.clone()
                    } else {
                        details.name.clone()
                    },
                    format: if matches!(details.format_type, 1..=4) {
                        details.format_type
                    } else {
                        sticker.format
                    },
                });
            }
        }
        Expression::Default(_) => {}
        Expression::Emoji { id, .. } => {
            if let Some((_, emoji)) = app.store.emoji_origin(*id) {
                return Expression::Emoji {
                    id: *id,
                    name: emoji.name.clone(),
                    animated: emoji.animated,
                };
            }
        }
    }
    expression.clone()
}

fn hover_card(app: &DiscordApp, hover: &Hover, cx: &App) -> AnyElement {
    let caption = || {
        div()
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(theme::muted()))
            .child("Click to learn more")
    };
    let expression = resolved_expression(app, &hover.expression);
    let label = expression.label();
    let body = theme::tooltip_bubble()
        .max_w(rpx(280.))
        .text_size(rpx(14.))
        .line_height(rpx(18.));
    let body = match &expression {
        Expression::Sticker(_) => body
            .flex()
            .flex_col()
            .gap(rpx(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .child(theme::icon("sticker", 16., theme::strong()))
                    .child(div().min_w_0().line_clamp(2).child(label)),
            )
            .child(caption()),
        Expression::Emoji { .. } | Expression::Default(_) => body
            .flex()
            .items_center()
            .gap(rpx(8.))
            .child(preview(&expression, 56., cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(rpx(4.))
                    .child(div().line_clamp(2).child(label))
                    .child(caption()),
            ),
    };
    body.into_any_element()
}

fn origin<'a>(app: &'a DiscordApp, expression: &Expression) -> Option<&'a Guild> {
    match expression {
        Expression::Emoji { id, .. } => app.store.emoji_origin(*id).map(|(guild, _)| guild),
        Expression::Sticker(sticker) => app.store.sticker_origin(sticker.id).or_else(|| {
            let details = app.expression_info.sticker_details(sticker.id)?;
            app.store.guild_by_id(details.guild_id?)
        }),
        Expression::Default(_) => None,
    }
}

fn server_explanation(expression: &Expression, discoverable: Option<bool>) -> &'static str {
    match (expression, discoverable) {
        (Expression::Sticker(_), Some(false)) => "This sticker is from an invite-only server.",
        (Expression::Sticker(_), Some(true)) => "This sticker is from a discoverable server.",
        (Expression::Sticker(_), None) => {
            "This sticker is from a server that is either invite-only or unavailable."
        }
        (Expression::Emoji { .. }, Some(false)) => "This emoji is from an invite-only server.",
        (Expression::Emoji { .. }, Some(true)) => "This emoji is from a discoverable server.",
        (Expression::Emoji { .. }, None) => {
            "This emoji is from a server that is either invite-only or unavailable."
        }
        (Expression::Default(_), _) => DEFAULT_EMOJI,
    }
}

fn explanation(
    app: &DiscordApp,
    expression: &Expression,
    source: Option<&stickers::Source<'_>>,
) -> String {
    let discoverable = source.and_then(|source| source.discoverable);
    let location = source.filter(|source| source.joined).map(|source| {
        if source.id == app.selected_guild_id {
            "this server"
        } else {
            "one of your servers"
        }
    });
    match expression {
        Expression::Default(_) => DEFAULT_EMOJI.to_owned(),
        Expression::Emoji { id, .. } => {
            let Some((guild, emoji)) = app.store.emoji_origin(*id) else {
                return server_explanation(expression, discoverable).to_owned();
            };
            let access =
                app.store
                    .emoji_access(emoji, app.selected_guild_id, app.selected_channel_id);
            let usage = match access {
                EmojiAccess::Usable if app.store.has_nitro() => "You can use it everywhere.",
                EmojiAccess::Usable => "You can use it in this server.",
                EmojiAccess::NeedsNitro => "Nitro is required to use this emoji here.",
                EmojiAccess::NeedsRole => "A server role is required to use this emoji.",
                EmojiAccess::Unavailable => "This emoji is currently unavailable.",
                EmojiAccess::NoExternalPermission => {
                    "External emoji aren't allowed in this channel."
                }
            };
            let location = if guild.id == app.selected_guild_id {
                "this server"
            } else {
                "one of your servers"
            };
            format!("This emoji is from {location}. {usage}")
        }
        Expression::Sticker(sticker) => {
            if let Some(location) = location {
                return format!("This sticker is from {location}.");
            }
            match app.expression_info.sticker_details(sticker.id) {
                Some(details) if details.kind == 1 || details.pack_id.is_some() => {
                    "This sticker is from a Discord sticker pack.".to_owned()
                }
                _ => server_explanation(expression, discoverable).to_owned(),
            }
        }
    }
}

fn fallback_information(state: &State, expression: &Expression) -> Option<&'static str> {
    let Expression::Sticker(sticker) = expression else {
        return None;
    };
    if state
        .sticker_details(sticker.id)
        .is_some_and(|details| details.kind == 1 || details.pack_id.is_some())
    {
        Some("Discord sticker pack")
    } else if matches!(state.stickers.get(&sticker.id), Some(Load::Pending))
        || matches!(state.guilds.get(&sticker.id), Some(Load::Pending))
    {
        Some("Loading sticker information...")
    } else {
        None
    }
}

fn info_card(app: &DiscordApp, popout: &Popout, window: &Window, cx: &App) -> AnyElement {
    let resolved = resolved_expression(app, &popout.expression);
    let expression = &resolved;
    let source = source(app, popout);
    let header = div()
        .flex()
        .items_start()
        .gap(rpx(12.))
        .p(rpx(16.))
        .bg(rgb(theme::elevated()))
        .child(preview(expression, 56., cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(2.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(2)
                        .text_color(rgb(theme::text()))
                        .child(expression.label()),
                )
                .child(
                    div()
                        .text_size(rpx(14.))
                        .line_height(rpx(20.))
                        .text_color(rgb(theme::text()))
                        .child(explanation(app, expression, source.as_ref())),
                ),
        );
    let body = if let Some(source) = source {
        Some(stickers::source_card(app, popout, &source, cx))
    } else {
        fallback_information(&app.expression_info, expression).map(|text| {
            div()
                .text_color(rgb(theme::muted()))
                .child(text)
                .into_any_element()
        })
    };
    let max_height =
        (window.viewport_size().height - px(24.) - window.client_inset().unwrap_or(px(0.)) * 2.)
            .max(px(0.));
    div()
        .id("expression-info-scroll")
        .w(rpx(326.))
        .max_w(relative(1.))
        .max_h(max_height)
        .rounded(rpx(8.))
        .overflow_hidden()
        .overflow_y_scrollbar()
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .bg(rgb(theme::bg()))
        .text_size(rpx(14.))
        .line_height(rpx(20.))
        .text_color(rgb(theme::text()))
        .child(header)
        .children(body.map(|body| div().p(rpx(16.)).child(body)))
        .into_any_element()
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    if let Some(popout) = &app.expression_info.popout {
        let entity = app.entity.clone();
        let position = point(
            popout.bounds.right() + rpx(8.).to_pixels(window.rem_size()),
            popout.bounds.top(),
        );
        return Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(Edges::all(px(12.)))
                    .child(
                        div()
                            .id("expression-info-popout")
                            .occlude()
                            .without_text_selection()
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .on_mouse_down_out(move |_, _, cx| {
                                entity
                                    .update(cx, |app, cx| {
                                        app.close_expression_info(cx);
                                    })
                                    .ok();
                            })
                            .child(info_card(app, popout, window, cx)),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        );
    }
    let hover = app.expression_info.hover.as_ref()?;
    if !hover.bounds.contains(&window.mouse_position()) {
        return None;
    }
    let position = point(
        hover.bounds.center().x,
        hover.bounds.top() - rpx(8.).to_pixels(window.rem_size()),
    );
    Some(
        deferred(
            anchored()
                .position(position)
                .anchor(Anchor::BottomCenter)
                .snap_to_window_with_margin(Edges::all(px(12.)))
                .child(hover_card(app, hover, cx)),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CACHE_LIMIT, Expression, HoverOwner, Load, Popout, State, fallback_information,
        reserve_load, server_explanation,
    };
    use crate::{discord::model, store::Sticker};
    use gpui::{Bounds, point, px, size};
    use serde_json::json;
    use std::collections::HashMap;

    fn emoji(id: u64) -> Expression {
        Expression::Emoji {
            id,
            name: "wave".to_owned(),
            animated: true,
        }
    }

    #[test]
    fn server_explanations_distinguish_unknown_private_and_public_origins() {
        let sticker = Expression::Sticker(Sticker {
            id: 42,
            name: "wave".into(),
            format: 1,
        });
        for (expression, kind) in [(sticker, "sticker"), (emoji(43), "emoji")] {
            assert_eq!(
                server_explanation(&expression, None),
                format!("This {kind} is from a server that is either invite-only or unavailable.")
            );
            assert_eq!(
                server_explanation(&expression, Some(false)),
                format!("This {kind} is from an invite-only server.")
            );
            assert_eq!(
                server_explanation(&expression, Some(true)),
                format!("This {kind} is from a discoverable server.")
            );
        }
    }

    #[test]
    fn missing_server_details_have_no_unavailable_footer_but_keep_loading_and_pack_labels() {
        let sticker = Expression::Sticker(Sticker {
            id: 42,
            name: "wave".into(),
            format: 1,
        });
        let mut state = State::default();
        assert_eq!(fallback_information(&state, &emoji(43)), None);
        assert_eq!(fallback_information(&state, &sticker), None);
        state.stickers.insert(42, Load::Pending);
        assert_eq!(
            fallback_information(&state, &sticker),
            Some("Loading sticker information...")
        );
        state.stickers.insert(42, Load::Failed);
        assert_eq!(fallback_information(&state, &sticker), None);
        state.guilds.insert(42, Load::Pending);
        assert_eq!(
            fallback_information(&state, &sticker),
            Some("Loading sticker information...")
        );
        state.guilds.insert(42, Load::Failed);
        assert_eq!(fallback_information(&state, &sticker), None);
        let pack = serde_json::from_value(json!({
            "id": "42", "name": "wave", "type": 1, "format_type": 1, "pack_id": "100"
        }))
        .unwrap();
        state.stickers.insert(42, Load::Ready(pack));
        assert_eq!(
            fallback_information(&state, &sticker),
            Some("Discord sticker pack")
        );
    }

    #[test]
    fn leaving_an_old_expression_cannot_hide_the_new_tooltip() {
        let mut state = State::default();
        let bounds = Bounds::new(point(px(10.), px(10.)), size(px(22.), px(22.)));
        let first = HoverOwner::Emoji(1, 1, 0);
        let second = HoverOwner::Emoji(1, 1, 1);
        assert!(state.set_hover(first, emoji(42), bounds, true));
        assert!(!state.set_hover(first, emoji(42), bounds, true));
        assert!(state.set_hover(second, emoji(42), bounds, true));
        assert!(!state.set_hover(first, emoji(42), bounds, false));
        assert!(
            state
                .hover
                .as_ref()
                .is_some_and(|hover| hover.owner == second)
        );
        assert!(state.set_hover(second, emoji(42), bounds, false));
        assert!(state.hover.is_none());
    }

    #[test]
    fn open_cards_suppress_hover_and_account_reset_invalidates_requests() {
        let mut state = State::default();
        let bounds = Bounds::new(point(px(10.), px(10.)), size(px(22.), px(22.)));
        state.popout = Some(Popout::new(emoji(42), bounds));
        assert!(!state.set_hover(HoverOwner::Emoji(1, 1, 0), emoji(42), bounds, true));
        assert!(state.clear_open());
        assert!(!state.clear_open());
        state.stickers.insert(42, Load::Pending);
        state.guilds.insert(42, Load::Pending);
        state.catalogs.insert(200, Load::Pending);
        let generation = state.generation;
        state.reset();
        assert!(state.stickers.is_empty());
        assert!(state.guilds.is_empty());
        assert!(state.catalogs.is_empty());
        assert_ne!(state.generation, generation);
    }

    #[test]
    fn expanding_and_browsing_preserve_the_source_sticker_and_anchor() {
        let bounds = Bounds::new(point(px(20.), px(40.)), size(px(160.), px(160.)));
        let mut popout = Popout::new(
            Expression::Sticker(Sticker {
                id: 42,
                name: "first".into(),
                format: 1,
            }),
            bounds,
        );
        assert!(!popout.expanded);
        assert!(popout.toggle_stickers());
        assert!(popout.expanded);
        popout.hovered_sticker = Some(43);
        assert!(popout.inspect_sticker(Sticker {
            id: 43,
            name: "second".into(),
            format: 4
        }));
        assert_eq!(popout.expression.key(), (true, 43));
        assert_eq!(popout.source_sticker, Some(42));
        assert_eq!(popout.bounds, bounds);
        assert!(popout.expanded);
        assert!(popout.toggle_stickers());
        assert!(!popout.expanded);
        assert!(popout.hovered_sticker.is_none());
        let mut emoji = Popout::new(emoji(44), bounds);
        assert!(!emoji.toggle_stickers());
        assert!(!emoji.inspect_sticker(Sticker {
            id: 43,
            name: "second".into(),
            format: 4
        }));
    }

    #[test]
    fn metadata_requests_deduplicate_bound_the_cache_and_allow_retry() {
        let mut cache: HashMap<u64, Load<()>> = HashMap::new();
        assert!(reserve_load(&mut cache, 1));
        assert!(!reserve_load(&mut cache, 1));
        cache.insert(1, Load::Failed);
        assert!(reserve_load(&mut cache, 1));
        cache.insert(1, Load::Ready(()));
        assert!(!reserve_load(&mut cache, 1));
        for id in 2..=CACHE_LIMIT as u64 {
            assert!(reserve_load(&mut cache, id));
        }
        assert!(reserve_load(&mut cache, 200));
        assert_eq!(cache.len(), CACHE_LIMIT);
        assert!(matches!(cache.get(&2), Some(Load::Pending)));
        assert!(!reserve_load(&mut cache, 201));
    }

    #[test]
    fn external_catalogs_are_reused_only_for_exact_sticker_ids() {
        let mut state = State::default();
        let guild: model::StickerGuild = serde_json::from_value(json!({
            "id": "200", "name": "External server", "stickers": [
                {"id": "42", "name": "same-name", "format_type": 1},
                {"id": "43", "name": "same-name", "format_type": 4}
            ]
        }))
        .unwrap();
        state.guilds.insert(42, Load::Ready(guild.clone()));
        assert_eq!(state.sticker_guild(43).unwrap().id, 200);
        assert_eq!(state.sticker_details(43).unwrap().format_type, 4);
        assert!(state.sticker_guild(44).is_none());
        state.guilds.clear();
        state.catalogs.insert(200, Load::Ready(guild));
        assert_eq!(state.sticker_guild(42).unwrap().id, 200);
        assert!(state.sticker_details(44).is_none());
    }
}
