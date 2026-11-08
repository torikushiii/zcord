use super::*;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum CacheRequest {
    Profile(u64, Option<u64>),
    Product(u64),
    CollectibleCatalog,
    Outbox(u64),
    Wishlist(u64),
    Invite(String),
    Identities(u64),
    Games,
    Applications,
    WidgetConfigs,
    WidgetConfig(u64),
    Badges(u64),
}

pub struct ProfileDetails {
    pub games: HashMap<u64, Arc<model::Game>>,
    pub applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub identities: Option<Arc<model::ApplicationIdentities>>,
    pub outbox: Option<Arc<model::Outbox>>,
    pub badges: Option<Arc<model::UserBadges>>,
    pub widget_configs: HashMap<u64, Arc<model::WidgetConfig>>,
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) profiles: HashMap<(u64, Option<u64>), CachedProfile>,
    pub(super) profile_revision: u64,
    pub(super) products: HashMap<u64, Arc<model::CollectibleProduct>>,
    pub(super) collectible_catalog: Option<Arc<Vec<Arc<model::CollectibleProduct>>>>,
    pub(super) outboxes: HashMap<u64, Cached<model::Outbox>>,
    pub(super) wishlists: HashMap<u64, Cached<model::Wishlist>>,
    pub(super) invites: HashMap<String, CachedInvite>,
    pub(super) identities: HashMap<u64, Cached<model::ApplicationIdentities>>,
    pub(super) badges: HashMap<u64, Cached<model::UserBadges>>,
    pub(super) games: HashMap<u64, Arc<model::Game>>,
    pub(super) missing_games: HashMap<u64, Instant>,
    pub(super) applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub(super) missing_applications: HashMap<u64, Instant>,
    pub(super) featured_widget_configs: Option<HashMap<u64, Arc<model::WidgetConfig>>>,
    pub(super) widget_configs_by_app: HashMap<u64, Option<Arc<model::WidgetConfig>>>,
    pub(super) failures: HashMap<CacheRequest, (Instant, Error)>,
}

pub(super) struct CachedProfile {
    kind: ProfileKind,
    fetched: Instant,
    profile: Arc<model::Profile>,
}

pub(super) struct Cached<T> {
    fetched: Instant,
    value: Arc<T>,
}

pub(super) struct CachedInvite {
    fetched: Instant,
    invite: Option<Arc<model::Invite>>,
}

impl Cache {
    pub(super) fn profile(
        &self,
        key: (u64, Option<u64>),
        kind: ProfileKind,
        now: Instant,
    ) -> Option<Arc<model::Profile>> {
        let cached = self.profiles.get(&key)?;
        // A full profile response includes everything the popout shows, so it can stand in.
        let covers = cached.kind == kind || cached.kind == ProfileKind::Modal;
        (covers && now.saturating_duration_since(cached.fetched) < PROFILE_TTL)
            .then(|| cached.profile.clone())
    }

    pub(super) fn insert_profile(
        &mut self,
        key: (u64, Option<u64>),
        kind: ProfileKind,
        profile: Arc<model::Profile>,
        now: Instant,
    ) {
        // A popout started before the modal must not replace its richer cached response.
        if kind == ProfileKind::Popout && self.profile(key, ProfileKind::Modal, now).is_some() {
            return;
        }
        if self.profiles.len() >= CACHE_LIMIT && !self.profiles.contains_key(&key) {
            let oldest = self
                .profiles
                .iter()
                .min_by_key(|(_, cached)| cached.fetched)
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                self.profiles.remove(&oldest);
            }
        }
        self.profiles.insert(
            key,
            CachedProfile {
                kind,
                fetched: now,
                profile,
            },
        );
    }

    pub(super) fn profile_details(
        &self,
        user_id: u64,
        game_ids: &[u64],
        application_ids: &[u64],
        now: Instant,
    ) -> ProfileDetails {
        let outbox = self.outbox(user_id, now);
        let mut applications: HashSet<_> = application_ids.iter().copied().collect();
        for entry in outbox.iter().flat_map(|outbox| &outbox.entries) {
            if let model::OutboxKind::PlayedGame {
                application_id: Some(id),
                ..
            }
            | model::OutboxKind::LaunchedActivity {
                application_id: Some(id),
                ..
            } = &entry.kind
            {
                applications.insert(*id);
            }
        }
        ProfileDetails {
            games: game_ids
                .iter()
                .filter_map(|id| Some((*id, self.games.get(id)?.clone())))
                .collect(),
            applications: applications
                .into_iter()
                .filter_map(|id| Some((id, self.applications.get(&id)?.clone())))
                .collect(),
            identities: self.identities(user_id, now),
            outbox,
            badges: self.badges(user_id, now),
            widget_configs: application_ids
                .iter()
                .filter_map(|id| {
                    let config = self
                        .featured_widget_configs
                        .as_ref()
                        .and_then(|configs| configs.get(id))
                        .or_else(|| self.widget_configs_by_app.get(id)?.as_ref())?;
                    Some((*id, config.clone()))
                })
                .collect(),
        }
    }

    pub(super) fn insert_product(&mut self, sku_id: u64, product: Arc<model::CollectibleProduct>) {
        if self.products.len() >= CACHE_LIMIT {
            let any = self.products.keys().next().copied();
            if let Some(any) = any {
                self.products.remove(&any);
            }
        }
        self.products.insert(sku_id, product);
    }

    pub(super) fn outbox(&self, user_id: u64, now: Instant) -> Option<Arc<model::Outbox>> {
        cached_value(&self.outboxes, user_id, now)
    }

    pub(super) fn insert_outbox(&mut self, user_id: u64, outbox: Arc<model::Outbox>, now: Instant) {
        insert_cached(&mut self.outboxes, user_id, outbox, now);
    }

    pub(super) fn wishlist(&self, wishlist_id: u64, now: Instant) -> Option<Arc<model::Wishlist>> {
        cached_value(&self.wishlists, wishlist_id, now)
    }

    pub(super) fn insert_wishlist(
        &mut self,
        wishlist_id: u64,
        wishlist: Arc<model::Wishlist>,
        now: Instant,
    ) {
        insert_cached(&mut self.wishlists, wishlist_id, wishlist, now);
    }

    pub(super) fn invite(&self, code: &str, now: Instant) -> Option<Option<Arc<model::Invite>>> {
        let cached = self.invites.get(code)?;
        (now.saturating_duration_since(cached.fetched) < PROFILE_TTL).then(|| cached.invite.clone())
    }

    pub(super) fn insert_invite(
        &mut self,
        code: String,
        invite: Option<Arc<model::Invite>>,
        now: Instant,
    ) {
        if self.invites.len() >= CACHE_LIMIT && !self.invites.contains_key(&code) {
            let oldest = self
                .invites
                .iter()
                .min_by_key(|(_, cached)| cached.fetched)
                .map(|(code, _)| code.clone());
            if let Some(oldest) = oldest {
                self.invites.remove(&oldest);
            }
        }
        self.invites.insert(
            code,
            CachedInvite {
                fetched: now,
                invite,
            },
        );
    }

    pub(super) fn identities(
        &self,
        user_id: u64,
        now: Instant,
    ) -> Option<Arc<model::ApplicationIdentities>> {
        cached_value(&self.identities, user_id, now)
    }

    pub(super) fn insert_identities(
        &mut self,
        user_id: u64,
        identities: Arc<model::ApplicationIdentities>,
        now: Instant,
    ) {
        insert_cached(&mut self.identities, user_id, identities, now);
    }

    pub(super) fn badges(&self, user_id: u64, now: Instant) -> Option<Arc<model::UserBadges>> {
        cached_value(&self.badges, user_id, now)
    }

    pub(super) fn insert_badges(
        &mut self,
        user_id: u64,
        badges: Arc<model::UserBadges>,
        now: Instant,
    ) {
        insert_cached(&mut self.badges, user_id, badges, now);
    }

    pub(super) fn insert_widget_config(
        &mut self,
        application_id: u64,
        config: Option<Arc<model::WidgetConfig>>,
    ) {
        if self.widget_configs_by_app.len() >= CACHE_LIMIT
            && !self.widget_configs_by_app.contains_key(&application_id)
            && let Some(any) = self.widget_configs_by_app.keys().next().copied()
        {
            self.widget_configs_by_app.remove(&any);
        }
        self.widget_configs_by_app.insert(application_id, config);
    }

    pub(super) fn failure(&self, key: CacheRequest, now: Instant) -> Option<Error> {
        self.failures
            .get(&key)
            .filter(|(at, _)| now.saturating_duration_since(*at) < FAILURE_TTL)
            .map(|(_, error)| error.clone())
    }

    pub(super) fn remember_failure(&mut self, key: CacheRequest, error: &Error, now: Instant) {
        let remember = match error {
            Error::Rejected { .. } | Error::Decode => true,
            Error::Http(status) => *status != 401,
            _ => false,
        };
        if !remember {
            return;
        }
        if self.failures.len() >= CACHE_LIMIT && !self.failures.contains_key(&key) {
            self.failures
                .retain(|_, (at, _)| now.saturating_duration_since(*at) < FAILURE_TTL);
            if self.failures.len() >= CACHE_LIMIT {
                let oldest = self
                    .failures
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    self.failures.remove(&oldest);
                }
            }
        }
        self.failures.insert(key, (now, error.clone()));
    }
}

pub(super) fn missing_ids<T>(
    known: &HashMap<u64, Arc<T>>,
    absent: &HashMap<u64, Instant>,
    ids: &[u64],
    now: Instant,
) -> Vec<u64> {
    let mut seen = HashSet::new();
    ids.iter()
        .copied()
        .filter(|id| {
            seen.insert(*id)
                && !known.contains_key(id)
                && !absent
                    .get(id)
                    .is_some_and(|fetched| now.saturating_duration_since(*fetched) < PROFILE_TTL)
        })
        .collect()
}

// Only successful lookups populate this cache; network failures must remain retryable.
pub(super) fn insert_missing(map: &mut HashMap<u64, Instant>, id: u64, now: Instant) {
    if map.len() >= CACHE_LIMIT
        && !map.contains_key(&id)
        && let Some(oldest) = map
            .iter()
            .min_by_key(|(_, fetched)| **fetched)
            .map(|(id, _)| *id)
    {
        map.remove(&oldest);
    }
    map.insert(id, now);
}

pub(super) fn cached_value<T>(
    map: &HashMap<u64, Cached<T>>,
    key: u64,
    now: Instant,
) -> Option<Arc<T>> {
    let cached = map.get(&key)?;
    (now.saturating_duration_since(cached.fetched) < PROFILE_TTL).then(|| cached.value.clone())
}

pub(super) fn insert_cached<T>(
    map: &mut HashMap<u64, Cached<T>>,
    key: u64,
    value: Arc<T>,
    now: Instant,
) {
    if map.len() >= CACHE_LIMIT && !map.contains_key(&key) {
        let oldest = map
            .iter()
            .min_by_key(|(_, cached)| cached.fetched)
            .map(|(key, _)| *key);
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        }
    }
    map.insert(
        key,
        Cached {
            fetched: now,
            value,
        },
    );
}

pub(super) fn insert_bounded<T>(map: &mut HashMap<u64, Arc<T>>, key: u64, value: Arc<T>) {
    if map.len() >= CACHE_LIMIT
        && !map.contains_key(&key)
        && let Some(any) = map.keys().next().copied()
    {
        map.remove(&any);
    }
    map.insert(key, value);
}

#[cfg(test)]
mod tests;
