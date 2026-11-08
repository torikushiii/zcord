use super::super::profile::avatar_colors;
use super::super::*;
use crate::media::images;
use crate::store::ActivityCard;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const JITTER_MAX_MS: u64 = 5_000;

#[derive(Default)]
pub(crate) struct ActivityFeed {
    task: Option<Task<()>>,
    fetch_at: Option<Instant>,
    failures: u32,
    fetching: bool,
    covers: HashMap<u64, String>,
    games_requested: HashSet<u64>,
    colors: HashMap<u64, [u32; 2]>,
    colors_requested: HashSet<u64>,
    expanded: bool,
}

impl DiscordApp {
    /// Fetch on connect (`force`) and on focus events. Never more often than the last
    /// fetch's `wait_ms_until_next_fetch` (or the failure backoff) allows.
    pub(crate) fn refresh_activity_feed(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.window_active || !self.live || self.session.is_none() {
            return;
        }
        if self.activity_feed.task.is_some() || self.activity_feed.fetching {
            return;
        }
        let now = Instant::now();
        let deadline = self.activity_feed.fetch_at.unwrap_or(now);
        if !force && deadline > now {
            return;
        }
        self.schedule_activity_fetch(deadline.max(now), cx);
    }

    fn schedule_activity_fetch(&mut self, deadline: Instant, cx: &mut Context<Self>) {
        self.activity_feed.fetch_at = Some(deadline);
        self.activity_feed.task = Some(cx.spawn(async move |this, cx| {
            let delay = deadline.saturating_duration_since(Instant::now());
            cx.background_executor().timer(delay).await;
            this.update(cx, |app, cx| {
                app.activity_feed.task = None;
                app.activity_feed.fetch_at = None;
                if !app.window_active || !app.live {
                    return;
                }
                let Some(session) = app.session.clone() else {
                    return;
                };
                app.activity_feed.fetching = true;
                let request = Tokio::spawn(cx, async move { session.content_inbox().await });
                cx.spawn(async move |this, cx| {
                    let result = request.await;
                    this.update(cx, |app, cx| app.activity_inbox_result(result, cx))
                        .ok();
                })
                .detach();
            })
            .ok();
        }));
    }

    fn activity_inbox_result(
        &mut self,
        result: Result<
            Result<discord::model::ContentInbox, discord::Error>,
            tokio::task::JoinError,
        >,
        cx: &mut Context<Self>,
    ) {
        self.activity_feed.fetching = false;
        match result {
            Ok(Ok(inbox)) => {
                let wait = inbox.wait_ms_until_next_fetch;
                self.store.set_content_inbox(Some(inbox));
                self.activity_feed.failures = 0;
                let jitter = Duration::from_millis(rand::random::<u64>() % JITTER_MAX_MS);
                self.activity_feed.fetch_at =
                    wait.map(|wait| Instant::now() + Duration::from_millis(wait) + jitter);
                self.load_activity_feed(cx);
                self.schedule_next_activity_fetch(cx);
            }
            Ok(Err(error)) => {
                log::warn!("Could not fetch content inbox: {error}");
                self.activity_feed_failed(cx);
            }
            Err(error) => {
                log::warn!("Content inbox task failed: {error}");
                self.activity_feed_failed(cx);
            }
        }
    }

    /// Back off at 1, 2, 4, 8 minutes plus jitter; the 4th consecutive failure clears the feed.
    fn activity_feed_failed(&mut self, cx: &mut Context<Self>) {
        let failures = self.activity_feed.failures + 1;
        self.activity_feed.failures = failures;
        if failures >= 4 {
            self.store.set_content_inbox(None);
            self.sync_member_rows();
        }
        if failures > 4 {
            self.activity_feed.fetch_at = None;
        } else {
            let jitter = Duration::from_millis(rand::random::<u64>() % JITTER_MAX_MS);
            self.activity_feed.fetch_at =
                Some(Instant::now() + Duration::from_secs(60 << (failures - 1)) + jitter);
        }
        self.views.members.update(cx, |_, cx| cx.notify());
        self.schedule_next_activity_fetch(cx);
    }

    /// A deadline that passes while the window is unfocused waits for the next focus event.
    fn schedule_next_activity_fetch(&mut self, cx: &mut Context<Self>) {
        if let Some(deadline) = self.activity_feed.fetch_at {
            self.schedule_activity_fetch(deadline, cx);
        }
    }

    fn load_activity_feed(&mut self, cx: &mut Context<Self>) {
        self.request_activity_members();
        self.load_activity_assets(cx);
        self.sync_member_rows();
        self.arm_activity_ticker(cx);
        self.views.members.update(cx, |_, cx| cx.notify());
    }

    /// Cheap to repeat: ids already requested are skipped, so call it whenever the visible
    /// cards may change.
    pub(crate) fn load_activity_assets(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self
            .activity_cards()
            .iter()
            .filter_map(|card| card.application_id)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.load_game_apps(ids.clone(), cx);
        self.load_games(ids, cx);
    }

    /// Ticks once a second while the feed is loaded, repainting the member list only when
    /// the selected guild shows a live timer.
    pub(crate) fn arm_activity_ticker(&mut self, cx: &mut Context<Self>) {
        if self.activity_ticker.is_some() {
            return;
        }
        self.activity_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let loaded = this
                    .update(cx, |app, cx| {
                        if app.store.content_inbox().is_none() {
                            app.activity_ticker = None;
                            return false;
                        }
                        if app
                            .activity_cards()
                            .iter()
                            .any(|card| card.live_since_ms.is_some())
                        {
                            app.views.members.update(cx, |_, cx| cx.notify());
                        }
                        true
                    })
                    .unwrap_or(false);
                if !loaded {
                    break;
                }
            }
        }));
    }

    pub(crate) fn request_activity_members(&mut self) {
        if self.selected_guild_id == 0 {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let authors = self.store.activity_authors();
        let missing = self.store.missing_members(self.selected_guild_id, authors);
        if !missing.is_empty() {
            session.request_members(self.selected_guild_id, missing);
        }
    }

    pub(crate) fn activity_expanded(&self) -> bool {
        self.activity_feed.expanded
    }

    pub(crate) fn toggle_activity_expanded(&mut self, cx: &mut Context<Self>) {
        self.activity_feed.expanded = !self.activity_feed.expanded;
        // The list caches row heights, and the Activity section is row 0.
        self.member_rows.remeasure_items(0..1);
        self.views.members.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn activity_cards(&self) -> Vec<ActivityCard> {
        self.store.activity_cards(
            self.selected_guild_id,
            chrono::Utc::now().timestamp_millis(),
        )
    }

    pub(crate) fn load_games(&mut self, ids: Vec<u64>, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let ids: Vec<u64> = ids
            .into_iter()
            .filter(|id| self.activity_feed.games_requested.insert(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let requested = ids.clone();
        let request = Tokio::spawn(cx, async move { session.games(&ids).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(games)) => {
                    for game in games {
                        if let Some(cover) = game.media.cover_url.clone() {
                            app.activity_feed.covers.insert(game.id, cover);
                        }
                    }
                    app.views.members.update(cx, |_, cx| cx.notify());
                }
                Ok(Err(error)) => {
                    log::warn!("Could not load games: {error}");
                    app.forget_requested_games(&requested);
                }
                Err(error) => {
                    log::warn!("Games task failed: {error}");
                    app.forget_requested_games(&requested);
                }
            })
            .ok();
        })
        .detach();
    }

    fn forget_requested_games(&mut self, ids: &[u64]) {
        for id in ids {
            self.activity_feed.games_requested.remove(id);
        }
    }

    pub(crate) fn game_cover(&self, application_id: u64) -> Option<String> {
        self.activity_feed.covers.get(&application_id).cloned()
    }

    /// Two-colour palette of the application icon, computed off the main thread once the
    /// icon file is cached; the member list is notified when it lands.
    pub(crate) fn ensure_activity_colors(&mut self, application_id: u64, cx: &mut Context<Self>) {
        if self.activity_feed.colors.contains_key(&application_id)
            || self
                .activity_feed
                .colors_requested
                .contains(&application_id)
        {
            return;
        }
        let Some(spec) = self.game_icon_spec(application_id) else {
            return;
        };
        self.activity_feed.colors_requested.insert(application_id);
        let Some(service) = images::download_service(cx) else {
            return;
        };
        let request = Tokio::spawn(cx, async move {
            let path = service.load(&spec, 128).await?;
            tokio::task::spawn_blocking(move || avatar_colors::load(&path))
                .await
                .ok()
                .flatten()
        });
        cx.spawn(async move |this, cx| {
            let colors = request.await.ok().flatten();
            this.update(cx, |app, cx| {
                if let Some(colors) = colors {
                    app.activity_feed
                        .colors
                        .insert(application_id, colors.map(overlay_color));
                } else {
                    app.activity_feed.colors_requested.remove(&application_id);
                }
                app.views.members.update(cx, |_, cx| cx.notify());
                app.views.messages.update(cx, |_, cx| cx.notify());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn activity_colors(&self, application_id: u64) -> Option<[u32; 2]> {
        self.activity_feed.colors.get(&application_id).copied()
    }

    /// Replies to (or reacts on) an inbox entry in its author's DM, without navigating.
    pub(crate) fn send_activity_reply(
        &mut self,
        entry_id: u64,
        content: String,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self
            .store
            .content_inbox()
            .and_then(|inbox| inbox.entries.iter().find(|entry| entry.id == entry_id))
            .cloned()
        else {
            return;
        };
        let Some(user_id) = entry.author_id else {
            return;
        };
        let raw = entry.raw.clone();
        let entity = cx.entity();
        let handle = cx.active_window();
        if let Some(channel_id) = self.store.dm_channel_with(user_id) {
            cx.spawn(async move |_, cx| {
                if let Some(handle) = handle
                    && handle
                        .update(cx, move |_, window, cx| {
                            entity.update(cx, |app, cx| {
                                app.submit_send(
                                    channel_id,
                                    content,
                                    None,
                                    Vec::new(),
                                    Some(raw),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .is_err()
                {
                    log::warn!("Activity reply dropped; window closed");
                }
            })
            .detach();
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, async move { session.open_dm(user_id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let pending = this
                .update(cx, |app, _| match result {
                    Ok(Ok(channel)) => {
                        let channel_id = channel.id;
                        let change = app.store.apply(Event::ChannelUpsert(Box::new(channel)));
                        Some((change, channel_id))
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not open DM for activity reply: {error}");
                        None
                    }
                    Err(error) => {
                        log::warn!("Open DM task failed: {error}");
                        None
                    }
                })
                .ok()
                .flatten();
            if let Some((change, channel_id)) = pending
                && let Some(handle) = handle
            {
                handle
                    .update(cx, move |_, window, cx| {
                        entity.update(cx, |app, cx| {
                            app.apply_change(change, window, cx);
                            app.submit_send(
                                channel_id,
                                content,
                                None,
                                Vec::new(),
                                Some(raw),
                                window,
                                cx,
                            );
                        });
                    })
                    .ok();
            }
        })
        .detach();
    }
}

fn overlay_color(color: u32) -> u32 {
    let mut color = color & 0xFF_FFFF;
    for _ in 1..8 {
        if darkness(color) >= 0.725 {
            break;
        }
        let [l, a, b] = rgb_to_lab(color);
        color = lab_to_rgb([l - 9., a, b]);
    }
    color
}

fn darkness(color: u32) -> f64 {
    let [r, g, b] = channels(color);
    1. - (0.299 * r + 0.587 * g + 0.114 * b) / 255.
}

fn channels(color: u32) -> [f64; 3] {
    [
        f64::from((color >> 16) & 0xFF),
        f64::from((color >> 8) & 0xFF),
        f64::from(color & 0xFF),
    ]
}

const WHITE: [f64; 3] = [0.950_470, 1., 1.088_830];
const T0: f64 = 4. / 29.;
const T1: f64 = 6. / 29.;
const T2: f64 = 3. * T1 * T1;
const T3: f64 = T1 * T1 * T1;

fn rgb_to_lab(color: u32) -> [f64; 3] {
    let [r, g, b] = channels(color).map(|channel| {
        let channel = channel / 255.;
        if channel <= 0.040_45 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    let f = |t: f64| if t > T3 { t.cbrt() } else { t / T2 + T0 };
    let x = f((0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / WHITE[0]);
    let y = f((0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b) / WHITE[1]);
    let z = f((0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / WHITE[2]);
    let l = 116. * y - 16.;
    [l.max(0.), 500. * (x - y), 200. * (y - z)]
}

fn lab_to_rgb([l, a, b]: [f64; 3]) -> u32 {
    let f = |t: f64| if t > T1 { t * t * t } else { T2 * (t - T0) };
    let y = (l + 16.) / 116.;
    let x = WHITE[0] * f(y + a / 500.);
    let z = WHITE[2] * f(y - b / 200.);
    let y = WHITE[1] * f(y);
    let gamma = |channel: f64| {
        let channel = if channel <= 0.003_130_8 {
            12.92 * channel
        } else {
            1.055 * channel.powf(1. / 2.4) - 0.055
        };
        (channel * 255.).round().clamp(0., 255.) as u32
    };
    let r = gamma(3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z);
    let g = gamma(-0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z);
    let b = gamma(0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z);
    (r << 16) | (g << 8) | b
}
