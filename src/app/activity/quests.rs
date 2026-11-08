mod banner;
mod card;

use banner::{banner, control_button, sponsored_banner};
use card::{card, claimed_card, claimed_quests};

use std::{
    cell::RefCell,
    collections::HashMap,
    f32::consts::{FRAC_PI_2, TAU},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, Utc};
use gpui::{prelude::*, *};
use gpui_component::{ElementExt as _, scroll::ScrollableElement as _};
use gpui_tokio::Tokio;

use crate::{
    app::{DiscordApp, chat, guilds::channel_list, guilds::guild_rail},
    discord::model::{Quest, QuestHomeHero, QuestList, QuestReward},
    media::images::{self, ImageSpec},
    theme::{self, rpx},
};

const REFRESH_AFTER: Duration = Duration::from_secs(5 * 60);
const CONTENT_MAX_WIDTH: f32 = 1300.;
const CONTENT_PADDING_X: f32 = 20.;
const CONTENT_PADDING_Y: f32 = 24.;
const GRID_GAP: f32 = 24.;
const CARD_MIN_WIDTH: f32 = 336.;
const CLAIMED_MIN_WIDTH: f32 = 230.;
const CLAIMED_MAX_WIDTH: f32 = 330.;
const CARD_HEADER: f32 = 150.;
const CARD_MIDDLE: f32 = 104.;
const CARD_FOOTER: f32 = 62.;
const CARD_HEIGHT: f32 = CARD_HEADER + CARD_MIDDLE + CARD_FOOTER;
/// `overflow_hidden` clips to a rectangle, so layers that reach a rounded box's corners round
/// themselves to its radius less its 1px border.
const CARD_RADIUS: f32 = 10.;
const BANNER_RADIUS: f32 = 24.;
const MOD_SUBTLE: u32 = 0x97979F1F;
const UNAVAILABLE: &str = "Quests can't be started from zcord yet";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    All,
    Claimed,
}

#[derive(Default)]
pub(crate) struct QuestHome {
    list: Option<Arc<QuestList>>,
    hero: Option<Arc<QuestHomeHero>>,
    loading: bool,
    failed: bool,
    loaded_at: Option<Instant>,
    tab: Tab,
    hovered: Option<u64>,
    left: Option<(u64, u64)>,
    hover_changes: u64,
    text_heights: Rc<RefCell<HashMap<u64, f32>>>,
    owner: u64,
}

impl DiscordApp {
    pub(crate) fn load_quests(&mut self, cx: &mut Context<Self>) {
        let owner = self.store.current_user_id;
        if self.home.quests.owner != owner {
            self.home.quests = QuestHome {
                owner,
                ..QuestHome::default()
            };
        }
        let home = &mut self.home.quests;
        let fresh = home
            .loaded_at
            .is_some_and(|at| at.elapsed() < REFRESH_AFTER);
        if home.loading || fresh {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        home.loading = true;
        let request = Tokio::spawn(cx, async move {
            tokio::join!(session.quests(), session.quest_home_hero())
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let home = &mut app.home.quests;
                if home.owner != owner {
                    return;
                }
                home.loading = false;
                let result = result.map(|(list, hero)| {
                    home.hero = match hero {
                        Ok(hero) => hero.map(Arc::new),
                        Err(error) => {
                            log::debug!("Could not load the Quest Home banner: {error}");
                            None
                        }
                    };
                    list
                });
                match result {
                    Ok(Ok(list)) => {
                        home.text_heights
                            .borrow_mut()
                            .retain(|id, _| list.quests.iter().any(|quest| quest.id == *id));
                        home.list = Some(Arc::new(list));
                        home.failed = false;
                        home.loaded_at = Some(Instant::now());
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not load quests: {error}");
                        home.failed = true;
                    }
                    Err(error) => {
                        log::warn!("Quest request task failed: {error}");
                        home.failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn hover_quest(&mut self, quest_id: u64, hovered: bool, cx: &mut Context<Self>) {
        let home = &mut self.home.quests;
        let next = if hovered {
            Some(quest_id)
        } else if home.hovered == Some(quest_id) {
            None
        } else {
            return;
        };
        if home.hovered != next {
            home.hover_changes += 1;
            if let Some(previous) = home.hovered {
                home.left = Some((previous, home.hover_changes));
            }
            home.hovered = next;
            cx.notify();
        }
    }

    fn select_quest_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.home.quests.tab != tab {
            self.home.quests.tab = tab;
            cx.notify();
        }
    }
}

fn asset_url(path: &str) -> String {
    format!(
        "https://cdn.discordapp.com/{}",
        path.trim_start_matches('/')
    )
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn month_day(value: &str) -> String {
    parse_time(value)
        .map(|time| time.with_timezone(&Local).format("%-m/%-d").to_string())
        .unwrap_or_default()
}

fn short_date(value: &str) -> String {
    parse_time(value)
        .map(|time| time.with_timezone(&Local).format("%-m/%-d/%Y").to_string())
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    Enrolled,
    Completed,
    Claimed,
    Ended,
}

struct View<'a> {
    quest: &'a Quest,
    state: State,
    expired: bool,
    stale: bool,
    expires: Option<DateTime<Utc>>,
}

impl<'a> View<'a> {
    fn new(quest: &'a Quest, now: DateTime<Utc>) -> Self {
        let status = quest.user_status.as_ref();
        let enrolled_at = status.and_then(|status| status.enrolled_at.as_deref());
        let completed = status.is_some_and(|status| status.completed_at.is_some());
        let claimed = status.is_some_and(|status| status.claimed_at.is_some());
        let expires = parse_time(&quest.config.expires_at);
        let expired = expires.is_some_and(|at| at <= now);
        let claimable = completed
            && !claimed
            && quest
                .config
                .rewards_config
                .rewards_expire_at
                .as_deref()
                .and_then(parse_time)
                .is_none_or(|at| at > now);
        let state = match () {
            _ if claimed => State::Claimed,
            _ if completed && (!expired || claimable) => State::Completed,
            _ if expired => State::Ended,
            _ if enrolled_at.is_some() => State::Enrolled,
            _ => State::Open,
        };
        let stale = state == State::Enrolled
            && enrolled_at
                .and_then(parse_time)
                .is_some_and(|at| now - at > chrono::Duration::minutes(20));
        Self {
            quest,
            state,
            expired,
            stale,
            expires,
        }
    }

    fn listed(&self) -> bool {
        !self.expired || self.state == State::Completed
    }

    fn rank(&self) -> (bool, u8, i64) {
        let expires = self.expires.map_or(i64::MAX, |at| at.timestamp());
        if self.expired {
            return (true, 0, -expires);
        }
        let group = match self.state {
            State::Claimed => 3,
            _ if self.stale => 2,
            State::Enrolled | State::Completed => 0,
            _ => 1,
        };
        (false, group, expires)
    }

    fn reward(&self) -> Option<&'a QuestReward> {
        self.quest.config.rewards_config.rewards.first()
    }

    fn task(&self) -> Option<(&'a str, u64)> {
        let tasks = &self.quest.config.task_config_v2.tasks;
        [
            "PLAY_ON_DESKTOP",
            "PLAY_ACTIVITY",
            "STREAM_ON_DESKTOP",
            "WATCH_VIDEO",
            "WATCH_VIDEO_ON_MOBILE",
            "ACHIEVEMENT_IN_ACTIVITY",
            "PLAY_ON_PLAYSTATION",
            "PLAY_ON_XBOX",
        ]
        .into_iter()
        .find_map(|kind| tasks.get(kind).map(|task| (kind, task.target)))
        .or_else(|| {
            tasks
                .values()
                .next()
                .map(|task| (task.kind.as_str(), task.target))
        })
    }

    fn is_video(&self) -> bool {
        matches!(
            self.task(),
            Some(("WATCH_VIDEO" | "WATCH_VIDEO_ON_MOBILE", _))
        )
    }

    fn progress(&self) -> f32 {
        let Some((kind, target)) = self.task() else {
            return 0.;
        };
        let value = self
            .quest
            .user_status
            .as_ref()
            .and_then(|status| status.progress.get(kind))
            .map_or(0., |progress| progress.value);
        if target == 0 {
            return 0.;
        }
        (value / target as f64).clamp(0., 1.) as f32
    }
}

fn orbs(reward: &QuestReward) -> Option<u64> {
    (reward.kind == QuestReward::ORBS).then(|| reward.orb_quantity.unwrap_or(0))
}

fn reward_line(view: &View) -> Div {
    let line = div().flex().flex_wrap().items_center();
    let Some(reward) = view.reward() else {
        return line;
    };
    let claimed = view.state == State::Claimed;
    match orbs(reward) {
        Some(count) => line
            .when(!claimed, |line| line.child("Claim\u{a0}"))
            .child(
                theme::icon("orb", 14., theme::strong())
                    .flex_none()
                    .mr(rpx(3.))
                    .mt(rpx(1.)),
            )
            .child(format!("{} Orbs", grouped(count))),
        None if claimed => line.child(reward.messages.name.clone()),
        None => line.child(format!("Claim {}", with_article(reward))),
    }
}

fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn with_article(reward: &QuestReward) -> String {
    reward
        .messages
        .name_with_article
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| reward.messages.name.clone())
}

fn description(view: &View) -> String {
    if view.state == State::Claimed {
        let claimed = view
            .quest
            .user_status
            .as_ref()
            .and_then(|status| status.claimed_at.as_deref())
            .map(short_date)
            .unwrap_or_default();
        return format!("You claimed this reward on {claimed}");
    }
    let reward = view.reward().map(with_article).unwrap_or_default();
    let game = &view.quest.config.messages.game_title;
    let Some((kind, target)) = view.task() else {
        return String::new();
    };
    let minutes = target.div_ceil(60);
    match kind {
        "WATCH_VIDEO" | "WATCH_VIDEO_ON_MOBILE" => format!("Watch the video to win {reward}!"),
        "STREAM_ON_DESKTOP" => {
            format!("Stream {game} to a friend for {minutes} minutes and win {reward}.")
        }
        "PLAY_ACTIVITY" | "ACHIEVEMENT_IN_ACTIVITY" => {
            format!("Play {game} in Discord for {minutes} minutes to unlock {reward}.")
        }
        _ => format!("Play {game} for {minutes} minutes and win {reward}."),
    }
}

fn watch_label(target: u64) -> String {
    if target >= 60 {
        format!("Watch {}m", target.div_ceil(60))
    } else {
        format!("Watch {target}s")
    }
}

pub(crate) fn header(app: &DiscordApp) -> impl IntoElement + use<> {
    let selected = app.home.quests.tab;
    let tab = |id: &'static str, tab: Tab, text: &'static str| {
        let entity = app.entity.clone();
        let active = selected == tab;
        div()
            .id(id)
            .flex()
            .items_center()
            .h_full()
            .border_b_2()
            .border_color(if active {
                rgb(theme::strong()).into()
            } else {
                transparent_black()
            })
            .when(!active, |this| {
                this.hover(|style| style.border_color(rgb(theme::BLURPLE)))
            })
            .cursor_pointer()
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(if active {
                theme::strong()
            } else {
                theme::muted()
            }))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_quest_tab(tab, cx))
                    .ok();
            })
            .child(text)
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(24.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("laurel-wreath", 20., theme::faint()))
        .child(tab("quests-tab-all", Tab::All, "All Quests"))
        .child(tab("quests-tab-claimed", Tab::Claimed, "Claimed Quests"))
}

fn content_width(window: &Window) -> f32 {
    let unit = rpx(1.).to_pixels(window.rem_size());
    let column = window.viewport_size().width / unit - guild_rail::WIDTH - channel_list::WIDTH;
    column.min(CONTENT_MAX_WIDTH) - CONTENT_PADDING_X * 2.
}

fn grid_columns(width: f32, min: f32) -> (usize, f32) {
    let columns = (((width + GRID_GAP) / (min + GRID_GAP)).floor() as usize).max(1);
    let card = ((width - GRID_GAP * (columns - 1) as f32) / columns as f32).max(min);
    (columns, card)
}

fn grid(cards: Vec<AnyElement>, columns: usize) -> Div {
    let mut rows = Vec::new();
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        rows.push(
            div()
                .flex()
                .gap(rpx(GRID_GAP))
                .children(cards.by_ref().take(columns)),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(rpx(GRID_GAP))
        .w_full()
        .children(rows)
}

pub(crate) fn page(app: &DiscordApp, cards: &QuestCards, window: &Window, cx: &App) -> AnyElement {
    let home = &app.home.quests;
    let body = match (&home.list, home.tab) {
        (Some(list), Tab::All) => all_quests(app, list, cards, window, cx),
        (Some(list), Tab::Claimed) => claimed_quests(app, list, cards, window, cx),
        (None, _) if home.failed => notice(
            "Quests didn't load",
            "Something went wrong getting your Quests. Try again later.",
        ),
        (None, tab) => skeleton(tab, window, cx),
    };
    div()
        .id("quest-home")
        .flex()
        .flex_col()
        .items_center()
        .size_full()
        .bg(rgb(theme::bg()))
        .overflow_y_scrollbar()
        .child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .max_w(rpx(CONTENT_MAX_WIDTH))
                .px(rpx(CONTENT_PADDING_X))
                .py(rpx(CONTENT_PADDING_Y))
                .child(body),
        )
        .into_any_element()
}

fn skeleton(tab: Tab, window: &Window, cx: &App) -> AnyElement {
    let (columns, card_width) = grid_columns(content_width(window), CARD_MIN_WIDTH);
    let reduce_motion = cx.reduce_motion();
    let pulsing = |id: &'static str, block: Div| -> AnyElement {
        if reduce_motion {
            return block.bg(skeleton_fill(0.)).into_any_element();
        }
        block
            .with_animation(
                id,
                Animation::new(SKELETON_PULSE).repeat(),
                |block, delta| {
                    let half = if delta < 0.5 {
                        delta * 2.
                    } else {
                        (1. - delta) * 2.
                    };
                    block.bg(skeleton_fill(ease_in_out(half)))
                },
            )
            .into_any_element()
    };
    let cards = SKELETON_CARD_IDS
        .iter()
        .map(|id| {
            pulsing(
                id,
                div()
                    .flex_none()
                    .w(rpx(card_width))
                    .h(rpx(CARD_HEADER + CARD_MIDDLE + CARD_FOOTER))
                    .rounded(rpx(CARD_RADIUS)),
            )
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .when(tab == Tab::All, |page| {
            page.child(pulsing(
                "quest-skeleton-hero",
                div()
                    .mb(rpx(56.))
                    .min_h(rpx(500.))
                    .rounded(rpx(BANNER_RADIUS)),
            ))
        })
        .child(pulsing(
            "quest-skeleton-title",
            div().w(rpx(145.)).h(rpx(32.)).rounded(rpx(4.)),
        ))
        .child(grid(cards, columns).mt(rpx(24.)))
        .into_any_element()
}

const SKELETON_PULSE: Duration = Duration::from_millis(1600);
const SKELETON_CARD_IDS: [&str; 6] = [
    "quest-skeleton-card-0",
    "quest-skeleton-card-1",
    "quest-skeleton-card-2",
    "quest-skeleton-card-3",
    "quest-skeleton-card-4",
    "quest-skeleton-card-5",
];

fn skeleton_fill(amount: f32) -> Rgba {
    let (subtle, muted) = if theme::is_light() {
        (0x29, 0x1F)
    } else {
        (0x1F, 0x14)
    };
    let alpha = subtle as f32 + (muted as f32 - subtle as f32) * amount;
    rgba(0x97979F00 | alpha.round() as u32)
}

fn notice(title: &'static str, detail: &'static str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(8.))
        .pt(rpx(96.))
        .text_center()
        .child(
            div()
                .text_size(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .max_w(rpx(480.))
                .text_size(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(detail),
        )
        .into_any_element()
}

fn all_quests(
    app: &DiscordApp,
    list: &QuestList,
    cards: &QuestCards,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let now = Utc::now();
    let mut views: Vec<View> = list
        .quests
        .iter()
        .map(|quest| View::new(quest, now))
        .filter(View::listed)
        .collect();
    views.sort_by_key(View::rank);
    let width = content_width(window);
    let (columns, card_width) = grid_columns(width, CARD_MIN_WIDTH);
    let quests = if views.is_empty() {
        notice(
            "Sorry adventurer, there are no Quests for you right now",
            "We couldn't find any Quests that are a good fit for you. Some Quests are only \
             available in certain regions or to users of certain ages. Try checking back later!",
        )
    } else {
        grid(
            views
                .iter()
                .map(|view| {
                    cards.place(view.quest.id, false, card_width, CARD_HEIGHT, || {
                        card(app, view, card_width, cx)
                    })
                })
                .collect(),
            columns,
        )
        .mt(rpx(24.))
        .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .child(match app.home.quests.hero.as_deref() {
            Some(hero) => sponsored_banner(hero, list, width, cx),
            None => banner(width, cx),
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(rpx(16.))
                .child(
                    div()
                        .text_size(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child("Available Quests"),
                )
                .child(
                    div()
                        .flex()
                        .gap(rpx(8.))
                        .child(control_button("quests-sort", "Suggested", true))
                        .child(control_button("quests-filters", "Filters", false)),
                ),
        )
        .child(quests)
        .into_any_element()
}

/// Animated art wakes the view it's drawn in on every frame, so a card playing a clip
/// redraws just itself instead of the whole page.
pub(crate) struct QuestCardView {
    app: WeakEntity<DiscordApp>,
    quest_id: u64,
    claimed: bool,
    _observe: Subscription,
}

impl Render for QuestCardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cx: &App = cx;
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = app.read(cx);
        let Some(quest) = app
            .home
            .quests
            .list
            .as_ref()
            .and_then(|list| list.quests.iter().find(|quest| quest.id == self.quest_id))
        else {
            return div().into_any_element();
        };
        let width = content_width(window);
        if self.claimed {
            let (_, width) = grid_columns(width, CLAIMED_MIN_WIDTH);
            claimed_card(app, quest, width.min(CLAIMED_MAX_WIDTH), cx)
        } else {
            let (_, width) = grid_columns(width, CARD_MIN_WIDTH);
            card(app, &View::new(quest, Utc::now()), width, cx)
        }
    }
}

#[derive(Default)]
pub(crate) struct QuestCards(HashMap<(u64, bool), Entity<QuestCardView>>);

impl QuestCards {
    pub(crate) fn sync(&mut self, app: &Entity<DiscordApp>, cx: &mut App) {
        let home = &app.read(cx).home.quests;
        let keys: Vec<(u64, bool)> = match (&home.list, home.tab) {
            (Some(list), Tab::All) => {
                let now = Utc::now();
                list.quests
                    .iter()
                    .filter(|quest| View::new(quest, now).listed())
                    .map(|quest| (quest.id, false))
                    .collect()
            }
            (Some(list), Tab::Claimed) => list
                .quests
                .iter()
                .filter(|quest| {
                    quest
                        .user_status
                        .as_ref()
                        .is_some_and(|status| status.claimed_at.is_some())
                })
                .map(|quest| (quest.id, true))
                .collect(),
            (None, _) => Vec::new(),
        };
        self.0.retain(|key, _| keys.contains(key));
        for (quest_id, claimed) in keys {
            self.0.entry((quest_id, claimed)).or_insert_with(|| {
                cx.new(|cx| QuestCardView {
                    app: app.downgrade(),
                    quest_id,
                    claimed,
                    _observe: cx.observe(app, |_, _, cx| cx.notify()),
                })
            });
        }
    }

    fn place(
        &self,
        quest_id: u64,
        claimed: bool,
        width: f32,
        height: f32,
        fallback: impl FnOnce() -> AnyElement,
    ) -> AnyElement {
        match self.0.get(&(quest_id, claimed)) {
            Some(view) => view
                .clone()
                .cached(
                    StyleRefinement::default()
                        .flex_none()
                        .w(rpx(width))
                        .h(rpx(height)),
                )
                .into_any_element(),
            None => fallback(),
        }
    }
}
