use super::serde_helpers::*;
use serde::Deserialize;

const INVITE_HOSTS: [(&str, bool); 3] = [
    ("discord.gg/", false),
    ("discord.com/invite/", true),
    ("discordapp.com/invite/", true),
];
const INVITE_LIMIT: usize = 3;
const INVITE_CODE_MAX: usize = 32;

pub fn invite_codes(content: &str) -> Vec<String> {
    let mut found: Vec<(usize, usize, bool)> = Vec::new();
    for (host, subdomains) in INVITE_HOSTS {
        for (index, _) in content.match_indices(host) {
            found.push((index, host.len(), subdomains));
        }
    }
    found.sort_by_key(|(index, _, _)| *index);

    let mut codes: Vec<String> = Vec::new();
    let mut consumed = 0;
    for (index, prefix_len, subdomains) in found {
        if index < consumed {
            continue;
        }
        let Some(start) = invite_url_start(content, index, subdomains) else {
            continue;
        };
        if !invite_link_boundary(content, start) {
            continue;
        }
        let code_start = index + prefix_len;
        let code_end = content[code_start..]
            .find(|c: char| !is_invite_code_char(c))
            .map_or(content.len(), |offset| code_start + offset);
        let code = &content[code_start..code_end];
        if code.len() < 2 || code.len() > INVITE_CODE_MAX {
            continue;
        }
        if !codes.iter().any(|existing| existing == code) {
            codes.push(code.to_owned());
            if codes.len() == INVITE_LIMIT {
                break;
            }
        }
        consumed = code_end;
    }
    codes
}

fn invite_url_start(content: &str, host_start: usize, subdomains: bool) -> Option<usize> {
    let mut start = host_start;
    if subdomains {
        for subdomain in ["www.", "ptb.", "canary."] {
            if content[..start].ends_with(subdomain) {
                start -= subdomain.len();
                break;
            }
        }
    }
    for scheme in ["https://", "http://"] {
        if let Some(before) = content[..start].strip_suffix(scheme) {
            return Some(before.len());
        }
    }
    Some(start)
}

fn invite_link_boundary(content: &str, start: usize) -> bool {
    start == 0
        || !content[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '@'))
}

fn is_invite_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Invite {
    #[serde(default)]
    pub code: String,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild: Option<InviteGuild>,
    #[serde(default, deserialize_with = "default_value")]
    pub profile: Option<InviteProfile>,
    #[serde(default, deserialize_with = "default_value")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub approximate_member_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub approximate_presence_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InviteGuild {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub splash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InviteProfile {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub icon_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub custom_banner_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub member_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub online_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "ids")]
    pub game_application_ids: Vec<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub tag: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub badge_hash: Option<String>,
}
