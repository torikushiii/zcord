use super::{Error, settings_proto};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfStatus {
    Online,
    Idle,
    DoNotDisturb,
    Invisible,
}

impl SelfStatus {
    pub const ALL: [Self; 4] = [
        Self::Online,
        Self::Idle,
        Self::DoNotDisturb,
        Self::Invisible,
    ];

    pub fn wire(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::DoNotDisturb => "dnd",
            Self::Invisible => "invisible",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Idle => "Idle",
            Self::DoNotDisturb => "Do Not Disturb",
            Self::Invisible => "Invisible",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "online" => Self::Online,
            "idle" => Self::Idle,
            "dnd" => Self::DoNotDisturb,
            "invisible" | "offline" => Self::Invisible,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusDuration {
    Minutes15,
    Hour1,
    Hours8,
    Hours24,
    Days3,
    Forever,
}

impl StatusDuration {
    pub const ALL: [Self; 6] = [
        Self::Minutes15,
        Self::Hour1,
        Self::Hours8,
        Self::Hours24,
        Self::Days3,
        Self::Forever,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Minutes15 => "For 15 Minutes",
            Self::Hour1 => "For 1 Hour",
            Self::Hours8 => "For 8 Hours",
            Self::Hours24 => "For 24 Hours",
            Self::Days3 => "For 3 Days",
            Self::Forever => "Forever",
        }
    }

    pub(super) fn deadline_ms(self, now_ms: u64) -> Result<Option<u64>, Error> {
        let seconds = match self {
            Self::Minutes15 => 15 * 60,
            Self::Hour1 => 60 * 60,
            Self::Hours8 => 8 * 60 * 60,
            Self::Hours24 => 24 * 60 * 60,
            Self::Days3 => 3 * 24 * 60 * 60,
            Self::Forever => return Ok(None),
        };
        now_ms
            .checked_add(seconds * 1000)
            .map(Some)
            .ok_or(Error::Decode)
    }
}

/// Aggregated sessions/presences contain other clients' RPC activities and must never be
/// copied into this client's outgoing presence.
#[derive(Default)]
pub(super) struct Presence {
    proto: Option<Vec<u8>>,
    local_activity: Option<Value>,
    pub revision: u64,
    pub settings_revision: u64,
}

impl Presence {
    pub fn set_local_activity(&mut self, activity: Option<Value>) -> bool {
        if self.local_activity == activity {
            return false;
        }
        self.local_activity = activity;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn shows_current_game(&self) -> bool {
        !matches!(
            self.proto
                .as_deref()
                .and_then(|proto| settings_proto::decode(proto).ok())
                .and_then(|settings| settings.status)
                .and_then(|status| status.show_current_game),
            Some(false)
        )
    }

    pub fn replace(&mut self, encoded: Option<&str>) {
        self.proto = encoded
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .filter(|proto| {
                settings_proto::decode(proto).is_ok()
                    && settings_proto::status_timing(proto).is_ok()
            });
        self.revision = self.revision.wrapping_add(1);
        self.settings_revision = self.settings_revision.wrapping_add(1);
    }

    pub fn update(&mut self, encoded: &str, partial: bool) -> Result<bool, Error> {
        let update = STANDARD.decode(encoded).map_err(|_| Error::Decode)?;
        let proto = if partial {
            let base = self.proto.as_deref().ok_or(Error::Decode)?;
            settings_proto::merge_partial(base, &update).map_err(|_| Error::Decode)?
        } else {
            update
        };
        let revision = self.revision;
        self.install(proto)?;
        Ok(self.revision != revision)
    }

    pub fn install(&mut self, proto: Vec<u8>) -> Result<(), Error> {
        settings_proto::decode(&proto).map_err(|_| Error::Decode)?;
        settings_proto::status_timing(&proto).map_err(|_| Error::Decode)?;
        let next = settings_proto::data_version(&proto).map_err(|_| Error::Decode)?;
        let current = self
            .proto
            .as_deref()
            .and_then(|proto| settings_proto::data_version(proto).ok().flatten());
        if current.is_some_and(|current| next.is_none_or(|next| next < current)) {
            return Err(Error::SettingsOutdated);
        }
        if self.proto.as_ref() != Some(&proto) {
            let previous = self.payload(0);
            let timing = self.timing();
            self.proto = Some(proto);
            if self.payload(0) != previous || self.timing() != timing {
                self.revision = self.revision.wrapping_add(1);
                self.settings_revision = self.settings_revision.wrapping_add(1);
            }
        }
        Ok(())
    }

    pub fn status(&self) -> Option<SelfStatus> {
        self.status_at(chrono::Utc::now().timestamp_millis().max(0) as u64)
    }

    pub fn configured_status(&self) -> Option<SelfStatus> {
        let settings = settings_proto::decode(self.proto.as_deref()?).ok()?;
        SelfStatus::parse(settings.status?.status.as_deref()?)
    }

    pub fn timing(&self) -> Option<settings_proto::StatusTiming> {
        settings_proto::status_timing(self.proto.as_deref()?).ok()
    }

    pub fn status_at(&self, now_ms: u64) -> Option<SelfStatus> {
        let status = self.configured_status()?;
        let expiry = self.timing()?.expires_at_ms;
        Some(if expiry != 0 && now_ms >= expiry {
            SelfStatus::Online
        } else {
            status
        })
    }

    pub fn next_status_expiry_at(&self, now_ms: u64) -> Option<SystemTime> {
        if self.configured_status()? == SelfStatus::Online {
            return None;
        }
        let expiry = self.timing()?.expires_at_ms;
        if expiry == 0 || expiry <= now_ms {
            return None;
        }
        UNIX_EPOCH.checked_add(Duration::from_millis(expiry))
    }

    pub fn payload(&self, now_ms: u64) -> Option<Value> {
        let settings = settings_proto::decode(self.proto.as_deref()?).ok()?;
        let settings = settings.status?;
        let status = self.status_at(now_ms)?;
        let mut activities = Vec::new();
        if let Some(custom) = settings
            .custom_status
            .filter(|custom| custom.expires_at_ms == 0 || custom.expires_at_ms > now_ms)
        {
            let mut activity = json!({"name":"Custom Status", "type":4});
            if !custom.text.is_empty() {
                activity["state"] = custom.text.into();
            }
            if custom.emoji_id != 0 {
                activity["emoji"] = json!({
                    "id": custom.emoji_id.to_string(), "name": custom.emoji_name
                });
            } else if !custom.emoji_name.is_empty() {
                activity["emoji"] = json!({"name":custom.emoji_name});
            }
            activities.push(activity);
        }
        if settings.show_current_game != Some(false) {
            if let Some(activity) = &self.local_activity {
                activities.push(activity.clone());
            }
        }
        Some(json!({"op":3,"d":{
            "status":status.wire(),"since":if status == SelfStatus::Idle { now_ms } else { 0 },
            "afk":false,"activities":activities
        }}))
    }
}

#[cfg(test)]
mod tests {
    use super::{Presence, SelfStatus, StatusDuration};
    use crate::discord::{
        Error,
        settings_fixtures::{field, status_fixture, uint},
        settings_proto,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use std::time::{Duration, UNIX_EPOCH};

    fn snapshot(status: &str, version: u64) -> Vec<u8> {
        let mut proto = STANDARD
            .decode(status_fixture(
                Some(status),
                Some(("working", 42, "party", 0)),
            ))
            .unwrap();
        proto.extend(field(1, &uint(3, version)));
        proto
    }

    fn timed_snapshot(status: &str, version: u64, expires: u64, created: u64) -> Vec<u8> {
        let base = snapshot(status, version);
        let patch = settings_proto::status_patch_at(&base, status, expires, Some(created)).unwrap();
        settings_proto::merge_partial(&base, &patch).unwrap()
    }

    #[test]
    fn duration_choices_match_reference_order_labels_and_checked_timestamp_math() {
        assert_eq!(
            StatusDuration::ALL.map(StatusDuration::label),
            [
                "For 15 Minutes",
                "For 1 Hour",
                "For 8 Hours",
                "For 24 Hours",
                "For 3 Days",
                "Forever",
            ]
        );
        let milliseconds = [900_000, 3_600_000, 28_800_000, 86_400_000, 259_200_000];
        for (choice, milliseconds) in StatusDuration::ALL.into_iter().zip(milliseconds) {
            assert_eq!(choice.deadline_ms(123).unwrap(), Some(123 + milliseconds));
            assert!(matches!(choice.deadline_ms(u64::MAX), Err(Error::Decode)));
        }
        assert_eq!(StatusDuration::Forever.deadline_ms(u64::MAX).unwrap(), None);
    }

    #[test]
    fn manual_status_expires_exactly_to_online_without_clearing_custom_status() {
        for status in [
            SelfStatus::Idle,
            SelfStatus::DoNotDisturb,
            SelfStatus::Invisible,
        ] {
            let mut presence = Presence::default();
            presence
                .install(timed_snapshot(status.wire(), 1, 2_000, 1_000))
                .unwrap();
            let custom = presence.payload(1_999).unwrap()["d"]["activities"].clone();
            assert_eq!(custom.as_array().unwrap().len(), 1);
            assert_eq!(presence.status_at(1_999), Some(status));
            assert_eq!(
                presence.next_status_expiry_at(1_999),
                Some(UNIX_EPOCH + Duration::from_millis(2_000))
            );
            for now in [2_000, 2_001, u64::MAX] {
                assert_eq!(presence.status_at(now), Some(SelfStatus::Online));
                let payload = presence.payload(now).unwrap();
                assert_eq!(payload["d"]["status"], "online");
                assert_eq!(payload["d"]["since"], 0);
                assert_eq!(payload["d"]["activities"], custom);
                assert_eq!(presence.next_status_expiry_at(now), None);
            }
        }
    }

    #[test]
    fn reconnect_keeps_absolute_deadline_and_newer_timing_supersedes_old_timer_state() {
        let original = timed_snapshot("idle", 1, 901_000, 1_000);
        let mut presence = Presence::default();
        presence.install(original.clone()).unwrap();
        let deadline = UNIX_EPOCH + Duration::from_millis(901_000);
        assert_eq!(presence.next_status_expiry_at(1_000), Some(deadline));
        presence.replace(Some(&STANDARD.encode(&original)));
        assert_eq!(presence.next_status_expiry_at(400_000), Some(deadline));
        assert_eq!(presence.status_at(901_000), Some(SelfStatus::Online));

        let revision = presence.revision;
        let newer = timed_snapshot("idle", 2, 1_500_000, 600_000);
        let mut partial =
            settings_proto::status_patch_at(&original, "idle", 1_500_000, Some(600_000)).unwrap();
        partial.extend(field(1, &uint(3, 2)));
        assert!(presence.update(&STANDARD.encode(&partial), true).unwrap());
        assert!(
            presence.revision > revision,
            "a timing-only edit invalidates older commands"
        );
        assert_eq!(presence.status_at(901_000), Some(SelfStatus::Idle));
        assert_eq!(presence.payload(901_000).unwrap()["d"]["status"], "idle");
        assert_eq!(
            presence.next_status_expiry_at(901_000),
            Some(UNIX_EPOCH + Duration::from_millis(1_500_000))
        );
        assert!(matches!(
            presence.install(original),
            Err(Error::SettingsOutdated)
        ));
        assert_eq!(presence.status_at(901_000), Some(SelfStatus::Idle));

        let clear = settings_proto::status_patch(&newer, "invisible").unwrap();
        let clear = settings_proto::merge_partial(&newer, &clear).unwrap();
        presence.install(clear).unwrap();
        assert_eq!(presence.status_at(u64::MAX), Some(SelfStatus::Invisible));
        assert_eq!(presence.next_status_expiry_at(0), None);
    }

    #[test]
    fn only_a_known_non_online_future_manual_deadline_is_schedulable() {
        let mut presence = Presence::default();
        assert_eq!(presence.next_status_expiry_at(0), None);
        for (status, deadline) in [("idle", 0), ("idle", 1), ("unknown", 0), ("online", 3_000)] {
            presence.replace(Some(
                &STANDARD.encode(timed_snapshot(status, 1, deadline, 0)),
            ));
            assert_eq!(presence.next_status_expiry_at(2_000), None);
        }
        // A custom activity expiring first cannot shorten the separate manual deadline.
        let mut base = STANDARD
            .decode(status_fixture(Some("dnd"), Some(("expired", 0, "", 1))))
            .unwrap();
        base.extend(field(1, &uint(3, 2)));
        let patch = settings_proto::status_patch_at(&base, "dnd", 3_000, Some(2_000)).unwrap();
        presence
            .install(settings_proto::merge_partial(&base, &patch).unwrap())
            .unwrap();
        assert_eq!(presence.status_at(2_000), Some(SelfStatus::DoNotDisturb));
        assert_eq!(
            presence.payload(2_000).unwrap()["d"]["activities"],
            json!([])
        );
        assert_eq!(
            presence.next_status_expiry_at(2_000),
            Some(UNIX_EPOCH + Duration::from_millis(3_000))
        );
    }

    #[test]
    fn custom_activity_is_withdrawn_at_the_exact_expiry_boundary() {
        let mut presence = Presence::default();
        presence.replace(Some(&status_fixture(
            Some("invisible"),
            Some(("expiring", 0, "", u64::MAX)),
        )));
        assert_eq!(
            presence.payload(u64::MAX - 1).unwrap()["d"]["activities"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let expired = presence.payload(u64::MAX).unwrap();
        assert_eq!(expired["d"]["activities"], json!([]));
        assert_eq!(expired["d"]["status"], "invisible");
    }

    #[test]
    fn choices_round_trip_and_invisible_is_never_sent_as_offline() {
        for status in SelfStatus::ALL {
            assert_eq!(SelfStatus::parse(status.wire()), Some(status));
            assert!(!status.label().is_empty());
            let mut presence = Presence::default();
            presence.install(snapshot(status.wire(), 1)).unwrap();
            let payload = presence.payload(1234).unwrap();
            assert_eq!(payload["op"], 3);
            assert_eq!(payload["d"]["status"], status.wire());
            assert_eq!(
                payload["d"]["since"],
                if status == SelfStatus::Idle { 1234 } else { 0 }
            );
            assert_eq!(payload["d"]["afk"], false);
            assert_eq!(
                payload["d"]["activities"],
                json!([{
                    "name":"Custom Status","type":4,"state":"working",
                    "emoji":{"id":"42","name":"party"}
                }])
            );
        }
        assert_eq!(SelfStatus::parse("offline"), Some(SelfStatus::Invisible));
        assert_eq!(SelfStatus::parse("unknown"), None);
    }

    #[test]
    fn ready_unknown_does_not_force_online_and_reconnect_uses_current_settings() {
        let mut presence = Presence::default();
        presence.replace(Some(&status_fixture(Some("unknown"), None)));
        assert!(presence.payload(0).is_none());
        presence.replace(Some(&STANDARD.encode(snapshot("invisible", 3))));
        assert_eq!(presence.payload(0).unwrap()["d"]["status"], "invisible");
        let revision = presence.revision;
        assert!(
            presence
                .update(&STANDARD.encode(snapshot("dnd", 4)), false)
                .unwrap()
        );
        assert!(presence.revision > revision);
        assert!(matches!(
            presence.install(snapshot("online", 3)),
            Err(Error::SettingsOutdated)
        ));
        assert!(matches!(
            presence.update(&STANDARD.encode(snapshot("online", 3)), false),
            Err(Error::SettingsOutdated)
        ));
        assert_eq!(presence.status(), Some(SelfStatus::DoNotDisturb));
        // A fresh READY is authoritative even when this is a different account/version.
        presence.replace(Some(&STANDARD.encode(snapshot("idle", 1))));
        assert_eq!(presence.status(), Some(SelfStatus::Idle));
    }

    fn snapshot_with_game(show: Option<bool>) -> Vec<u8> {
        let mut custom = field(1, b"working");
        custom.push(17);
        custom.extend(42u64.to_le_bytes());
        custom.extend(field(3, b"party"));
        custom.push(33);
        custom.extend(0u64.to_le_bytes());
        let mut status = field(1, &field(1, b"online"));
        status.extend(field(2, &custom));
        if let Some(show) = show {
            let value = if show { uint(1, 1) } else { Vec::new() };
            status.extend(field(3, &value));
        }
        field(11, &status)
    }

    #[test]
    fn local_activity_follows_custom_status_and_respects_account_privacy() {
        let mut presence = Presence::default();
        presence.install(snapshot_with_game(None)).unwrap();
        assert!(presence.shows_current_game());
        assert!(presence.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
        let activities = presence.payload(0).unwrap()["d"]["activities"].clone();
        assert_eq!(activities[0]["name"], "Custom Status");
        assert_eq!(activities[1]["name"], "Game");

        let mut hidden = Presence::default();
        hidden.install(snapshot_with_game(Some(false))).unwrap();
        assert!(!hidden.shows_current_game());
        assert!(hidden.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
        assert_eq!(
            hidden.payload(0).unwrap()["d"]["activities"],
            json!([{
                "name":"Custom Status","type":4,"state":"working",
                "emoji":{"id":"42","name":"party"}
            }])
        );

        let mut shown = Presence::default();
        shown.install(snapshot_with_game(Some(true))).unwrap();
        assert!(shown.shows_current_game());
    }

    #[test]
    fn local_activity_revision_bumps_only_on_change() {
        let mut presence = Presence::default();
        presence.install(snapshot_with_game(None)).unwrap();
        let revision = presence.revision;
        let activity = json!({"type":0,"name":"Game"});
        assert!(presence.set_local_activity(Some(activity.clone())));
        assert_eq!(presence.revision, revision.wrapping_add(1));
        assert!(!presence.set_local_activity(Some(activity.clone())));
        assert_eq!(presence.revision, revision.wrapping_add(1));
        assert!(presence.set_local_activity(None));
        assert_eq!(presence.revision, revision.wrapping_add(2));
        assert!(!presence.set_local_activity(None));
        assert_eq!(presence.revision, revision.wrapping_add(2));
    }

    #[test]
    fn activity_change_during_a_status_write_does_not_look_outdated() {
        let mut presence = Presence::default();
        presence.install(snapshot_with_game(None)).unwrap();
        let settings_revision = presence.settings_revision;
        assert!(presence.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
        assert_eq!(presence.settings_revision, settings_revision);
        assert_ne!(presence.revision, settings_revision);
    }

    #[test]
    fn unrelated_or_malformed_settings_do_not_publish_presence_or_erase_custom_status() {
        let mut presence = Presence::default();
        presence.install(snapshot("invisible", 1)).unwrap();
        let expected = presence.payload(0);
        let revision = presence.revision;
        assert!(
            !presence
                .update(&STANDARD.encode(field(99, b"unrelated")), true)
                .unwrap()
        );
        assert_eq!(presence.revision, revision);
        assert_eq!(presence.payload(0), expected);
        assert!(presence.update("not base64", true).is_err());
        assert_eq!(presence.payload(0), expected);
        presence.replace(Some(&status_fixture(
            Some("online"),
            Some(("expired", 0, "", 1)),
        )));
        assert_eq!(presence.payload(0).unwrap()["d"]["activities"], json!([]));
    }
}
