mod cosmetics;
mod requests;

use super::*;

impl DiscordApp {
    pub(super) fn profile_slot(&mut self, panel: bool) -> Option<&mut ProfileCard> {
        if panel {
            self.dm_profile.as_mut()
        } else {
            self.profile.as_mut()
        }
    }

    pub(super) fn active_card(&mut self) -> Option<&mut ProfileCard> {
        self.profile.as_mut().or(self.dm_profile.as_mut())
    }

    pub(crate) fn open_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        cx: &mut Context<Self>,
    ) {
        self.open_profile_at(user_id, position, anchor, ProfileOrigin::Member, cx);
    }

    pub(crate) fn open_account_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        panel: AccountPanelBounds,
        cx: &mut Context<Self>,
    ) {
        self.open_profile_at(
            user_id,
            position,
            Anchor::BottomLeft,
            ProfileOrigin::Account(panel),
            cx,
        );
    }

    fn open_profile_at(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        origin: ProfileOrigin,
        cx: &mut Context<Self>,
    ) {
        let guild_id = match &origin {
            ProfileOrigin::Account(_) => None,
            ProfileOrigin::Member => {
                (self.selected_guild_id != 0).then_some(self.selected_guild_id)
            }
        };
        self.drop_styles_editor();
        self.profile = Some(ProfileCard::new(
            user_id, guild_id, position, anchor, origin,
        ));
        self.load_profile_scope(cx);
        cx.notify();
    }

    pub(crate) fn set_profile_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile
            && card.more_menu_open != open
        {
            card.more_menu_open = open;
            cx.notify();
        }
    }

    pub(crate) fn switch_profile_scope(&mut self, cx: &mut Context<Self>) {
        if self.profile_styles.is_some() {
            if let Some(card) = &self.profile
                && let Some(guild_id) = card.source_guild_id
            {
                let scope = if card.guild_id.is_some() {
                    styles::StyleScope::Main
                } else {
                    styles::StyleScope::Guild(guild_id)
                };
                self.set_style_scope(scope, cx);
            }
            return;
        }
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.switch_scope() {
            self.load_profile_scope(cx);
        }
        cx.notify();
    }

    pub(super) fn load_profile_scope(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &self.profile else {
            return;
        };
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        if let Some(profile) = self
            .session
            .as_ref()
            .filter(|_| self.live)
            .and_then(|session| session.cached_profile(user_id, guild_id, ProfileKind::Popout))
        {
            self.profile_loaded(user_id, guild_id, ProfileKind::Popout, profile, cx);
        }
        self.load_profile(ProfileKind::Modal, cx);
        self.prefetch_profile_details(false, cx);
        self.start_profile_game_timer(cx);
        if let Some(card) = &mut self.profile {
            card._tasks.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(POPOUT_WAIT).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }));
        }
    }

    pub(crate) fn open_full_profile_for(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.drop_styles_editor();
        self.profile = Some(ProfileCard::new(
            user_id,
            None,
            point(px(0.), px(0.)),
            Anchor::TopRight,
            ProfileOrigin::Member,
        ));
        self.load_profile_scope(cx);
        self.open_full_profile(cx);
    }

    pub(crate) fn open_full_profile(&mut self, cx: &mut Context<Self>) {
        self.open_full_profile_on(Tab::Board, cx);
    }

    pub(super) fn open_full_profile_on(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.profile.is_none()
            && let Some(user_id) = self.dm_profile.as_ref().map(|card| card.user_id)
        {
            self.profile = Some(ProfileCard::new(
                user_id,
                None,
                point(px(0.), px(0.)),
                Anchor::TopRight,
                ProfileOrigin::Member,
            ));
            self.load_profile_scope(cx);
        }
        let Some(card) = &mut self.profile else {
            return;
        };
        card.modal = true;
        card.opened = Instant::now();
        card.effect_started.replace(None);
        card.tab = tab;
        self.open_styles_editor(cx);
        self.load_profile(ProfileKind::Modal, cx);
        self.start_profile_game_timer(cx);
        cx.notify();
    }

    pub(super) fn toggle_profile_widget(&mut self, widget_id: u64, cx: &mut Context<Self>) {
        if let Some(card) = self.active_card() {
            if !card.expanded_widgets.remove(&widget_id) {
                card.expanded_widgets.insert(widget_id);
            }
            let generation = card
                .widget_toggles
                .get(&widget_id)
                .map_or(0, |(generation, _)| generation + 1);
            card.widget_toggles
                .insert(widget_id, (generation, Instant::now()));
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(board::WIDGET_TOGGLE).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
    }

    pub(super) fn open_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.open_full_profile(cx);
        self.select_profile_tab(tab, cx);
    }

    pub(crate) fn hover_profile_art(
        &mut self,
        slot: ArtSlot,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let state = &mut card.hovered_art[slot as usize];
        if *state != hovered {
            *state = hovered;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn hover_profile_stack(
        &mut self,
        panel: bool,
        stack: Stack,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let card = if panel {
            self.dm_profile.as_mut()
        } else {
            self.profile.as_mut()
        };
        let Some(state) = card.map(|card| &mut card.stack_hover[stack as usize]) else {
            return;
        };
        if state.hovered != hovered {
            state.hovered = hovered;
            state.generation += 1;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn hover_profile_cover(
        &mut self,
        key: (u64, u64),
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let next = if hovered {
            Some(key)
        } else {
            card.hovered_cover.filter(|current| *current != key)
        };
        if card.hovered_cover != next {
            card.hovered_cover = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn hover_profile_badge(
        &mut self,
        index: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let next = if hovered {
            Some(index)
        } else {
            card.hovered_badge.filter(|current| *current != index)
        };
        if card.hovered_badge != next {
            card.hovered_badge = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn select_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile {
            card.tab = tab;
            card.shop_menu_open = false;
            cx.notify();
        }
    }

    pub(crate) fn set_shop_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile
            && card.shop_menu_open != open
        {
            card.shop_menu_open = open;
            cx.notify();
        }
    }

    pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) -> bool {
        self.drop_styles_editor();
        let open = self.profile.take().is_some();
        if open {
            cx.notify();
        }
        open
    }
}

impl ProfileCard {
    pub(super) fn new(
        user_id: u64,
        guild_id: Option<u64>,
        position: Point<Pixels>,
        anchor: Anchor,
        origin: ProfileOrigin,
    ) -> Self {
        Self {
            user_id,
            panel: false,
            guild_id,
            source_guild_id: guild_id,
            more_menu_open: false,
            hovered_action: None,
            status_menu: popout::status::State::default(),
            scope_snapshots: HashMap::new(),
            origin,
            position,
            anchor,
            modal: false,
            profile: None,
            games: HashMap::new(),
            games_pending: HashSet::new(),
            applications: HashMap::new(),
            applications_pending: HashSet::new(),
            identities: None,
            identities_pending: false,
            outbox: None,
            wishlist: None,
            note: None,
            badge_details: None,
            hovered_badge: None,
            hovered_cover: None,
            hovered_art: [false; 4],
            stack_hover: Default::default(),
            widget_configs: HashMap::new(),
            widget_configs_pending: HashSet::new(),
            board_requested: false,
            modal_request_pending: false,
            details_requested: false,
            wishlist_requested: false,
            note_requested: false,
            shop_menu_open: false,
            game_timer_started: false,
            effect_timer_started: false,
            avatar_theme: None,
            avatar_theme_source: None,
            effect: None,
            frame: None,
            cosmetics_pending: HashSet::new(),
            opened: Instant::now(),
            effect_started: RefCell::new(None),
            tab: Tab::Activity,
            status_prompt: rand::random::<usize>() % STATUS_PROMPTS.len(),
            activity_prompt: rand::random::<usize>() % ACTIVITY_PROMPTS.len(),
            expanded_widgets: HashSet::new(),
            widget_toggles: HashMap::new(),
            commands: Vec::new(),
            commands_requested: false,
            commands_pending: false,
            hovered_command: None,
            _tasks: Vec::new(),
        }
    }

    fn load_commands(&mut self, session: Session, cx: &mut Context<DiscordApp>) {
        let Some(guild_id) = self.source_guild_id.filter(|_| !self.commands_requested) else {
            return;
        };
        let Some(application) = self
            .profile
            .as_ref()
            .and_then(|profile| profile.application.as_ref())
            .filter(|application| !application.popular_application_command_ids.is_empty())
        else {
            return;
        };
        self.commands_requested = true;
        self.commands_pending = true;
        let user_id = self.user_id;
        let application_id = application.id;
        let popular = application.popular_application_command_ids.clone();
        let request = Tokio::spawn(cx, async move {
            session
                .popular_commands(guild_id, application_id, popular)
                .await
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let commands = match request.await {
                Ok(Ok(commands)) => commands,
                Ok(Err(error)) => {
                    log::warn!("Could not load commands for app {application_id}: {error}");
                    Vec::new()
                }
                Err(error) => {
                    log::warn!("Commands task failed for app {application_id}: {error}");
                    Vec::new()
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app.profile.as_mut().filter(|card| card.user_id == user_id) else {
                    return;
                };
                card.commands_pending = false;
                card.commands = commands;
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn switch_scope(&mut self) -> bool {
        self.more_menu_open = false;
        let Some(source_guild_id) = self.source_guild_id else {
            return false;
        };
        let guild_id = self.guild_id.is_none().then_some(source_guild_id);
        self.switch_to_scope(guild_id)
    }

    pub(super) fn switch_to_scope(&mut self, guild_id: Option<u64>) -> bool {
        self.more_menu_open = false;
        if self.guild_id == guild_id {
            return false;
        }
        if let Some(profile) = &self.profile {
            self.scope_snapshots.insert(self.guild_id, profile.clone());
        }
        if guild_id.is_some() {
            self.source_guild_id = guild_id;
        }
        self.replace_scope(guild_id);
        true
    }

    fn replace_scope(&mut self, guild_id: Option<u64>) {
        let mut next = Self::new(
            self.user_id,
            guild_id,
            self.position,
            self.anchor,
            self.origin.clone(),
        );
        next.source_guild_id = self.source_guild_id;
        next.modal = self.modal;
        next.tab = self.tab;
        next.status_prompt = self.status_prompt;
        next.activity_prompt = self.activity_prompt;
        next.scope_snapshots = std::mem::take(&mut self.scope_snapshots);
        next.profile = next.scope_snapshots.get(&guild_id).cloned().or_else(|| {
            self.profile
                .as_ref()
                .filter(|_| guild_id.is_none())
                .map(|profile| scoped_profile(profile.clone(), None))
        });
        *self = next;
    }
}

pub(super) fn cosmetic_skus(profile: &model::Profile) -> (Option<u64>, Option<u64>) {
    let effect_of = |p: &model::UserProfile| {
        p.profile_effect
            .as_ref()
            .map(|effect| effect.sku_id)
            .or_else(|| {
                p.collectibles
                    .iter()
                    .find(|item| item.kind == 1)
                    .map(|item| item.sku_id)
            })
    };
    let effect = guild_profile(profile)
        .and_then(effect_of)
        .or_else(|| profile.user_profile.as_ref().and_then(effect_of));
    let frame_of = |p: &model::UserProfile| {
        p.collectibles
            .iter()
            .find(|c| c.kind == 3)
            .map(|c| c.sku_id)
    };
    let frame = guild_profile(profile)
        .and_then(frame_of)
        .or_else(|| profile.user_profile.as_ref().and_then(frame_of));
    (effect, frame)
}

fn scoped_profile(mut profile: Arc<model::Profile>, guild_id: Option<u64>) -> Arc<model::Profile> {
    if guild_id.is_none()
        && (profile.guild_member.is_some() || profile.guild_member_profile.is_some())
    {
        let profile = Arc::make_mut(&mut profile);
        profile.guild_member = None;
        profile.guild_member_profile = None;
    }
    profile
}

pub(super) fn guild_profile(profile: &model::Profile) -> Option<&model::UserProfile> {
    profile.guild_member_profile.as_ref()
}

pub(super) fn widget_game_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        let games = match &widget.data {
            model::WidgetData::FavoriteGames { games }
            | model::WidgetData::CurrentGames { games }
            | model::WidgetData::PlayedGames { games }
            | model::WidgetData::WantToPlayGames { games } => games,
            _ => continue,
        };
        for game in games {
            if !ids.contains(&game.game_id) {
                ids.push(game.game_id);
            }
        }
    }
    ids
}

pub(super) fn widget_application_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        if let model::WidgetData::Application { application_id } = &widget.data
            && !ids.contains(application_id)
        {
            ids.push(*application_id);
        }
    }
    ids
}

pub(super) fn outbox_application_ids(outbox: &model::Outbox) -> Vec<u64> {
    let mut ids = Vec::new();
    for entry in &outbox.entries {
        if let model::OutboxKind::PlayedGame {
            application_id: Some(id),
            ..
        }
        | model::OutboxKind::LaunchedActivity {
            application_id: Some(id),
            ..
        } = &entry.kind
            && !ids.contains(id)
        {
            ids.push(*id);
        }
    }
    ids
}
