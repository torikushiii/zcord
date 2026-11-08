mod element;
mod layout;

use layout::{Fragment, Layout, Line};

use std::{
    cell::RefCell,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use crate::theme;

mod selection;

pub(crate) use selection::keep_registered;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod metrics {
    use std::cell::Cell;
    thread_local! {
        pub static SHAPES: Cell<usize> = const { Cell::new(0) };
        pub static LAYOUTS: Cell<usize> = const { Cell::new(0) };
        pub static PAINTS: Cell<usize> = const { Cell::new(0) };
        pub static EMOJIS: Cell<usize> = const { Cell::new(0) };
    }
}

#[derive(Clone)]
pub enum Action {
    OpenUrl(SharedString),
    OpenUserProfile(u64),
    RevealSpoiler(u16),
    InspectEmoji {
        id: u64,
        name: SharedString,
        animated: bool,
    },
    InspectDefaultEmoji(SharedString),
    OpenChannel {
        channel_id: u64,
        message_id: Option<u64>,
    },
}

#[derive(Clone, Copy, PartialEq)]
pub enum Chip {
    Code,
    Mention { color: Option<u32> },
    Spoiler { revealed: bool },
    Timestamp,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconShape {
    Round,
    Square,
}

#[derive(Clone, Default)]
pub struct PieceStyle {
    pub weight: Option<FontWeight>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub color: Option<Hsla>,
    pub scale: Option<f32>,
    pub mono: bool,
    pub chip: Option<Chip>,
    pub group: u32,
    pub nowrap: bool,
    pub cell: bool,
    pub action: Option<Action>,
    pub icon: Option<IconShape>,
}

pub enum EmojiSource {
    Still(ImageSource),
    Animated(Arc<RenderImage>),
    Icon(SharedString),
}

pub enum Content {
    Text(SharedString),
    Emoji(Option<EmojiSource>, SharedString),
}

pub struct Piece {
    pub content: Content,
    pub style: PieceStyle,
}

impl Piece {
    pub fn text(text: impl Into<SharedString>, style: PieceStyle) -> Self {
        Self {
            content: Content::Text(text.into()),
            style,
        }
    }
}

type ActionHandler = Rc<dyn Fn(&Action, &mut Window, &mut App)>;
type ActionHoverHandler = Rc<dyn Fn(ActionHover, &mut Window, &mut App)>;

pub struct ActionHover {
    pub action: Action,
    pub piece: usize,
    pub hovered: bool,
    pub bounds: Bounds<Pixels>,
}

pub struct RichText {
    id: ElementId,
    pieces: Rc<Vec<Piece>>,
    line_height: Option<Rems>,
    emoji_size: Option<Rems>,
    emoji_gap: Option<Rems>,
    emoji_scale: f32,
    line_clamp: Option<usize>,
    wrap: bool,
    truncate: bool,
    selectable: bool,
    on_action: Option<ActionHandler>,
    on_action_hover: Option<ActionHoverHandler>,
}

pub fn rich_text(id: impl Into<ElementId>, pieces: Vec<Piece>) -> RichText {
    RichText {
        id: id.into(),
        pieces: Rc::new(pieces),
        line_height: None,
        emoji_size: None,
        emoji_gap: None,
        emoji_scale: 1.375,
        line_clamp: None,
        wrap: true,
        truncate: false,
        selectable: false,
        on_action: None,
        on_action_hover: None,
    }
}

impl RichText {
    pub fn selectable(mut self) -> Self {
        self.selectable = true;
        self
    }

    pub fn line_height(mut self, line_height: Rems) -> Self {
        self.line_height = Some(line_height);
        self
    }

    pub fn emoji_size(mut self, size: Rems) -> Self {
        self.emoji_size = Some(size);
        self
    }

    pub fn line_clamp(mut self, lines: usize) -> Self {
        self.line_clamp = Some(lines);
        self
    }

    pub fn emoji_gap(mut self, gap: Rems) -> Self {
        self.emoji_gap = Some(gap);
        self
    }

    pub fn emoji_scale(mut self, scale: f32) -> Self {
        self.emoji_scale = scale;
        self
    }

    pub fn single_line(mut self) -> Self {
        self.wrap = false;
        self
    }

    pub fn truncate(mut self) -> Self {
        self.wrap = false;
        self.truncate = true;
        self
    }

    pub fn on_action(mut self, handler: impl Fn(&Action, &mut Window, &mut App) + 'static) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    pub fn on_action_hover(
        mut self,
        handler: impl Fn(ActionHover, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action_hover = Some(Rc::new(handler));
        self
    }
}
