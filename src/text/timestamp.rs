use chrono::{Local, TimeZone};

use crate::settings;

/// Renders a `<t:unix:style>` timestamp the way Discord shows it.
pub fn format_timestamp(unix: i64, style: char) -> String {
    let Some(time) = Local.timestamp_opt(unix, 0).single() else {
        return unix.to_string();
    };
    match style {
        't' => time
            .format(settings::clock_format("%-I:%M %p", "%H:%M"))
            .to_string(),
        'T' => time
            .format(settings::clock_format("%-I:%M:%S %p", "%H:%M:%S"))
            .to_string(),
        'd' => time.format("%-m/%-d/%Y").to_string(),
        'D' => time.format("%B %-d, %Y").to_string(),
        'F' => time
            .format(settings::clock_format(
                "%A, %B %-d, %Y %-I:%M %p",
                "%A, %B %-d, %Y %H:%M",
            ))
            .to_string(),
        'R' => relative(unix - Local::now().timestamp()),
        _ => time
            .format(settings::clock_format(
                "%B %-d, %Y %-I:%M %p",
                "%B %-d, %Y %H:%M",
            ))
            .to_string(),
    }
}

fn relative(delta: i64) -> String {
    let seconds = delta.unsigned_abs();
    let (amount, unit) = match seconds {
        0..60 => (seconds, "second"),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let unit = if amount == 1 {
        unit.to_owned()
    } else {
        format!("{unit}s")
    };
    if delta >= 0 {
        format!("in {amount} {unit}")
    } else {
        format!("{amount} {unit} ago")
    }
}
