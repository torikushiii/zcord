use std::time::Instant;

use super::active_now::active_now;
use super::*;

const ACTION_SIZE: f32 = 36.;
const ROW_MOTION: Duration = Duration::from_millis(240);
const ROW_STAGGER: Duration = Duration::from_millis(30);
const ROW_STAGGER_LIMIT: u32 = 8;
const ROW_HEIGHT: f32 = 68.;
const DIVIDER_HEIGHT: f32 = 1.;
const ROW_PITCH: f32 = ROW_HEIGHT + DIVIDER_HEIGHT;
// Above ROW_PITCH, so a finished row is never clipped.
const ROW_MOTION_HEIGHT: f32 = 76.;
const STACK_APPEAR: f32 = 0.06;
const STACK_HOLD: f32 = 0.08;
const STACK_TRAVEL: f32 = 0.28;
const STACK_STAGGER: f32 = 0.02;
const STACK_STAGGER_LIMIT: usize = 8;
const STACK_TOTAL: f32 = STACK_HOLD + STACK_STAGGER * STACK_STAGGER_LIMIT as f32 + STACK_TRAVEL;
const STACK_GROUPS: usize = 4;
const STACK_PEEK: f32 = 8.;
const STACK_INSET: f32 = 12.;

#[derive(Clone, Copy, Default)]
pub(crate) enum OnlineStack {
    #[default]
    Waiting,
    Running(Instant),
    Done,
}

fn online_stack_start(home: &Home, has_rows: bool, still: bool) -> Option<Instant> {
    match home.online_stack.get() {
        OnlineStack::Waiting | OnlineStack::Running(_) if still => {
            home.online_stack.set(OnlineStack::Done);
            None
        }
        OnlineStack::Waiting if has_rows => {
            let now = Instant::now();
            home.online_stack.set(OnlineStack::Running(now));
            Some(now)
        }
        OnlineStack::Running(start) if start.elapsed().as_secs_f32() < STACK_TOTAL => Some(start),
        OnlineStack::Running(_) => {
            home.online_stack.set(OnlineStack::Done);
            None
        }
        OnlineStack::Waiting | OnlineStack::Done => None,
    }
}

struct StackPose {
    lift: f32,
    inset: f32,
    opacity: f32,
    surface: f32,
    divider: f32,
}

fn ease_out_back(delta: f32) -> f32 {
    const OVERSHOOT: f32 = 0.9;
    let delta = delta - 1.;
    1. + (OVERSHOOT + 1.) * delta.powi(3) + OVERSHOOT * delta.powi(2)
}

fn stack_group_sizes(count: usize) -> impl Iterator<Item = usize> {
    (0..STACK_GROUPS)
        .map(move |group| count / STACK_GROUPS + usize::from(group < count % STACK_GROUPS))
        .filter(|size| *size > 0)
}

struct FoldedSlot {
    top: f32,
    index_in_group: usize,
}

fn folded_slot(index: usize, count: usize) -> FoldedSlot {
    let mut first = 0;
    let mut top = 0.;
    for size in stack_group_sizes(count) {
        if index < first + size {
            let index_in_group = index - first;
            return FoldedSlot {
                top: top + STACK_PEEK * index_in_group.min(1) as f32,
                index_in_group,
            };
        }
        first += size;
        top += ROW_PITCH + if size > 1 { STACK_PEEK } else { 0. };
    }
    FoldedSlot {
        top,
        index_in_group: 0,
    }
}

fn stack_pose(index: usize, count: usize, start: Instant) -> StackPose {
    let elapsed = start.elapsed().as_secs_f32();
    let FoldedSlot {
        top: folded,
        index_in_group: member,
    } = folded_slot(index, count);
    let delay = STACK_HOLD + STACK_STAGGER * member.min(STACK_STAGGER_LIMIT) as f32;
    let travel = ((elapsed - delay) / STACK_TRAVEL).clamp(0., 1.);
    let spread = ease_out_back(travel);
    let resting = match member {
        0 => 1.,
        1 => 0.6,
        _ => 0.,
    };
    StackPose {
        lift: (folded - ROW_PITCH * index as f32) * (1. - spread),
        inset: STACK_INSET * member.min(1) as f32 * (1. - spread.min(1.)),
        opacity: (elapsed / STACK_APPEAR).min(1.)
            * (resting + (1. - resting) * (travel * 2.).min(1.)),
        surface: 1. - ease_out_quint()(travel),
        divider: ((travel - 0.6) / 0.4).clamp(0., 1.),
    }
}

fn stack_animation() -> Animation {
    Animation::new(Duration::from_secs_f32(STACK_TOTAL))
}

fn stacked_card(
    row: Stateful<Div>,
    user_id: u64,
    (index, count): (usize, usize),
    start: Instant,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(row.mx_0())
        .with_animation(
            ("friend-stack", user_id),
            stack_animation(),
            move |card, _| {
                let pose = stack_pose(index, count, start);
                let surface = pose.surface;
                card.relative()
                    .top(rpx(pose.lift))
                    .ml(rpx(14. + pose.inset))
                    .mr(rpx(10. + pose.inset))
                    .opacity(pose.opacity)
                    .rounded(rpx(8.))
                    .bg(rgb(theme::elevated()).opacity(surface))
                    .shadow(vec![
                        BoxShadow::new(
                            px(0.),
                            px(0.),
                            rgba(theme::border()).opacity(surface).into(),
                        )
                        .spread_radius(px(1.)),
                        BoxShadow::new(px(0.), px(4.), hsla(0., 0., 0., 0.24 * surface))
                            .blur_radius(px(12.)),
                    ])
            },
        )
        .into_any_element()
}

fn stacked_divider(
    divider: Div,
    user_id: u64,
    (index, count): (usize, usize),
    start: Instant,
) -> AnyElement {
    divider
        .with_animation(
            ("friend-stack-divider", user_id),
            stack_animation(),
            move |divider, _| divider.opacity(stack_pose(index, count, start).divider),
        )
        .into_any_element()
}

#[derive(Default)]
pub(crate) struct RowMotion {
    key: Option<(FriendsTab, String)>,
    present: HashSet<u64>,
    shown: Vec<u64>,
    entering: HashMap<u64, Entering>,
    leaving: Vec<Leaving>,
    serial: u64,
}

struct Entering {
    serial: u64,
    delay: Duration,
    since: Instant,
}

struct Leaving {
    id: u64,
    serial: u64,
    after: Option<u64>,
    index: usize,
    since: Instant,
}

#[derive(Clone, Copy)]
enum Motion {
    Still,
    Enter { serial: u64, delay: Duration },
    Leave { serial: u64 },
}

impl RowMotion {
    pub(crate) fn forget(&mut self) {
        self.key = None;
    }

    fn rows(&mut self, key: (FriendsTab, String), ids: &[u64], still: bool) -> Vec<(u64, Motion)> {
        if still || self.key.as_ref() != Some(&key) {
            *self = Self {
                key: Some(key),
                present: ids.iter().copied().collect(),
                shown: ids.to_vec(),
                serial: self.serial,
                ..Self::default()
            };
            return ids.iter().map(|id| (*id, Motion::Still)).collect();
        }
        let now = Instant::now();
        let current: HashSet<u64> = ids.iter().copied().collect();
        self.entering
            .retain(|_, entering| now.duration_since(entering.since) < entering.delay + ROW_MOTION);
        self.leaving.retain(|leaving| {
            now.duration_since(leaving.since) < ROW_MOTION && !current.contains(&leaving.id)
        });
        let mut after = None;
        for (index, id) in self.shown.iter().copied().enumerate() {
            if self.present.contains(&id) && !current.contains(&id) {
                self.serial += 1;
                self.entering.remove(&id);
                self.leaving.push(Leaving {
                    id,
                    serial: self.serial,
                    after,
                    index,
                    since: now,
                });
            }
            after = Some(id);
        }
        let arrived: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|id| !self.present.contains(id))
            .collect();
        for (batch, id) in (0u32..).zip(arrived) {
            self.serial += 1;
            self.entering.insert(
                id,
                Entering {
                    serial: self.serial,
                    delay: ROW_STAGGER * batch.min(ROW_STAGGER_LIMIT),
                    since: now,
                },
            );
        }
        let mut rows: Vec<(u64, Motion)> = ids
            .iter()
            .map(|id| {
                let motion =
                    self.entering
                        .get(id)
                        .map_or(Motion::Still, |entering| Motion::Enter {
                            serial: entering.serial,
                            delay: entering.delay,
                        });
                (*id, motion)
            })
            .collect();
        for leaving in &self.leaving {
            let at = match leaving.after {
                None => 0,
                Some(after) => rows
                    .iter()
                    .position(|(id, _)| *id == after)
                    .map_or(leaving.index.min(rows.len()), |index| index + 1),
            };
            rows.insert(
                at,
                (
                    leaving.id,
                    Motion::Leave {
                        serial: leaving.serial,
                    },
                ),
            );
        }
        self.present = current;
        self.shown = rows.iter().map(|(id, _)| *id).collect();
        rows
    }
}

fn reveal(frame: Div, shown: f32) -> Div {
    frame.max_h(rpx(ROW_MOTION_HEIGHT * shown)).opacity(shown)
}

fn moving_row(divider: Option<Div>, row: AnyElement, motion: Motion) -> AnyElement {
    let frame = div()
        .flex()
        .flex_col()
        .flex_none()
        .overflow_hidden()
        .children(divider)
        .child(row);
    match motion {
        Motion::Still => frame.into_any_element(),
        Motion::Enter { serial, delay } => {
            let total = delay + ROW_MOTION;
            let lead = delay.as_secs_f32() / total.as_secs_f32();
            let ease = ease_out_quint();
            frame
                .with_animation(
                    ("friend-row-enter", serial),
                    Animation::new(total)
                        .with_easing(move |delta| ease(((delta - lead) / (1. - lead)).max(0.))),
                    reveal,
                )
                .into_any_element()
        }
        Motion::Leave { serial } => frame
            .with_animation(
                ("friend-row-leave", serial),
                Animation::new(ROW_MOTION).with_easing(ease_out_quint()),
                |frame, delta| reveal(frame, 1. - delta),
            )
            .into_any_element(),
    }
}

pub(crate) fn friends_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |tab: FriendsTab, badge: u32| {
        let selected = app.home.tab == tab;
        let entity = app.entity.clone();
        div()
            .id(tab.label())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .min_w(rpx(40.))
            .gap(rpx(8.))
            .h(rpx(28.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .text_color(rgb(if selected {
                theme::strong()
            } else {
                theme::muted()
            }))
            .when(selected, |this| this.bg(rgba(theme::selected())))
            .when(!selected, |this| {
                this.hover(|style| {
                    style
                        .bg(rgba(theme::hover()))
                        .text_color(rgb(theme::text()))
                })
            })
            .child(tab.label())
            .when(badge > 0, |this| this.child(count_pill(badge)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_friends_tab(tab, cx))
                    .ok();
            })
    };
    let suggestions = app.store.friend_suggestion_count();
    let adding = app.home.tab == FriendsTab::Add;
    let open_add = app.entity.clone();
    let add_tab = div()
        .id("friends-add-tab")
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(28.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if adding { theme::BLURPLE } else { 0xFFFFFF }))
        .when(adding, |this| this.bg(rgba(theme::selected())))
        .when(!adding, |this| {
            this.bg(rgb(theme::BLURPLE))
                .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
        })
        .child(FriendsTab::Add.label())
        .on_click(move |_, _, cx| {
            open_add
                .update(cx, |app, cx| app.select_friends_tab(FriendsTab::Add, cx))
                .ok();
        });
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("users", 20., theme::faint()))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Friends"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::faint())),
        )
        .child(tab(FriendsTab::Online, 0))
        .child(tab(FriendsTab::All, 0))
        .child(tab(FriendsTab::Pending, app.store.pending_count()))
        .when(
            suggestions > 0 || app.home.tab == FriendsTab::Suggestions,
            |this| this.child(tab(FriendsTab::Suggestions, suggestions)),
        )
        .child(add_tab)
}

fn search_query(app: &DiscordApp, cx: &App) -> String {
    app.home
        .search
        .as_ref()
        .map(|search| search.read(cx).value().trim().to_lowercase())
        .unwrap_or_default()
}

fn status_text(user: Option<&User>, kind: RelationshipKind) -> String {
    match kind {
        RelationshipKind::Incoming => return "Incoming Friend Request".into(),
        RelationshipKind::Outgoing => return "Outgoing Friend Request".into(),
        _ => {}
    }
    let Some(user) = user else {
        return "Offline".into();
    };
    if let Some(activity) = user.activity.as_ref().filter(|_| user.status.is_online()) {
        return activity.clone();
    }
    match user.status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Offline",
    }
    .into()
}

pub(crate) fn friends_page(app: &DiscordApp, cx: &App) -> AnyElement {
    let query = search_query(app, cx);
    let tab = app.home.tab;
    if tab == FriendsTab::Add {
        app.home.row_motion.borrow_mut().forget();
        return div()
            .flex()
            .size_full()
            .child(friends::add_friend_page(app))
            .child(active_now(app, cx))
            .into_any_element();
    }
    let ids = match tab {
        FriendsTab::Online => app.store.online_friends(),
        FriendsTab::All => app.store.friends(),
        FriendsTab::Pending => app.store.pending_requests(),
        FriendsTab::Suggestions => app
            .store
            .friend_suggestions()
            .into_iter()
            .map(|suggestion| suggestion.user_id)
            .collect(),
        FriendsTab::Add => Vec::new(),
    };
    let ids: Vec<u64> = ids
        .into_iter()
        .filter(|id| {
            query.is_empty()
                || app.store.friend_name(*id).to_lowercase().contains(&query)
                || app
                    .store
                    .user(*id)
                    .is_some_and(|user| user.username.to_lowercase().contains(&query))
        })
        .collect();
    let heading = format!(
        "{} — {}",
        match tab {
            FriendsTab::Online => "Online",
            FriendsTab::All => "All friends",
            FriendsTab::Pending => "Pending",
            FriendsTab::Suggestions => "Friend Suggestions",
            FriendsTab::Add => "",
        },
        ids.len()
    );
    let empty = ids.is_empty();
    let reduce_motion = cx.reduce_motion();
    let stack = (tab == FriendsTab::Online)
        .then(|| online_stack_start(&app.home, !empty, reduce_motion))
        .flatten();
    let shown =
        app.home
            .row_motion
            .borrow_mut()
            .rows((tab, query), &ids, reduce_motion || stack.is_some());
    let mut rows: Vec<AnyElement> = Vec::with_capacity(shown.len() * 2);
    for (index, (id, motion)) in shown.iter().copied().enumerate() {
        let divider = (index > 0).then(|| row_divider(app, shown[index - 1].0, id));
        if let Some(start) = stack {
            let slot = (index, shown.len());
            rows.extend(divider.map(|divider| stacked_divider(divider, id, slot, start)));
            rows.push(stacked_card(friend_row(app, id, cx), id, slot, start));
            continue;
        }
        let row = match tab {
            FriendsTab::Suggestions => suggestion_row(app, id, cx).into_any_element(),
            _ => friend_row(app, id, cx).into_any_element(),
        };
        match motion {
            Motion::Still => {
                rows.extend(divider.map(IntoElement::into_any_element));
                rows.push(row);
            }
            motion => rows.push(moving_row(divider, row, motion)),
        }
    }
    // Later children paint on top, so the stack is laid out reversed to keep the first card in front.
    if stack.is_some() {
        rows.reverse();
    }
    div()
        .flex()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .children(app.home.search.as_ref().map(|search| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .mt(rpx(12.))
                        .mb(rpx(8.))
                        .ml(rpx(24.))
                        .mr(rpx(16.))
                        .h(rpx(36.))
                        .px(rpx(10.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::elevated()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(search)
                                    .appearance(false)
                                    .px_0()
                                    .text_size(rpx(16.)),
                            ),
                        )
                        .child(theme::icon("search", 18., theme::muted()))
                }))
                .child(
                    div()
                        .flex_none()
                        .pt(rpx(16.))
                        .px(rpx(30.))
                        .pb(rpx(8.))
                        .border_b_1()
                        .border_color(rgba(theme::border_subtle()))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child(heading),
                )
                .children(
                    matches!(tab, FriendsTab::Pending | FriendsTab::Suggestions)
                        .then(|| app.pending_error())
                        .flatten()
                        .map(|text| {
                            div()
                                .flex_none()
                                .px(rpx(30.))
                                .pt(rpx(8.))
                                .text_size(rpx(14.))
                                .text_color(rgb(theme::RED))
                                .child(text)
                        }),
                )
                .child(
                    div()
                        .id("friends-list")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pb(rpx(8.))
                        .map(|this| {
                            if stack.is_some() {
                                this.child(
                                    div().flex().flex_col_reverse().flex_none().children(rows),
                                )
                            } else {
                                this.children(rows)
                            }
                        })
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .mt(rpx(40.))
                                    .text_center()
                                    .text_size(rpx(14.))
                                    .text_color(rgb(theme::muted()))
                                    .child(match tab {
                                        FriendsTab::Pending => {
                                            "There are no pending friend requests."
                                        }
                                        FriendsTab::Suggestions
                                            if !app.store.friend_suggestions_loaded() =>
                                        {
                                            "Loading suggestions…"
                                        }
                                        FriendsTab::Suggestions => {
                                            "No friend suggestions right now."
                                        }
                                        _ => "No one's around to play with Wumpus.",
                                    }),
                            )
                        }),
                ),
        )
        .child(active_now(app, cx))
        .into_any_element()
}

fn round_action(
    id: impl Into<ElementId>,
    icon: &'static str,
    row_group: impl Into<SharedString>,
) -> Stateful<Div> {
    let row_group = row_group.into();
    let group = SharedString::from(format!("{row_group}-{icon}"));
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(ACTION_SIZE))
        .rounded_full()
        .bg(rgb(theme::base_lower()))
        .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
        .cursor_pointer()
        .active(|style| style.bg(rgba(theme::active())))
        .child(
            theme::icon(icon, 20., theme::text())
                .group_hover(group, |style| style.text_color(rgb(theme::strong()))),
        )
}

fn muted_action(
    id: (&'static str, u64),
    icon: &'static str,
    tip: &'static str,
    row_group: SharedString,
) -> Stateful<Div> {
    let tip_key = (tip, id.1);
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(rpx(ACTION_SIZE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .rounded_full()
                .opacity(0.3)
                .bg(rgb(theme::base_lower()))
                .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
                .child(theme::icon(icon, 20., theme::text())),
        )
        .child(theme::hover_tooltip(tip_key, tip))
        .on_click(|_, _, cx| cx.stop_propagation())
}

fn row_group(user_id: u64) -> SharedString {
    format!("people-row-{user_id}").into()
}

fn row_divider(app: &DiscordApp, above: u64, below: u64) -> Div {
    let hidden = matches!(app.home.hovered_row, Some(id) if id == above || id == below);
    div()
        .flex_none()
        .h(rpx(1.))
        .ml(rpx(30.))
        .mr(rpx(20.))
        .when(!hidden, |this| this.bg(rgba(theme::border_subtle())))
}

pub(super) fn people_row(app: &DiscordApp, id: ElementId, user_id: u64) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .group(row_group(user_id))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .ml(rpx(14.))
        .mr(rpx(10.))
        .py(rpx(16.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .hover(|style| style.bg(rgba(theme::hover())))
        .without_text_selection()
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| app.hover_people_row(user_id, *hovered, cx))
                .ok();
        })
}

fn name_line(user_id: u64, name: &str, username: Option<String>, tag: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("people-name", user_id), name)),
        )
        .children(tag)
        .children(username.map(|username| {
            div()
                .invisible()
                .group_hover(row_group(user_id), |style| style.visible())
                .ml(rpx(4.))
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::text()))
                .child(username)
        }))
}

fn subtext_line(id: (&'static str, u64), text: &str, icon: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::muted()))
        .children(icon.map(|icon| theme::icon(icon, 12., theme::GREEN)))
        .child(div().flex_1().min_w_0().truncate().child(label(id, text)))
}

fn friend_row(app: &DiscordApp, user_id: u64, cx: &App) -> Stateful<Div> {
    let user = app.store.user(user_id);
    let kind = app
        .store
        .relationship(user_id)
        .map_or(RelationshipKind::Friend, |relationship| relationship.kind);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| crate::settings::shown_username(&user.username));
    let status = user.map_or(Status::Offline, |user| user.status);
    let icon = user
        .filter(|user| user.status.is_online())
        .and_then(theme::activity_icon);
    let line = status_text(user, kind);
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx).into_any_element());
    let entity = app.entity.clone();
    let group = row_group(user_id);
    let pending = matches!(
        kind,
        RelationshipKind::Incoming | RelationshipKind::Outgoing
    );
    people_row(app, ("friend", user_id).into(), user_id)
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| {
                    if pending {
                        app.open_full_profile_for(user_id, cx);
                    } else {
                        app.open_dm(user_id, window, cx);
                    }
                })
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            DM_AVATAR,
            Some((status, theme::bg())),
            false,
            cx,
        ))
        .child(div().flex().flex_col().flex_1().min_w_0().map(|this| {
            if pending {
                let username = username.unwrap_or(line);
                this.child(name_line(user_id, &name, None, tag))
                    .child(subtext_line(("friend-status", user_id), &username, None))
            } else {
                this.child(name_line(user_id, &name, username, tag))
                    .child(subtext_line(("friend-status", user_id), &line, icon))
            }
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .when(kind == RelationshipKind::Incoming, |this| {
                    this.child(muted_action(
                        ("friend-accept", user_id),
                        "check",
                        "Accept friend requests from Discord for now",
                        group.clone(),
                    ))
                })
                .when(pending, |this| {
                    let tip = if kind == RelationshipKind::Incoming {
                        "Ignore friend requests from Discord for now"
                    } else {
                        "Cancel friend requests from Discord for now"
                    };
                    this.child(muted_action(
                        ("friend-drop", user_id),
                        "x",
                        tip,
                        group.clone(),
                    ))
                })
                .when(!pending, |this| {
                    this.child(round_action(
                        ("friend-message", user_id),
                        "message-circle",
                        group.clone(),
                    ))
                }),
        )
}

fn suggestion_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| user.username.clone()).unwrap_or_default();
    let reason = app
        .store
        .friend_suggestions()
        .into_iter()
        .find(|suggestion| suggestion.user_id == user_id)
        .and_then(|suggestion| suggestion.reason.clone())
        .unwrap_or_else(|| username.clone());
    let group = row_group(user_id);
    let entity = app.entity.clone();
    people_row(app, ("suggestion", user_id).into(), user_id)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.open_full_profile_for(user_id, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user, user_id, DM_AVATAR, None, false, cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(name_line(user_id, &name, None, None))
                .child(subtext_line(("suggestion-reason", user_id), &reason, None)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .child(muted_action(
                    ("suggestion-add", user_id),
                    "check",
                    "Send friend requests from Discord for now",
                    group.clone(),
                ))
                .child(muted_action(
                    ("suggestion-dismiss", user_id),
                    "x",
                    "Dismiss suggestions from Discord for now",
                    group,
                )),
        )
}
