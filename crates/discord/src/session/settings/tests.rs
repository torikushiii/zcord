use super::*;

fn offline_session(
    generation: u64,
) -> (
    Session,
    tokio::sync::mpsc::UnboundedReceiver<(u64, commands::Command)>,
) {
    let (control, _) = watch::channel(Control::Running);
    let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let session = Session {
        inner: Arc::new(Inner {
            rest: Err(Error::Network),
            gif_client: Err(Error::Network),
            cache: Mutex::default(),
            requests: Mutex::default(),
            sends: Mutex::default(),
            control,
            commands,
            generation: Arc::new(AtomicU64::new(generation)),
            presence: Arc::default(),
            voice: voice::VoiceManager::default(),
        }),
    };
    (session, receiver)
}

#[test]
fn manual_expiry_deadline_survives_reconnect_and_refreshes_online_without_rest() {
    use crate::settings_fixtures::status_fixture;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::time::{Duration, UNIX_EPOCH};

    let (session, mut receiver) = offline_session(0);
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let deadline = now + 86_400_000;
    let base = STANDARD
        .decode(status_fixture(
            Some("idle"),
            Some(("keep custom", 42, "party", 0)),
        ))
        .unwrap();
    let patch = settings_proto::status_patch_at(&base, "idle", deadline, Some(now)).unwrap();
    session
        .inner
        .presence
        .lock()
        .unwrap()
        .install(settings_proto::merge_partial(&base, &patch).unwrap())
        .unwrap();
    assert_eq!(session.next_status_expiry(), None, "not READY yet");
    session.inner.generation.store(1, Ordering::Release);
    let expected = Some(UNIX_EPOCH + Duration::from_millis(deadline));
    assert_eq!(session.next_status_expiry(), expected);
    assert_eq!(session.self_status(), Some(SelfStatus::Idle));
    session.inner.generation.store(2, Ordering::Release);
    assert_eq!(session.next_status_expiry(), None);
    session.inner.generation.store(3, Ordering::Release);
    assert_eq!(
        session.next_status_expiry(),
        expected,
        "reconnect does not reset the clock"
    );

    let expired = settings_proto::status_patch_at(&base, "dnd", 1, Some(0)).unwrap();
    session
        .inner
        .presence
        .lock()
        .unwrap()
        .install(settings_proto::merge_partial(&base, &expired).unwrap())
        .unwrap();
    assert_eq!(session.next_status_expiry(), None);
    assert_eq!(session.self_status(), Some(SelfStatus::Online));
    session.refresh_presence().unwrap();
    let (generation, command) = receiver.try_recv().unwrap();
    assert_eq!(generation, 3);
    assert_eq!(command.payload["d"]["status"], "online");
    assert_eq!(
        command.payload["d"]["activities"][0]["state"],
        "keep custom"
    );
    let presence = session.inner.presence.lock().unwrap();
    assert_eq!(presence.configured_status(), Some(SelfStatus::DoNotDisturb));
    assert_eq!(
        presence.timing().unwrap().expires_at_ms,
        1,
        "expiry is projected, not persisted again"
    );
    assert!(session.inner.rest.is_err());
}

#[test]
fn expiry_refresh_withdraws_custom_activity_using_current_status_without_rest() {
    use crate::settings_fixtures::status_fixture;
    use base64::{Engine, engine::general_purpose::STANDARD};

    let (session, mut receiver) = offline_session(1);
    let install = |status, custom| {
        session
            .inner
            .presence
            .lock()
            .unwrap()
            .install(
                STANDARD
                    .decode(status_fixture(Some(status), Some(custom)))
                    .unwrap(),
            )
            .unwrap();
    };
    install("online", ("expiring", 42, "party", u64::MAX));
    session.refresh_presence().unwrap();
    let (_, initial) = receiver.try_recv().unwrap();
    assert_eq!(
        initial.payload["d"]["activities"].as_array().unwrap().len(),
        1
    );
    initial.complete(Ok(()));

    install("invisible", ("expiring", 42, "party", 1));
    session.refresh_presence().unwrap();
    let (generation, expired) = receiver.try_recv().unwrap();
    assert_eq!(generation, 1);
    assert_eq!(expired.payload["op"], 3);
    assert_eq!(expired.payload["d"]["status"], "invisible");
    assert_eq!(expired.payload["d"]["activities"], serde_json::json!([]));
    assert_eq!(session.self_status(), Some(SelfStatus::Invisible));

    install("dnd", ("replacement", 0, "", u64::MAX));
    session.refresh_presence().unwrap();
    let (_, replacement) = receiver.try_recv().unwrap();
    assert_eq!(replacement.payload["d"]["status"], "dnd");
    assert_eq!(
        replacement.payload["d"]["activities"][0]["state"],
        "replacement"
    );
    session.inner.generation.store(2, Ordering::Release);
    assert!(matches!(session.refresh_presence(), Err(Error::Closed)));
    assert!(receiver.try_recv().is_err());
    session.inner.generation.store(3, Ordering::Release);
    session.refresh_presence().unwrap();
    assert_eq!(receiver.try_recv().unwrap().0, 3);
    assert!(session.inner.rest.is_err());
}

#[tokio::test]
async fn account_settings_use_cached_values_and_guard_saves_by_generation() {
    use crate::settings_fixtures::{field, uint};
    use settings_proto::{AccountTheme, ClientTheme, FavoriteChannel, TextImages};

    let (session, mut receiver) = offline_session(0);
    assert_eq!(session.text_images(), TextImages::default());
    assert_eq!(session.client_theme(), None);
    assert_eq!(session.favorites(), settings_proto::Favorites::default());
    let proto = [
        field(1, &uint(3, 42)),
        field(6, &field(11, &[])),
        settings_proto::appearance_patch(&[], AccountTheme::Dark, Some(&ClientTheme::Preset(42)))
            .unwrap(),
        field(15, &field(3, &uint(1, 1))),
    ]
    .concat();
    session
        .inner
        .presence
        .lock()
        .unwrap()
        .install(proto.clone())
        .unwrap();
    assert!(!session.text_images().gif_auto_play);
    assert_eq!(session.client_theme(), Some(ClientTheme::Preset(42)));
    assert!(session.favorites().guild_visible);
    for generation in [0, 1, 2] {
        session
            .inner
            .generation
            .store(generation, Ordering::Release);
        let results = [
            session
                .set_text_images(TextImages::default(), TextImages::default())
                .await,
            session.set_appearance(AccountTheme::Light, None).await,
            session.set_favorites_visible(false).await,
            session
                .set_favorite_channel(
                    123,
                    Some(FavoriteChannel {
                        id: 123,
                        ..Default::default()
                    }),
                )
                .await,
        ];
        for result in results {
            if generation == 1 {
                assert!(matches!(result, Err(Error::Network)));
            } else {
                assert!(matches!(result, Err(Error::Closed)));
            }
        }
    }
    session.inner.generation.store(3, Ordering::Release);
    assert!(matches!(
        session.save_preloaded_patch(1, proto.clone(), None).await,
        Err(Error::Closed)
    ));
    let saved = session.save_preloaded_patch(3, proto, None).await.unwrap();
    assert_eq!(settings_proto::data_version(&saved).unwrap(), Some(42));
    assert!(receiver.try_recv().is_err());
    session.shutdown();
    assert!(matches!(
        session.set_favorites_visible(false).await,
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn status_writes_fail_before_rest_when_not_ready_or_shutdown() {
    let (session, mut receiver) = offline_session(0);
    assert!(!session.status_connected());
    assert!(matches!(
        session
            .set_status_for(SelfStatus::Invisible, StatusDuration::Forever)
            .await,
        Err(Error::Closed)
    ));
    assert!(matches!(
        session
            .set_status_for(SelfStatus::Idle, StatusDuration::Minutes15)
            .await,
        Err(Error::Closed)
    ));
    assert!(matches!(
        session.set_theme(settings_proto::AccountTheme::Light).await,
        Err(Error::Closed)
    ));
    session.inner.generation.store(1, Ordering::Release);
    assert!(session.status_connected());
    assert!(matches!(
        session
            .set_status_for(SelfStatus::Idle, StatusDuration::Forever)
            .await,
        Err(Error::Network)
    ));
    assert!(matches!(
        session
            .set_status_for(SelfStatus::Invisible, StatusDuration::Days3)
            .await,
        Err(Error::Network)
    ));
    assert!(matches!(
        session.set_theme(settings_proto::AccountTheme::Dark).await,
        Err(Error::Network)
    ));
    session.shutdown();
    assert!(!session.status_connected());
    assert!(matches!(
        session
            .set_status_for(SelfStatus::Online, StatusDuration::Forever)
            .await,
        Err(Error::Closed)
    ));
    assert!(matches!(
        session.set_theme(settings_proto::AccountTheme::Dark).await,
        Err(Error::Closed)
    ));
    assert!(receiver.try_recv().is_err());
}
