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

    pub fn proto(&self) -> Option<&[u8]> {
        self.proto.as_deref()
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
mod tests;
