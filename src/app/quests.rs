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
    app::{DiscordApp, channel_list, chat, guild_rail},
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
const DETAILS_GROW: Duration = Duration::from_millis(250);
/// `overflow_hidden` clips to a rectangle, so layers that reach a rounded box's corners round
/// themselves to its radius less its 1px border.
const CARD_RADIUS: f32 = 10.;
const BANNER_RADIUS: f32 = 24.;
const HERO_ASPECT: f32 = 370. / 1320.;
const REWARD_TILE: f32 = 80.;
const RING: f32 = 76.;
const RING_STROKE: f32 = 4.;
const TEXT_BRAND: u32 = 0x94A8FF;
const POSITIVE: u32 = 0x23A55A;
const ORB_PURPLE: u32 = 0x5B3FD9;
/// The video decoder only accepts MP4, so the still is cut from the clip's WebM twin.
const ORB_VIDEO: &str = "https://cdn.discordapp.com/assets/content/f763593591e9ac5ee5abab6d8cce6c395263fa51f6c1e30c07cbfc793cd877d2.mp4";
const ORB_STILL: &str = "https://cdn.discordapp.com/assets/content/fb761d9c206f93cd8c4e7301798abe3f623039a4054f2e7accd019e1bb059fc8.webm?format=webp";
const ORB_ART_SIZE: u32 = 320;
const MOD_SUBTLE: u32 = 0x97979F1F;
const MOD_STRONG: u32 = 0x97979F33;
const ONYX_BASE: u32 = 0x1A1A1E;
const BANNER_ART: &str = "https://cdn.discordapp.com/assets/content/fff9144c814ffc3126fdcaf5ec5bc68eff6d09e23ac8af5a3fbe61b05970cda0.png";
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
    pub(super) fn load_quests(&mut self, cx: &mut Context<Self>) {
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

fn control_button(id: &'static str, label: &'static str, chevron: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgba(MOD_SUBTLE))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .opacity(0.5)
        .cursor(CursorStyle::OperationNotAllowed)
        .child(label)
        .when(chevron, |this| {
            this.child(theme::icon("chevron-down", 16., theme::strong()))
        })
        .child(theme::hover_tooltip(
            SharedString::from(format!("{id}-tip")),
            "Sorting and filters aren't available in zcord yet",
        ))
}

fn banner(width: f32, cx: &App) -> Div {
    let art = images::resolve(
        &ImageSpec::Media {
            url: BANNER_ART.to_owned(),
            width: 0,
            height: 0,
            animated: false,
        },
        theme::image_px(width),
        cx,
    );
    div()
        .relative()
        .flex()
        .flex_col()
        .justify_center()
        .mb(rpx(64.))
        .min_h(rpx(if width < 725. { 580. } else { 500. }))
        .p(rpx(32.))
        .rounded(rpx(BANNER_RADIUS))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(rgb(ONYX_BASE))
        .children(art.map(|art| {
            img(art)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(BANNER_RADIUS - 1.))
                .object_fit(ObjectFit::Cover)
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(rpx(600.))
                .rounded_l(rpx(BANNER_RADIUS - 1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(ONYX_BASE), 0.32),
                    linear_color_stop(rgba(ONYX_BASE << 8), 1.),
                )),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .max_w(rpx(540.))
                .child(
                    div()
                        .text_size(rpx(if width < 500. { 32. } else { 44. }))
                        .line_height(rpx(if width < 500. { 32. } else { 44. }))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(white())
                        .child("PLAY, WATCH,")
                        .child("EARN REWARDS"),
                )
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .text_color(white())
                        .opacity(0.8)
                        .child(
                            "Win in-game items, Discord avatar decorations, and more through \
                             Quests.",
                        ),
                )
                .child(div().flex().child(button(
                    "quests-learn-more",
                    0,
                    "Learn More",
                    ButtonKind::Overlay,
                ))),
        )
}

fn hero_url(path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.to_owned()
    } else {
        asset_url(path)
    }
}

fn sponsored_banner(hero: &QuestHomeHero, list: &QuestList, width: f32, cx: &App) -> Div {
    let pixels = theme::image_px(width);
    let radius = rpx(BANNER_RADIUS - 1.);
    let clip = hero
        .hero_video
        .as_deref()
        .filter(|path| !cx.reduce_motion() && path.ends_with(".mp4"))
        .and_then(|path| {
            crate::media::video::resolve(
                &ImageSpec::Video {
                    url: hero_url(path),
                    width: 0,
                    height: 0,
                },
                pixels,
                cx,
            )
        })
        .map(|frames| {
            crate::media::animated::image(frames)
                .size_full()
                .rounded(radius)
                .object_fit(ObjectFit::Cover)
                .id("quest-home-hero-clip")
                .into_any_element()
        });
    let art = clip.or_else(|| {
        hero.hero_image
            .as_deref()
            .and_then(|path| {
                images::resolve(
                    &ImageSpec::Media {
                        url: hero_url(path),
                        width: 0,
                        height: 0,
                        animated: false,
                    },
                    pixels,
                    cx,
                )
            })
            .map(|still| {
                img(still)
                    .size_full()
                    .rounded(radius)
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
    });
    let logo = hero.sponsor_image.as_deref().and_then(|path| {
        images::resolve(
            &ImageSpec::Media {
                url: hero_url(path),
                width: 0,
                height: 0,
                animated: false,
            },
            theme::image_px(132.),
            cx,
        )
    });
    let cta = hero
        .cta
        .clone()
        .filter(|cta| !cta.button_label.is_empty() && !cta.url.is_empty());
    let quest = hero
        .quest_ids
        .iter()
        .find(|id| list.quests.iter().any(|quest| quest.id == **id))
        .copied();
    div()
        .relative()
        .flex()
        .flex_col()
        .mb(rpx(64.))
        .min_h(rpx(if width < 725. { 580. } else { 500. }))
        .p(rpx(32.))
        .rounded(rpx(BANNER_RADIUS))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(rgb(ONYX_BASE))
        .children(art.map(|art| div().absolute().inset_0().child(art)))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(rpx(600.))
                .max_w_full()
                .rounded_l(radius)
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(theme::base_lower()), 0.32),
                    linear_color_stop(rgba(theme::base_lower() << 8), 1.),
                )),
        )
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .justify_between()
                .children(logo.map(|logo| {
                    img(logo)
                        .flex_none()
                        .max_h(rpx(56.))
                        .max_w(rpx(132.))
                        .object_fit(ObjectFit::Contain)
                }))
                .child(
                    div()
                        .ml_auto()
                        .mt(rpx(-8.))
                        .mr(rpx(-8.))
                        .px(rpx(8.))
                        .py(rpx(4.))
                        .rounded(rpx(8.))
                        .bg(rgba(0x0000_0080))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::strong()))
                        .child("Sponsored"),
                ),
        )
        .child(div().flex_1().min_h(rpx(32.)))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .max_w(rpx(540.))
                .child(
                    div()
                        .text_size(rpx(if width < 500. { 32. } else { 44. }))
                        .line_height(rpx(if width < 500. { 32. } else { 44. }))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(white())
                        .child(hero.label_title.to_uppercase()),
                )
                .when(!hero.label_subtitle.is_empty(), |text| {
                    text.child(
                        div()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .text_color(white())
                            .opacity(0.8)
                            .child(hero.label_subtitle.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .children(cta.map(|cta| {
                            let url = cta.url;
                            div()
                                .id("quest-home-hero-cta")
                                .flex()
                                .items_center()
                                .h(rpx(38.))
                                .px(rpx(16.))
                                .rounded(rpx(8.))
                                .bg(white())
                                .hover(|style| style.opacity(0.9))
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(black())
                                .cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                                .child(cta.button_label)
                        }))
                        .children(quest.map(|quest| {
                            div()
                                .id(("quest-home-hero-view", quest))
                                .relative()
                                .flex()
                                .items_center()
                                .h(rpx(38.))
                                .px(rpx(16.))
                                .rounded(rpx(8.))
                                .hover(|style| style.bg(rgba(0xFFFF_FF1F)))
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white())
                                .child("View Quest")
                                .child(theme::hover_tooltip(
                                    ("quest-home-hero-view-tip", quest),
                                    UNAVAILABLE,
                                ))
                        })),
                ),
        )
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Primary,
    Secondary,
    Overlay,
}

fn button(
    id: &'static str,
    quest: u64,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Stateful<Div> {
    let (background, color) = match kind {
        ButtonKind::Primary => (rgb(theme::BLURPLE).into(), white()),
        ButtonKind::Secondary => (rgba(MOD_SUBTLE).into(), rgb(theme::strong()).into()),
        ButtonKind::Overlay => (white(), black()),
    };
    let background: Hsla = background;
    div()
        .id((id, quest))
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .h(rpx(38.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .bg(background)
        .text_size(rpx(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .opacity(0.5)
        .cursor(CursorStyle::OperationNotAllowed)
        .child(div().truncate().child(label.into()))
        .child(theme::hover_tooltip((id, quest + 1), UNAVAILABLE))
}

fn footer(view: &View) -> Div {
    let id = view.quest.id;
    let cta = view
        .quest
        .config
        .cta_config
        .as_ref()
        .and_then(|cta| cta.button_label.clone())
        .filter(|label| !label.is_empty());
    let buttons: Vec<Stateful<Div>> = match view.state {
        State::Ended => vec![button(
            "quest-ended",
            id,
            format!("Quest Ended {}", month_day(&view.quest.config.expires_at)),
            ButtonKind::Secondary,
        )],
        State::Claimed => {
            let code = view
                .reward()
                .is_some_and(|reward| reward.kind == QuestReward::REDEEM_CODE);
            vec![button(
                "quest-view-reward",
                id,
                if code { "See Code" } else { "View Reward" },
                ButtonKind::Secondary,
            )]
        }
        State::Completed => vec![button(
            "quest-claim",
            id,
            "Claim Reward",
            ButtonKind::Primary,
        )],
        State::Enrolled if view.is_video() => vec![button(
            "quest-resume",
            id,
            "Resume Video",
            ButtonKind::Primary,
        )],
        State::Enrolled => vec![match cta {
            Some(label) => button("quest-cta", id, label, ButtonKind::Secondary),
            None => button(
                "quest-accepted",
                id,
                "Quest Accepted",
                ButtonKind::Secondary,
            ),
        }],
        State::Open => {
            let accept = match view.task() {
                Some(("WATCH_VIDEO" | "WATCH_VIDEO_ON_MOBILE", target)) => watch_label(target),
                Some(("ACHIEVEMENT_IN_ACTIVITY", _)) => "Launch Quest".to_owned(),
                Some(("PLAY_ACTIVITY", _)) => "Play Now".to_owned(),
                _ => "Accept Quest".to_owned(),
            };
            cta.map(|label| button("quest-cta", id, label, ButtonKind::Secondary))
                .into_iter()
                .chain([button("quest-accept", id, accept, ButtonKind::Primary)])
                .collect()
        }
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .w_full()
        .h(rpx(CARD_FOOTER))
        .p(rpx(12.))
        .border_t_1()
        .border_color(rgba(theme::border_subtle()))
        .children(buttons)
}

fn media(path: &str, width: u32, height: u32, size: f32, cx: &App) -> Option<Arc<std::path::Path>> {
    images::resolve(
        &ImageSpec::Media {
            url: asset_url(path),
            width,
            height,
            animated: false,
        },
        theme::image_px(size),
        cx,
    )
}

fn hero_button(icon: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(24.))
        .rounded_full()
        .bg(rgba(0x1E1F227A))
        .child(theme::icon(icon, 14., 0xFFFFFF))
}

fn card_header(view: &View, width: f32, hovered: bool, cx: &App) -> Div {
    let config = &view.quest.config;
    let hero_height = (width * HERO_ASPECT).min(CARD_HEADER);
    let hero_clip = config
        .assets
        .hero_video
        .as_deref()
        .filter(|path| hovered && !cx.reduce_motion() && path.ends_with(".mp4"))
        .and_then(|path| {
            crate::media::video::resolve(
                &ImageSpec::Video {
                    url: asset_url(path),
                    width: 1320,
                    height: 370,
                },
                theme::image_px(width),
                cx,
            )
        })
        .map(|frames| {
            crate::media::animated::image(frames)
                .size_full()
                .rounded_t(rpx(CARD_RADIUS - 1.))
                .object_fit(ObjectFit::Cover)
                .id(("quest-hero-clip", view.quest.id))
                .into_any_element()
        });
    let hero = hero_clip.or_else(|| {
        config
            .assets
            .hero
            .as_deref()
            .and_then(|path| media(path, 1320, 370, width, cx))
            .map(|hero| {
                img(hero)
                    .size_full()
                    .rounded_t(rpx(CARD_RADIUS - 1.))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
    });
    let logotype = config
        .assets
        .logotype_dark
        .as_deref()
        .or(config.assets.logotype.as_deref())
        .and_then(|path| media(path, 0, 0, 121., cx));
    let publisher = &config.messages.game_publisher;
    let ends = (!matches!(view.state, State::Claimed | State::Ended))
        .then(|| format!("Ends {}", month_day(&config.expires_at)));
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .rounded_t(rpx(CARD_RADIUS - 1.))
        .bg(rgb(theme::surface_high()))
        .children(hero.map(|hero| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(rpx(hero_height))
                .child(hero)
        }))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(rpx(hero_height))
                .rounded_t(rpx(CARD_RADIUS - 1.))
                .bg(linear_gradient(
                    0.,
                    linear_color_stop(rgb(theme::surface_high()), 0.2),
                    linear_color_stop(rgba(theme::surface_high() << 8), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .p(rpx(12.))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(rpx(8.))
                        .when(view.is_video(), |this| this.child(hero_button("play")))
                        .child(hero_button("ellipsis").hover(|style| style.opacity(0.8))),
                )
                .child(div().flex_1())
                .children(logotype.map(|logotype| {
                    img(logotype)
                        .flex_none()
                        .h(rpx(34.))
                        .max_w(rpx(121.))
                        .mb(rpx(4.))
                        .object_fit(ObjectFit::Contain)
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rpx(4.))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .min_w_0()
                                .hover(|style| style.opacity(0.8))
                                .child(div().flex_none().child("Promoted by"))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(rpx(16.))
                                        .mx(rpx(3.))
                                        .rounded_full()
                                        .bg(rgb(POSITIVE))
                                        .child(theme::icon("check", 11., 0xFFFFFF)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_color(white())
                                        .child(publisher.clone()),
                                ),
                        )
                        .children(ends.map(|ends| div().flex_none().child(ends))),
                ),
        )
}

fn progress_ring(progress: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let unit = bounds.size.width / px(RING);
            let radius = (RING - RING_STROKE) / 2.;
            let center = bounds.center();
            let at = |angle: f32| {
                let (sin, cos) = angle.sin_cos();
                center + point(px(radius * cos * unit), px(radius * sin * unit))
            };
            let arc = |from: f32, sweep: f32| {
                let steps = ((sweep / TAU) * 96.).ceil().max(2.) as usize;
                let mut path = PathBuilder::stroke(px(RING_STROKE) * unit);
                path.move_to(at(from));
                for step in 1..=steps {
                    path.line_to(at(from + sweep * step as f32 / steps as f32));
                }
                path.build().ok()
            };
            if let Some(track) = arc(0., TAU) {
                window.paint_path(track, rgba(MOD_STRONG));
            }
            if progress > 0.
                && let Some(done) = arc(-FRAC_PI_2, TAU * progress)
            {
                window.paint_path(done, rgb(POSITIVE));
            }
        },
    )
    .absolute()
    .inset_0()
}

struct RewardArt {
    still: String,
    clip: Option<String>,
    side: u32,
    mark: &'static str,
    mark_background: Hsla,
}

impl RewardArt {
    fn orb() -> Self {
        Self {
            still: ORB_STILL.into(),
            clip: Some(ORB_VIDEO.into()),
            side: ORB_ART_SIZE,
            mark: "orb",
            mark_background: rgb(ORB_PURPLE).into(),
        }
    }

    fn asset(still: &str, clip: Option<&str>) -> Self {
        let url = asset_url(still);
        Self {
            still: if still.ends_with(".mp4") || still.ends_with(".webm") {
                format!("{url}?format=webp")
            } else {
                url
            },
            clip: clip.filter(|path| path.ends_with(".mp4")).map(asset_url),
            side: 0,
            mark: "award",
            mark_background: rgba(MOD_STRONG).into(),
        }
    }
}

fn reward_art(
    art: RewardArt,
    quest_id: u64,
    playing: bool,
    size: f32,
    radius: f32,
    cx: &App,
) -> AnyElement {
    let pixels = theme::image_px(size);
    let clip = art
        .clip
        .filter(|_| playing && !cx.reduce_motion())
        .and_then(|url| {
            crate::media::video::resolve(
                &ImageSpec::Video {
                    url,
                    width: art.side,
                    height: art.side,
                },
                pixels,
                cx,
            )
        });
    if let Some(frames) = clip {
        return crate::media::animated::image(frames)
            .size(rpx(size))
            .rounded(rpx(radius))
            .object_fit(ObjectFit::Cover)
            .id(("quest-reward-clip", quest_id))
            .into_any_element();
    }
    let still = images::resolve(
        &ImageSpec::Media {
            url: art.still,
            width: art.side,
            height: art.side,
            animated: false,
        },
        pixels,
        cx,
    );
    match still {
        Some(still) => img(still)
            .size(rpx(size))
            .rounded(rpx(radius))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => div()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(size))
            .rounded(rpx(radius))
            .bg(art.mark_background)
            .child(theme::icon(art.mark, size * 0.45, 0xFFFFFF))
            .into_any_element(),
    }
}

fn reward_tile(view: &View, playing: bool, cx: &App) -> AnyElement {
    let enrolled = matches!(view.state, State::Enrolled | State::Completed);
    let size = if enrolled { 60. } else { REWARD_TILE };
    let radius = if enrolled { 4. } else { 9.412 };
    let reward = view.reward();
    let asset = reward.and_then(|reward| reward.asset.as_deref());
    let video = asset.is_some_and(|path| path.ends_with(".mp4") || path.ends_with(".webm"));
    let clip = reward
        .and_then(|reward| reward.asset_video.as_deref())
        .or(asset.filter(|path| path.ends_with(".mp4")));
    let art = if reward.is_some_and(|reward| orbs(reward).is_some()) {
        Some(RewardArt::orb())
    } else if video || clip.is_some() {
        asset.or(clip).map(|still| RewardArt::asset(still, clip))
    } else {
        None
    };
    let tile = match (art, asset) {
        (Some(art), _) => reward_art(art, view.quest.id, playing, size, radius, cx),
        (None, Some(path)) => match media(path, 0, 0, size, cx) {
            Some(picture) => img(picture)
                .size(rpx(size))
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => reward_mark(size, radius),
        },
        (None, None) => reward_mark(size, radius),
    };
    if !enrolled {
        return tile;
    }
    div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(REWARD_TILE))
        .child(tile)
        .child(
            div()
                .absolute()
                .top(rpx((REWARD_TILE - RING) / 2.))
                .left(rpx((REWARD_TILE - RING) / 2.))
                .size(rpx(RING))
                .child(progress_ring(view.progress())),
        )
        .into_any_element()
}

fn reward_mark(size: f32, radius: f32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(size))
        .rounded(rpx(radius))
        .bg(rgba(MOD_STRONG))
        .child(theme::icon("award", size * 0.45, 0xFFFFFF))
        .into_any_element()
}

struct Expansion {
    hovered: bool,
    change: Option<u64>,
    full: f32,
}

fn card_middle(
    view: &View,
    expansion: Expansion,
    heights: &Rc<RefCell<HashMap<u64, f32>>>,
    cx: &App,
) -> AnyElement {
    let quest_id = view.quest.id;
    let name = format!("{} Quest", view.quest.config.messages.quest_name).to_uppercase();
    let overflows = expansion.full > CARD_MIDDLE;
    let heights = heights.clone();
    let text = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .p(rpx(12.))
        .child(
            div()
                .mb(rpx(4.))
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(TEXT_BRAND))
                .child(name),
        )
        .child(
            reward_line(view)
                .mb(rpx(2.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong())),
        )
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::text()))
                .child(description(view)),
        )
        .on_prepaint(move |bounds, window, _| {
            let unit = rpx(1.).to_pixels(window.rem_size());
            heights
                .borrow_mut()
                .insert(quest_id, bounds.size.height / unit);
        });
    let middle = div()
        .relative()
        .flex()
        .flex_none()
        .items_start()
        .w_full()
        .overflow_hidden()
        .bg(rgb(theme::surface_high()))
        .child(
            div()
                .flex_none()
                .pt(rpx(12.))
                .pl(rpx(12.))
                .child(reward_tile(view, expansion.hovered, cx)),
        )
        .child(text);
    let blur = |opacity: f32| {
        div()
            .absolute()
            .inset_0()
            .opacity(opacity)
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgba(theme::base_lower() << 8), 0.5),
                linear_color_stop(rgb(theme::base_lower()), 1.),
            ))
    };
    let (from, to) = if expansion.hovered {
        (CARD_MIDDLE, expansion.full)
    } else {
        (expansion.full, CARD_MIDDLE)
    };
    let size = move |middle: Div, progress: f32| {
        let height = from + (to - from) * progress;
        let open = (height - CARD_MIDDLE) / (expansion.full - CARD_MIDDLE).max(1.);
        middle
            .h(rpx(height))
            .when(overflows, |middle| middle.child(blur(1. - open)))
    };
    match expansion.change {
        Some(change) if overflows && !cx.reduce_motion() => middle
            .with_animation(
                ElementId::NamedInteger(format!("quest-details-{quest_id}").into(), change),
                Animation::new(DETAILS_GROW).with_easing(ease_out_quint()),
                size,
            )
            .into_any_element(),
        _ => size(middle, 1.).into_any_element(),
    }
}

fn card(app: &DiscordApp, view: &View, width: f32, cx: &App) -> AnyElement {
    let quest_id = view.quest.id;
    let home = &app.home.quests;
    let hovered = home.hovered == Some(quest_id);
    let full = home
        .text_heights
        .borrow()
        .get(&quest_id)
        .map_or(CARD_MIDDLE, |text| (text + 12.).max(CARD_MIDDLE));
    let expansion = Expansion {
        hovered,
        change: if hovered {
            Some(home.hover_changes)
        } else {
            home.left
                .filter(|(left, _)| *left == quest_id)
                .map(|(_, change)| change)
        },
        full,
    };
    let entity = app.entity.clone();
    div()
        .id(("quest-tile", quest_id))
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| app.hover_quest(quest_id, *hovered, cx))
                .ok();
        })
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(width))
        .h(rpx(CARD_HEADER + CARD_MIDDLE + CARD_FOOTER))
        .rounded(rpx(CARD_RADIUS))
        .overflow_hidden()
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_muted()))
        .shadow_sm()
        .child(card_header(view, width, hovered, cx))
        .child(card_middle(view, expansion, &home.text_heights, cx))
        .child(footer(view))
        .into_any_element()
}

fn claimed_quests(
    app: &DiscordApp,
    list: &QuestList,
    cards: &QuestCards,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let mut claimed: Vec<&Quest> = list
        .quests
        .iter()
        .filter(|quest| {
            quest
                .user_status
                .as_ref()
                .is_some_and(|status| status.claimed_at.is_some())
        })
        .collect();
    let claimed_at = |quest: &&Quest| {
        quest
            .user_status
            .as_ref()
            .and_then(|status| status.claimed_at.as_deref())
            .and_then(parse_time)
    };
    claimed.sort_by_key(|quest| std::cmp::Reverse(claimed_at(quest)));
    if claimed.is_empty() {
        return notice(
            "You haven't completed any Quests yet!",
            "Quests you claim rewards for show up here.",
        );
    }
    let (columns, width) = grid_columns(content_width(window), CLAIMED_MIN_WIDTH);
    let width = width.min(CLAIMED_MAX_WIDTH);
    grid(
        claimed
            .into_iter()
            .map(|quest| {
                cards.place(quest.id, true, width, width, || {
                    claimed_card(app, quest, width, cx)
                })
            })
            .collect(),
        columns,
    )
    .into_any_element()
}

fn claimed_card(app: &DiscordApp, quest: &Quest, width: f32, cx: &App) -> AnyElement {
    let view = View::new(quest, Utc::now());
    let playing = app.home.quests.hovered == Some(quest.id);
    let entity = app.entity.clone();
    let quest_id = quest.id;
    let config = &quest.config;
    let logotype = config
        .assets
        .logotype_dark
        .as_deref()
        .or(config.assets.logotype.as_deref())
        .and_then(|path| media(path, 0, 0, 121., cx));
    let reward = view
        .reward()
        .map(|reward| match orbs(reward) {
            Some(count) => format!("{count} Orbs"),
            None => reward.messages.name.clone(),
        })
        .unwrap_or_default();
    let claimed = quest
        .user_status
        .as_ref()
        .and_then(|status| status.claimed_at.as_deref())
        .map(month_day)
        .unwrap_or_default();
    div()
        .id(("quest-claimed", quest.id))
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| app.hover_quest(quest_id, *hovered, cx))
                .ok();
        })
        .flex()
        .flex_col()
        .flex_none()
        .gap(rpx(8.))
        .w(rpx(width))
        .h(rpx(width))
        .p(rpx(16.))
        .rounded(rpx(16.))
        .overflow_hidden()
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_muted()))
        .children(logotype.map(|logotype| {
            img(logotype)
                .flex_none()
                .h(rpx(28.))
                .max_w(rpx(121.))
                .object_fit(ObjectFit::Contain)
        }))
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(reward_tile(&view, playing, cx)),
        )
        .child(
            div()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(format!("{} Quest", config.messages.quest_name)),
        )
        .child(
            div()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::muted()))
                .child(format!("{reward} claimed on {claimed}")),
        )
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
