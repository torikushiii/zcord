use super::*;

#[test]
fn invite_view_prefers_custom_banner_and_resolves_game_icons() {
    let invite: model::Invite = serde_json::from_value(json!({
        "code": "sampleInviteTwo",
        "expires_at": "2024-02-01T00:00:00+00:00",
        "guild": {
            "id": "1191168993918976019",
            "name": "Sample Server One",
            "icon": "f0000000000000000000000000000020",
            "banner": "f0000000000000000000000000000002",
            "splash": "f0000000000000000000000000000013"
        },
        "guild_id": "1191168993918976019",
        "profile": {
            "id": "1191168993918976019",
            "name": "Sample Server One",
            "icon_hash": "f0000000000000000000000000000020",
            "member_count": 1234,
            "online_count": 321,
            "banner_hash": null,
            "custom_banner_hash": "f0000000000000000000000000000019",
            "game_application_ids": ["1191169086193664041"]
        },
        "approximate_member_count": 1234,
        "approximate_presence_count": 321
    }))
    .unwrap();
    let applications = [std::sync::Arc::new(model::PublicApplication {
        id: 1191169086193664041,
        name: "Example World Game".into(),
        icon: Some("iconhash".into()),
        cover_image: None,
        deeplink_uri: None,
        steam_app_id: None,
        linked_games: Vec::new(),
    })];
    let invite = Invite::from_model(&invite, &applications).unwrap();
    assert_eq!(invite.name, "Sample Server One");
    assert_eq!(invite.members, 1234);
    assert_eq!(invite.online, 321);
    assert_eq!(invite.games, [1191169086193664041]);
    assert_eq!(
        invite.banner,
        Some(InviteBanner::Custom(
            "f0000000000000000000000000000019".into()
        ))
    );
    assert_eq!(
        invite.banner_spec(),
        Some(ImageSpec::DiscoverySplash {
            guild_id: 1191168993918976019,
            hash: "f0000000000000000000000000000019".into()
        })
    );
    assert_eq!(
        invite.icon_spec(),
        Some(ImageSpec::GuildIcon {
            guild_id: 1191168993918976019,
            hash: "f0000000000000000000000000000020".into()
        })
    );
    assert!((invite.created_at.year, invite.created_at.month) == (2024, 1));
    assert!(invite.expires_at.is_some());
    let game = &invite.game_apps[0];
    assert_eq!(game.name, "Example World Game");
    assert_eq!(
        game.icon_spec(),
        Some(ImageSpec::AppImage {
            application_id: 1191169086193664041,
            hash: "iconhash".into(),
            uploaded: false,
            animated: false,
        })
    );

    // Without a banner the splash is used; a guild-less invite has no card.
    let splash: model::Invite = serde_json::from_value(json!({
        "code": "splash",
        "guild_id": "1",
        "guild": {"id": "1", "name": "Splash", "splash": "abc"}
    }))
    .unwrap();
    let splash = Invite::from_model(&splash, &[]).unwrap();
    assert_eq!(splash.banner, Some(InviteBanner::Splash("abc".into())));
    assert_eq!(
        splash.banner_spec(),
        Some(ImageSpec::GuildSplash {
            guild_id: 1,
            hash: "abc".into()
        })
    );
    let group: model::Invite = serde_json::from_value(json!({"code": "group"})).unwrap();
    assert!(Invite::from_model(&group, &[]).is_none());
}
