use super::*;

fn offline_session() -> Session {
    let (control, _) = watch::channel(Control::Running);
    let (commands, _) = tokio::sync::mpsc::unbounded_channel();
    Session {
        inner: Arc::new(Inner {
            rest: Err(Error::Network),
            gif_client: Err(Error::Network),
            cache: Mutex::default(),
            requests: Mutex::default(),
            sends: Mutex::default(),
            control,
            commands,
            generation: Arc::new(AtomicU64::new(0)),
            presence: Arc::default(),
            voice: voice::VoiceManager::default(),
        }),
    }
}

fn profile() -> Arc<model::Profile> {
    Arc::new(serde_json::from_str(include_str!("../fixtures/profile_modal.json")).unwrap())
}

#[test]
fn profile_cache_lets_full_profiles_serve_popouts_until_expiry() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let key = (1, Some(2));
    cache.insert_profile(key, ProfileKind::Popout, profile(), start);
    assert!(cache.profile(key, ProfileKind::Popout, start).is_some());
    assert!(cache.profile(key, ProfileKind::Modal, start).is_none());
    assert!(
        cache
            .profile((1, None), ProfileKind::Popout, start)
            .is_none()
    );
    cache.insert_profile(key, ProfileKind::Modal, profile(), start);
    assert!(cache.profile(key, ProfileKind::Popout, start).is_some());
    assert!(cache.profile(key, ProfileKind::Modal, start).is_some());
    let expired = start + PROFILE_TTL;
    assert!(cache.profile(key, ProfileKind::Popout, expired).is_none());
}

#[test]
fn late_popout_does_not_downgrade_or_extend_the_modal_cache() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let key = (1, Some(2));
    let modal = profile();
    cache.insert_profile(key, ProfileKind::Modal, modal.clone(), start);
    cache.insert_profile(
        key,
        ProfileKind::Popout,
        profile(),
        start + Duration::from_secs(1),
    );
    assert!(Arc::ptr_eq(
        &cache.profile(key, ProfileKind::Modal, start).unwrap(),
        &modal
    ));
    let expired = start + PROFILE_TTL;
    assert!(cache.profile(key, ProfileKind::Modal, expired).is_none());
    cache.insert_profile(key, ProfileKind::Popout, profile(), expired);
    assert!(cache.profile(key, ProfileKind::Popout, expired).is_some());
    assert!(cache.profile(key, ProfileKind::Modal, expired).is_none());
}

#[test]
fn profile_details_expose_cached_items_without_waiting_for_missing_ones() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let game: Arc<model::Game> =
        Arc::new(serde_json::from_value(serde_json::json!({"id": "7", "name": "Game"})).unwrap());
    cache.games.insert(7, game.clone());
    for id in [7, 8, 9] {
        let application: Arc<model::PublicApplication> = Arc::new(
            serde_json::from_value(serde_json::json!({"id": id.to_string(), "name": "App"}))
                .unwrap(),
        );
        cache.applications.insert(id, application);
    }
    let outbox: Arc<model::Outbox> = Arc::new(
        serde_json::from_value(serde_json::json!({"entries": [{
            "id": "10",
            "extra": {"type": "played_game_extra", "application_id": "8"}
        }]}))
        .unwrap(),
    );
    cache.insert_outbox(1, outbox.clone(), start);
    let identities = Arc::new(model::ApplicationIdentities::default());
    cache.insert_identities(1, identities.clone(), start);
    let config = Arc::new(
        model::parse_application_widget_configs(serde_json::json!({"surfaces": {}}), 7).unwrap(),
    );
    cache.insert_widget_config(7, Some(config.clone()));

    let details = cache.profile_details(1, &[7, 404], &[7, 404], start);
    assert_eq!(details.games.len(), 1);
    assert!(Arc::ptr_eq(&details.games[&7], &game));
    assert_eq!(details.applications.len(), 2);
    assert!(details.applications.contains_key(&8));
    assert!(!details.applications.contains_key(&9));
    assert!(Arc::ptr_eq(details.outbox.as_ref().unwrap(), &outbox));
    assert!(Arc::ptr_eq(
        details.identities.as_ref().unwrap(),
        &identities
    ));
    assert!(Arc::ptr_eq(&details.widget_configs[&7], &config));

    let other = cache.profile_details(2, &[], &[7], start);
    assert!(other.outbox.is_none());
    assert!(other.identities.is_none());
    assert!(!other.applications.contains_key(&8));
    let expired = cache.profile_details(1, &[7], &[7], start + PROFILE_TTL);
    assert!(expired.outbox.is_none());
    assert!(expired.identities.is_none());
    assert!(!expired.applications.contains_key(&8));
    assert_eq!(expired.games.len(), 1);
    assert_eq!(expired.widget_configs.len(), 1);

    cache.featured_widget_configs = Some(HashMap::from([(7, config.clone())]));
    cache.widget_configs_by_app.clear();
    let featured = cache.profile_details(1, &[], &[7], start);
    assert!(Arc::ptr_eq(&featured.widget_configs[&7], &config));
}

#[test]
fn missing_metadata_is_deduplicated_and_retried_after_expiry() {
    let start = Instant::now();
    let known = HashMap::from([(1, Arc::new(()))]);
    let mut absent = HashMap::new();
    insert_missing(&mut absent, 2, start);
    assert_eq!(missing_ids(&known, &absent, &[1, 2, 3, 3], start), [3]);
    assert_eq!(
        missing_ids(&known, &absent, &[1, 2, 3, 3], start + PROFILE_TTL),
        [2, 3]
    );
}

#[test]
fn missing_metadata_cache_is_bounded_and_evicts_the_oldest() {
    let start = Instant::now();
    let mut absent = HashMap::new();
    for id in 0..CACHE_LIMIT as u64 {
        insert_missing(&mut absent, id, start + Duration::from_secs(id));
    }
    insert_missing(&mut absent, 999, start + Duration::from_secs(999));
    assert_eq!(absent.len(), CACHE_LIMIT);
    assert!(!absent.contains_key(&0));
    assert!(absent.contains_key(&999));
}

#[test]
fn profile_cache_evicts_the_oldest_entry_when_full() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let profile = profile();
    for user in 0..CACHE_LIMIT as u64 {
        let fetched = start + Duration::from_secs(user + 1);
        cache.insert_profile((user, None), ProfileKind::Popout, profile.clone(), fetched);
    }
    cache.insert_profile((999, None), ProfileKind::Popout, profile, start);
    assert_eq!(cache.profiles.len(), CACHE_LIMIT);
    assert!(!cache.profiles.contains_key(&(0, None)));
    assert!(cache.profiles.contains_key(&(999, None)));
}

#[test]
fn outbox_and_identity_caches_expire_with_profiles() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let outbox = Arc::new(serde_json::from_value(serde_json::json!({"entries": []})).unwrap());
    let identities =
        Arc::new(serde_json::from_value(serde_json::json!({"identities": []})).unwrap());
    cache.insert_outbox(7, outbox, start);
    cache.insert_identities(7, identities, start);
    assert!(cache.outbox(7, start).is_some());
    assert!(cache.identities(7, start).is_some());
    assert!(cache.outbox(8, start).is_none());
    assert!(cache.identities(8, start).is_none());
    let expired = start + PROFILE_TTL;
    assert!(cache.outbox(7, expired).is_none());
    assert!(cache.identities(7, expired).is_none());
}

#[test]
fn failed_lookups_are_remembered_only_when_discord_rejects_them() {
    let mut cache = Cache::default();
    let start = Instant::now();
    let rejected = Error::Rejected {
        status: 403,
        code: 50001,
        message: "x".into(),
    };
    cache.remember_failure(CacheRequest::Outbox(1), &rejected, start);
    assert!(cache.failure(CacheRequest::Outbox(1), start).is_some());
    assert!(
        cache
            .failure(CacheRequest::Outbox(1), start + FAILURE_TTL)
            .is_none()
    );
    cache.remember_failure(CacheRequest::Identities(1), &Error::Network, start);
    cache.remember_failure(CacheRequest::Identities(1), &Error::Http(401), start);
    assert!(cache.failure(CacheRequest::Identities(1), start).is_none());
}

#[test]
fn game_and_application_caches_are_per_id_and_bounded() {
    let mut cache = Cache::default();
    for id in 0..CACHE_LIMIT as u64 {
        let game: Arc<model::Game> = Arc::new(
            serde_json::from_value(serde_json::json!({"id": id.to_string(), "name": "Game"}))
                .unwrap(),
        );
        insert_bounded(&mut cache.games, id, game);
        let application: Arc<model::PublicApplication> = Arc::new(
            serde_json::from_value(
                serde_json::json!({"id": id.to_string(), "name": "Application"}),
            )
            .unwrap(),
        );
        insert_bounded(&mut cache.applications, id, application);
    }
    assert_eq!(cache.games.len(), CACHE_LIMIT);
    assert_eq!(cache.applications.len(), CACHE_LIMIT);
    let game: Arc<model::Game> = Arc::new(
        serde_json::from_value(serde_json::json!({"id": "999", "name": "Newest"})).unwrap(),
    );
    insert_bounded(&mut cache.games, 999, game);
    let application: Arc<model::PublicApplication> = Arc::new(
        serde_json::from_value(serde_json::json!({"id": "999", "name": "Newest"})).unwrap(),
    );
    insert_bounded(&mut cache.applications, 999, application);
    assert_eq!(cache.games.len(), CACHE_LIMIT);
    assert_eq!(cache.applications.len(), CACHE_LIMIT);
    assert!(cache.games.contains_key(&999));
    assert!(cache.applications.contains_key(&999));
}

#[test]
fn request_locks_share_resources_and_discard_unused_keys() {
    let session = offline_session();
    let profile = session.request_lock(CacheRequest::Profile(1, Some(2)));
    let same = session.request_lock(CacheRequest::Profile(1, Some(2)));
    assert!(Arc::ptr_eq(&profile, &same));
    assert!(!Arc::ptr_eq(
        &profile,
        &session.request_lock(CacheRequest::Profile(1, None))
    ));
    assert!(!Arc::ptr_eq(
        &profile,
        &session.request_lock(CacheRequest::Outbox(1))
    ));
    for id in 0..1000 {
        drop(session.request_lock(CacheRequest::Identities(id)));
    }
    let outbox = session.request_lock(CacheRequest::Outbox(1));
    assert_eq!(session.inner.requests.lock().unwrap().len(), 2);
    drop((profile, same, outbox));
    let _games = session.request_lock(CacheRequest::Games);
    assert_eq!(session.inner.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn overlapping_profile_loads_recheck_the_cache_after_waiting() {
    let session = offline_session();
    let fetching = session
        .request_lock(CacheRequest::Profile(1, Some(2)))
        .lock_owned()
        .await;
    let full = session.profile(1, Some(2), None, ProfileKind::Modal);
    let popout = session.profile(1, Some(2), None, ProfileKind::Popout);
    tokio::pin!(full, popout);
    assert!(futures::poll!(&mut full).is_pending());
    assert!(futures::poll!(&mut popout).is_pending());
    let profile = profile();
    session.cache().insert_profile(
        (1, Some(2)),
        ProfileKind::Modal,
        profile.clone(),
        Instant::now(),
    );
    drop(fetching);
    let (full, popout) = tokio::join!(full, popout);
    assert!(Arc::ptr_eq(&full.unwrap(), &profile));
    assert!(Arc::ptr_eq(&popout.unwrap(), &profile));
}

#[tokio::test]
async fn profile_styles_catalog_reuses_session_cache_for_overlapping_loads() {
    let session = offline_session();
    let fetching = session
        .request_lock(CacheRequest::CollectibleCatalog)
        .lock_owned()
        .await;
    let first = session.collectible_catalog();
    let second = session.collectible_catalog();
    tokio::pin!(first, second);
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut second).is_pending());
    let product = serde_json::from_value(
        serde_json::json!({"sku_id": "1", "type": 1, "items": [{"sku_id": "1", "type": 1}]}),
    )
    .unwrap();
    let catalog = Arc::new(vec![Arc::new(product)]);
    session.cache().collectible_catalog = Some(catalog.clone());
    drop(fetching);
    let (first, second) = tokio::join!(first, second);
    assert!(Arc::ptr_eq(&first.unwrap(), &catalog));
    assert!(Arc::ptr_eq(&second.unwrap(), &catalog));
    assert!(Arc::ptr_eq(
        &session.clone().collectible_catalog().await.unwrap(),
        &catalog
    ));
}

#[tokio::test]
async fn activity_resources_recheck_cache_independently() {
    let session = offline_session();
    let games = session.request_lock(CacheRequest::Games).lock_owned().await;
    let outboxes = session
        .request_lock(CacheRequest::Outbox(1))
        .lock_owned()
        .await;
    let identities = session
        .request_lock(CacheRequest::Identities(1))
        .lock_owned()
        .await;
    let applications = session
        .request_lock(CacheRequest::Applications)
        .lock_owned()
        .await;
    let game_load = session.games(&[7]);
    let outbox_load = session.outbox(1);
    let identity_load = session.application_identities(1);
    let application_load = session.public_applications(&[7]);
    tokio::pin!(game_load, outbox_load, identity_load, application_load);
    assert!(futures::poll!(&mut game_load).is_pending());
    assert!(futures::poll!(&mut outbox_load).is_pending());
    assert!(futures::poll!(&mut identity_load).is_pending());
    assert!(futures::poll!(&mut application_load).is_pending());
    {
        let mut cache = session.cache();
        cache.games.insert(
            7,
            Arc::new(
                serde_json::from_value(serde_json::json!({"id": "7", "name": "Game"})).unwrap(),
            ),
        );
        cache.applications.insert(
            7,
            Arc::new(
                serde_json::from_value(serde_json::json!({"id": "7", "name": "App"})).unwrap(),
            ),
        );
        cache.insert_outbox(
            1,
            Arc::new(serde_json::from_value(serde_json::json!({"entries": []})).unwrap()),
            Instant::now(),
        );
        cache.insert_identities(
            1,
            Arc::new(model::ApplicationIdentities::default()),
            Instant::now(),
        );
    }
    drop(outboxes);
    assert!(outbox_load.await.is_ok());
    drop((games, identities, applications));
    assert_eq!(game_load.await.unwrap()[0].id, 7);
    assert!(identity_load.await.is_ok());
    assert_eq!(application_load.await.unwrap()[0].id, 7);
}
