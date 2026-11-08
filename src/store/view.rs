mod guilds;
mod invites;
mod messages;
mod relationships;
mod users;

pub use guilds::*;
pub use invites::*;
pub use messages::*;
pub use relationships::*;
pub use users::*;

use crate::discord::model;
pub use crate::discord::model::Attachment;
use crate::media::images::ImageSpec;
use std::sync::Arc;
use std::time::SystemTime;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(kind: u8, flags: u64) -> model::Message {
        serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "author": {"id": "3", "username": "tester", "discriminator": "0"},
            "timestamp": "2024-01-01T00:00:00+00:00",
            "type": kind,
            "flags": flags,
        }))
        .unwrap()
    }

    #[test]
    fn forwardable_plain_message() {
        assert!(is_forwardable(&message(0, 0)));
    }

    #[test]
    fn forwardable_rejects_kind_7() {
        assert!(!is_forwardable(&message(7, 0)));
    }

    #[test]
    fn forwardable_rejects_ephemeral_flag() {
        assert!(!is_forwardable(&message(0, 1 << 6)));
    }

    #[test]
    fn forwardable_rejects_poll() {
        let mut message = message(0, 0);
        message.poll = Some(json!({}));
        assert!(!is_forwardable(&message));
    }

    #[test]
    fn header_stamp_names_today_and_yesterday_then_the_full_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let at = |month, day| Timestamp {
            year: 2026,
            month,
            day,
            hour: 4,
            minute: 21,
        };
        // Another test flips the clock format, so compare against `time()` itself.
        let time = at(11, 7).time();
        assert_eq!(at(11, 7).header_on(today), time);
        assert_eq!(at(11, 6).header_on(today), format!("Yesterday at {time}"));
        assert_eq!(at(11, 5).header_on(today), format!("11/5/2026 {time}"));
        assert_eq!(at(11, 8).header_on(today), format!("11/8/2026 {time}"));
    }

    #[test]
    fn forwarded_stamp_names_the_time_yesterday_and_weekdays_then_the_date() {
        let saturday = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let at = |month, day| Timestamp {
            year: 2026,
            month,
            day,
            hour: 4,
            minute: 26,
        };
        assert_eq!(at(11, 7).forwarded_on(saturday), at(11, 7).time());
        assert_eq!(at(11, 6).forwarded_on(saturday), "Yesterday");
        assert_eq!(at(11, 5).forwarded_on(saturday), "Thursday");
        assert_eq!(at(11, 2).forwarded_on(saturday), "Monday");
        assert_eq!(at(11, 1).forwarded_on(saturday), "Sunday");
        assert_eq!(at(10, 31).forwarded_on(saturday), "10/31/2026");
        assert_eq!(at(11, 8).forwarded_on(saturday), "11/08/2026");
    }
}
