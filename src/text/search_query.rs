use chrono::{Days, Local, NaiveDate, TimeZone};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedSearch {
    pub content: String,
    pub from: Vec<String>,
    pub mentions: Vec<String>,
    pub channels: Vec<String>,
    pub has: Vec<String>,
    pub min_id: Option<u64>,
    pub max_id: Option<u64>,
    pub pinned: Option<bool>,
}

pub const HAS_OPTIONS: [(&str, &str); 9] = [
    ("link", "link"),
    ("embed", "embed"),
    ("poll", "poll"),
    ("file", "file"),
    ("video", "video"),
    ("image", "image"),
    ("sound", "sound"),
    ("sticker", "sticker"),
    ("forward", "snapshot"),
];

const FILTER_KEYS: [&str; 8] = [
    "from", "mentions", "in", "has", "before", "after", "during", "pinned",
];
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

struct Token {
    value: String,
    start: usize,
    end: usize,
    in_quotes: bool,
}

pub fn parse(text: &str) -> Result<ParsedSearch, String> {
    let mut parsed = ParsedSearch::default();
    let mut content = Vec::new();

    for token in tokenize(text) {
        if text[token.start..token.end].starts_with('"') {
            content.push(token.value);
            continue;
        }
        let Some((key, value)) = token.value.split_once(':') else {
            content.push(token.value);
            continue;
        };
        let key = key.to_ascii_lowercase();
        if !FILTER_KEYS.contains(&key.as_str()) || value.is_empty() {
            content.push(token.value);
            continue;
        }

        let value = match key.as_str() {
            "from" | "mentions" => value.strip_prefix('@').unwrap_or(value),
            "in" => value.strip_prefix('#').unwrap_or(value),
            _ => value,
        };
        if value.is_empty() {
            content.push(token.value);
            continue;
        }

        match key.as_str() {
            "from" => parsed.from.push(value.to_owned()),
            "mentions" => parsed.mentions.push(value.to_owned()),
            "in" => parsed.channels.push(value.to_owned()),
            "has" => {
                let Some((_, api_name)) = HAS_OPTIONS
                    .iter()
                    .find(|(typed, _)| typed.eq_ignore_ascii_case(value))
                else {
                    return Err("Unknown has filter.".to_owned());
                };
                parsed.has.push((*api_name).to_owned());
            }
            "before" => {
                let date = parse_date(value)?;
                parsed.max_id = Some(snowflake(local_midnight(date)?));
            }
            "after" => {
                let date = parse_date(value)?;
                let next = date
                    .checked_add_days(Days::new(1))
                    .ok_or_else(invalid_date)?;
                parsed.min_id = Some(snowflake(local_midnight(next)?));
            }
            "during" => {
                let date = parse_date(value)?;
                let next = date
                    .checked_add_days(Days::new(1))
                    .ok_or_else(invalid_date)?;
                parsed.min_id = Some(snowflake(local_midnight(date)?));
                parsed.max_id = Some(snowflake(local_midnight(next)?));
            }
            "pinned" => match value.to_ascii_lowercase().as_str() {
                "true" => parsed.pinned = Some(true),
                "false" => parsed.pinned = Some(false),
                _ => return Err("Use true or false for pinned.".to_owned()),
            },
            _ => unreachable!(),
        }
    }

    parsed.content = content.join(" ");
    Ok(parsed)
}

pub fn active_filter(text: &str) -> Option<(&str, &str)> {
    if text.chars().last().is_some_and(char::is_whitespace) {
        return None;
    }
    let token = tokenize(text).pop()?;
    let raw = &text[token.start..token.end];
    if raw.starts_with('"') || (raw.contains('"') && !token.in_quotes) {
        return None;
    }
    let (key, value) = raw.split_once(':')?;
    let key = FILTER_KEYS
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(key))?;
    let value = value.strip_prefix('"').unwrap_or(value);
    Some((*key, value))
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut value = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut start = 0;

    for (index, character) in text.char_indices() {
        if character == '"' {
            if !started {
                start = index;
            }
            started = true;
            in_quotes = !in_quotes;
        } else if character.is_whitespace() && !in_quotes {
            if started {
                tokens.push(Token {
                    value: std::mem::take(&mut value),
                    start,
                    end: index,
                    in_quotes,
                });
                started = false;
            }
        } else {
            if !started {
                start = index;
            }
            started = true;
            value.push(character);
        }
    }
    if started {
        tokens.push(Token {
            value,
            start,
            end: text.len(),
            in_quotes,
        });
    }
    tokens
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(invalid_date());
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid_date())
}

fn local_midnight(date: NaiveDate) -> Result<i64, String> {
    let midnight = date.and_hms_opt(0, 0, 0).ok_or_else(invalid_date)?;
    Local
        .from_local_datetime(&midnight)
        .single()
        .map(|date| date.timestamp_millis())
        .ok_or_else(invalid_date)
}

fn snowflake(timestamp_ms: i64) -> u64 {
    (timestamp_ms.saturating_sub(DISCORD_EPOCH_MS).max(0) as u64).saturating_mul(1 << 22)
}

fn invalid_date() -> String {
    "Use a valid date in YYYY-MM-DD format.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_parser_handles_filters_quotes_and_case() {
        let parsed =
            parse("hello from:\"Carol M\" MENTIONS:@alex in:#general has:forward pinned:TRUE")
                .unwrap();
        assert_eq!(parsed.content, "hello");
        assert_eq!(parsed.from, ["Carol M"]);
        assert_eq!(parsed.mentions, ["alex"]);
        assert_eq!(parsed.channels, ["general"]);
        assert_eq!(parsed.has, ["snapshot"]);
        assert_eq!(parsed.pinned, Some(true));
    }

    #[test]
    fn search_parser_preserves_phrases_and_repeated_filters() {
        let parsed = parse(
            "  hello\t\"a   phrase\" from:@alex FROM:\"@Carol M\" mentions:123 \
             mentions:\"@Alex B\" IN:#general in:\"#Team Chat\" pinned:false  ",
        )
        .unwrap();
        assert_eq!(parsed.content, "hello a   phrase");
        assert_eq!(parsed.from, ["alex", "Carol M"]);
        assert_eq!(parsed.mentions, ["123", "Alex B"]);
        assert_eq!(parsed.channels, ["general", "Team Chat"]);
        assert_eq!(parsed.pinned, Some(false));
        assert_eq!(
            parse("\"from:alex\" \"has:unknown\"").unwrap().content,
            "from:alex has:unknown"
        );
        assert_eq!(parse("hello from:\"Carol M").unwrap().from, ["Carol M"]);
        assert_eq!(
            parse("\"unfinished phrase").unwrap().content,
            "unfinished phrase"
        );
        assert_eq!(parse("\t  \n").unwrap(), ParsedSearch::default());
    }

    #[test]
    fn search_parser_maps_every_has_option() {
        let text = HAS_OPTIONS
            .iter()
            .map(|(typed, _)| format!("HAS:{}", typed.to_uppercase()))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            parse(&text).unwrap().has,
            HAS_OPTIONS.map(|(_, api)| api.to_owned())
        );
        assert!(parse("has:snapshot").is_err());
    }

    #[test]
    fn search_parser_keeps_unknown_and_empty_filters_as_content() {
        let parsed = parse("https://x at: custom:value from:").unwrap();
        assert_eq!(parsed.content, "https://x at: custom:value from:");
        let text = "from:@ mentions:@ in:# from: mentions: in: has: before: after: during: pinned:";
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.content, text);
        assert!(parsed.from.is_empty());
        assert!(parsed.mentions.is_empty());
        assert!(parsed.channels.is_empty());
        assert_eq!(
            parse("from:\"\" custom:\"two words\"").unwrap().content,
            "from: custom:two words"
        );
    }

    #[test]
    fn search_parser_rejects_bad_values() {
        assert!(parse("has:unknown").is_err());
        assert!(parse("before:2024-2-03").is_err());
        assert!(parse("during:2024-02-30").is_err());
        assert!(parse("pinned:sometimes").is_err());
        for value in [
            "2023-02-29",
            "2024-13-01",
            "2024-01-32",
            "24-01-01",
            "2024-01-01T00:00",
            "not-a-date",
        ] {
            for key in ["before", "after", "during"] {
                assert!(parse(&format!("{key}:{value}")).is_err());
            }
        }
    }

    #[test]
    fn search_parser_converts_dates_to_local_day_snowflakes() {
        let parsed = parse("during:2024-03-05").unwrap();
        let during_start = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
        let during_end = during_start.checked_add_days(Days::new(1)).unwrap();
        assert_eq!(
            parsed.min_id,
            Some(snowflake(local_midnight(during_start).unwrap()))
        );
        assert_eq!(
            parsed.max_id,
            Some(snowflake(local_midnight(during_end).unwrap()))
        );
        let before_after = parse("before:2024-03-02 after:2024-03-02").unwrap();
        let date = NaiveDate::from_ymd_opt(2024, 3, 2).unwrap();
        assert_eq!(
            before_after.min_id,
            Some(snowflake(
                local_midnight(date.checked_add_days(Days::new(1)).unwrap()).unwrap()
            ))
        );
        assert_eq!(
            before_after.max_id,
            Some(snowflake(local_midnight(date).unwrap()))
        );
        let leap_day = parse("DURING:2024-02-29").unwrap();
        let start = Local
            .with_ymd_and_hms(2024, 2, 29, 0, 0, 0)
            .single()
            .unwrap();
        let end = Local
            .with_ymd_and_hms(2024, 3, 1, 0, 0, 0)
            .single()
            .unwrap();
        assert_eq!(
            leap_day.min_id,
            Some(((start.timestamp_millis() - DISCORD_EPOCH_MS) as u64) << 22)
        );
        assert_eq!(
            leap_day.max_id,
            Some(((end.timestamp_millis() - DISCORD_EPOCH_MS) as u64) << 22)
        );
        assert_eq!(parse("during:2010-01-01").unwrap().min_id, Some(0));
        assert_eq!(parse("during:9999-12-30").unwrap().max_id, Some(u64::MAX));
    }

    #[test]
    fn search_snowflakes_saturate_at_both_bounds() {
        assert_eq!(snowflake(i64::MIN), 0);
        assert_eq!(snowflake(DISCORD_EPOCH_MS - 1), 0);
        assert_eq!(snowflake(DISCORD_EPOCH_MS), 0);
        assert_eq!(snowflake(DISCORD_EPOCH_MS + 1), 1 << 22);
        assert_eq!(snowflake(i64::MAX), u64::MAX);
    }

    #[test]
    fn search_active_filter_tracks_only_an_open_final_filter() {
        assert_eq!(active_filter("hi from:bo"), Some(("from", "bo")));
        assert_eq!(active_filter("in:"), Some(("in", "")));
        assert_eq!(active_filter("from:\""), Some(("from", "")));
        assert_eq!(active_filter("hi IN:\""), Some(("in", "")));
        assert_eq!(
            active_filter("hi from:\"Carol M"),
            Some(("from", "Carol M"))
        );
        assert_eq!(active_filter("hi from:\"Carol M\""), None);
        assert_eq!(active_filter("hi from:bo "), None);
        assert_eq!(active_filter("hi https://x"), None);
        assert_eq!(active_filter("hi custom:bo"), None);
        assert_eq!(active_filter("from:\"\""), None);
        assert_eq!(active_filter("from:\"Carol\"more"), None);
        assert_eq!(active_filter("\"from:bo\""), None);
        assert_eq!(active_filter("\"from:bo"), None);
        assert_eq!(active_filter("from:bo hi"), None);
        assert_eq!(active_filter("from:bo\t"), None);
        assert_eq!(active_filter("from:bo\n"), None);
        assert_eq!(active_filter(""), None);
        assert_eq!(
            active_filter("hi\u{2003}FROM:@\u{00e9}"),
            Some(("from", "@\u{00e9}"))
        );
    }
}
