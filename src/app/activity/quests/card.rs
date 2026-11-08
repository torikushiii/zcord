use super::*;

const DETAILS_GROW: Duration = Duration::from_millis(250);
const HERO_ASPECT: f32 = 370. / 1320.;
const REWARD_TILE: f32 = 80.;
const RING: f32 = 76.;
const RING_STROKE: f32 = 4.;
const TEXT_BRAND: u32 = 0x94A8FF;
const POSITIVE: u32 = 0x23A55A;
const ORB_PURPLE: u32 = 0x5B3FD9;
const ORB_VIDEO: &str = "https://cdn.discordapp.com/assets/content/f763593591e9ac5ee5abab6d8cce6c395263fa51f6c1e30c07cbfc793cd877d2.mp4";
const ORB_STILL: &str = "https://cdn.discordapp.com/assets/content/fb761d9c206f93cd8c4e7301798abe3f623039a4054f2e7accd019e1bb059fc8.webm?format=webp";
const ORB_ART_SIZE: u32 = 320;
const MOD_STRONG: u32 = 0x97979F33;

#[derive(Clone, Copy)]
pub(super) enum ButtonKind {
    Primary,
    Secondary,
    Overlay,
}

pub(super) fn button(
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

pub(super) fn media(
    path: &str,
    width: u32,
    height: u32,
    size: f32,
    cx: &App,
) -> Option<Arc<std::path::Path>> {
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

pub(super) fn card(app: &DiscordApp, view: &View, width: f32, cx: &App) -> AnyElement {
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

pub(super) fn claimed_quests(
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

pub(super) fn claimed_card(app: &DiscordApp, quest: &Quest, width: f32, cx: &App) -> AnyElement {
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
