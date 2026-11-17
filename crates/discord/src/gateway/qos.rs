use serde_json::{Value, json};

const VERSION: u8 = 31;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Activity {
    active: bool,
    reasons: Vec<&'static str>,
}

impl Activity {
    pub fn new(foregrounded: bool, rtc_connected: bool) -> Self {
        let reasons: Vec<_> = [
            (foregrounded, "foregrounded"),
            (rtc_connected, "rtc_connected"),
        ]
        .into_iter()
        .filter_map(|(on, reason)| on.then_some(reason))
        .collect();
        Self {
            active: !reasons.is_empty(),
            reasons,
        }
    }
}

/// A beat reports the whole span since the previous beat, not the current state: activity in
/// between must still show up, with every reason it had.
#[derive(Default)]
pub(super) struct Qos {
    observed: Option<Activity>,
    current: Option<Activity>,
    upcoming: Option<Activity>,
}

impl Qos {
    pub fn woke_by(&mut self, activity: Activity) -> bool {
        if self.observed.as_ref() == Some(&activity) {
            return false;
        }
        self.observed = Some(activity.clone());
        let was_inactive = !self.current.as_ref().is_some_and(|current| current.active);
        let current = self.current.get_or_insert_with(|| activity.clone());
        if activity.active {
            current.active = true;
            current.reasons.extend(&activity.reasons);
            current.reasons.sort_unstable();
            current.reasons.dedup();
        }
        let woke = was_inactive && activity.active;
        self.upcoming = Some(activity);
        woke
    }

    pub fn heartbeat(&mut self, seq: u64) -> Value {
        let qos = self.current.as_ref().map(|activity| {
            json!({"active": activity.active, "ver": VERSION, "reasons": activity.reasons})
        });
        if let Some(upcoming) = self.upcoming.take() {
            self.current = Some(upcoming);
        }
        json!({"op": 40, "d": {"seq": seq, "qos": qos}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qos(value: &Value) -> Value {
        value["d"]["qos"].clone()
    }

    #[test]
    fn heartbeats_report_accumulated_activity_then_the_latest_state() {
        let mut state = Qos::default();
        assert_eq!(
            state.heartbeat(0),
            json!({"op":40,"d":{"seq":0,"qos":null}})
        );
        assert!(state.woke_by(Activity::new(true, false)));
        assert!(!state.woke_by(Activity::new(true, false)));
        assert_eq!(
            qos(&state.heartbeat(3)),
            json!({"active":true,"ver":31,"reasons":["foregrounded"]})
        );
        assert!(!state.woke_by(Activity::new(false, true)));
        assert!(!state.woke_by(Activity::new(false, false)));
        assert_eq!(
            qos(&state.heartbeat(4)),
            json!({"active":true,"ver":31,"reasons":["foregrounded","rtc_connected"]})
        );
        assert_eq!(
            qos(&state.heartbeat(5)),
            json!({"active":false,"ver":31,"reasons":[]})
        );
        assert!(state.woke_by(Activity::new(false, true)));
        let beat = state.heartbeat(6);
        assert_eq!(beat["d"]["seq"], 6);
        assert_eq!(
            qos(&beat),
            json!({"active":true,"ver":31,"reasons":["rtc_connected"]})
        );
    }

    #[test]
    fn a_first_inactive_observation_is_reported_as_is() {
        let mut state = Qos::default();
        assert!(!state.woke_by(Activity::new(false, false)));
        assert_eq!(
            qos(&state.heartbeat(0)),
            json!({"active":false,"ver":31,"reasons":[]})
        );
    }
}
