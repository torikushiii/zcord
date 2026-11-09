// https://docs.discord.food/topics/experiments
use super::Guild;
use super::serde_helpers::id_value;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const NOT_ELIGIBLE: i32 = -1;
const CONTROL: i32 = 0;
const POSITIONS: u32 = 10_000;
const IS_OVERRIDE: u32 = 1 << 0;
const USE_AS_ELIGIBILITY: u32 = 1 << 3;
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
const DAY_MS: u64 = 86_400_000;

const GUILD_IDS: u32 = murmur3(b"guild_ids");
const GUILD_ID_RANGE: u32 = murmur3(b"guild_id_range");
const GUILD_AGE_RANGE_DAYS: u32 = murmur3(b"guild_age_range_days");
const GUILD_MEMBER_COUNT_RANGE: u32 = murmur3(b"guild_member_count_range");
const GUILD_HAS_FEATURE: u32 = murmur3(b"guild_has_feature");
const GUILD_HUB_TYPES: u32 = murmur3(b"guild_hub_types");
const GUILD_HAS_VANITY_URL: u32 = murmur3(b"guild_has_vanity_url");
const GUILD_IN_RANGE_BY_HASH: u32 = murmur3(b"guild_in_range_by_hash");
const MIN_ID: u32 = murmur3(b"min_id");
const MAX_ID: u32 = murmur3(b"max_id");
const HASH_KEY: u32 = murmur3(b"hash_key");
const TARGET: u32 = murmur3(b"target");

pub const fn murmur3(data: &[u8]) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let blocks = data.len() / 4;
    let mut hash = 0u32;
    let mut index = 0;
    while index < blocks {
        let at = index * 4;
        let block = u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        hash ^= block.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        hash = hash
            .rotate_left(13)
            .wrapping_mul(5)
            .wrapping_add(0xe654_6b64);
        index += 1;
    }
    let tail = blocks * 4;
    let mut rest = 0u32;
    let remaining = data.len() - tail;
    if remaining == 3 {
        rest ^= (data[tail + 2] as u32) << 16;
    }
    if remaining >= 2 {
        rest ^= (data[tail + 1] as u32) << 8;
    }
    if remaining >= 1 {
        rest ^= data[tail] as u32;
        hash ^= rest.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    }
    hash ^= data.len() as u32;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}

#[derive(Clone, Copy)]
struct Range {
    min: Option<u64>,
    max: Option<u64>,
}

impl Range {
    fn contains(self, value: u64) -> bool {
        self.min.is_none_or(|min| value > min) && self.max.is_none_or(|max| value < max)
    }
}

#[derive(Clone)]
enum Filter {
    GuildIds(Vec<u64>),
    IdRange(Range),
    AgeDays(Range),
    MemberCount(Range),
    Features(Vec<String>),
    HubTypes(Vec<i64>),
    VanityUrl(bool),
    InHashRange { key: String, target: u64 },
}

impl Filter {
    fn parse(entry: &Value) -> Option<Self> {
        let [kind, data] = entry.as_array()?.as_slice() else {
            return None;
        };
        let pairs: Vec<(u64, &Value)> = data
            .as_array()?
            .iter()
            .filter_map(|pair| match pair.as_array()?.as_slice() {
                [key, value] => Some((key.as_u64()?, value)),
                _ => None,
            })
            .collect();
        let field = |key: u32| {
            pairs
                .iter()
                .find(|(field, _)| *field == u64::from(key))
                .map(|(_, value)| *value)
        };
        let first = pairs.first().map(|(_, value)| *value);
        let range = || Range {
            min: field(MIN_ID).and_then(id_value),
            max: field(MAX_ID).and_then(id_value),
        };
        let kind = u32::try_from(kind.as_u64()?).ok()?;
        Some(match kind {
            GUILD_IDS => Self::GuildIds(
                field(GUILD_IDS)?
                    .as_array()?
                    .iter()
                    .filter_map(id_value)
                    .collect(),
            ),
            GUILD_ID_RANGE => Self::IdRange(range()),
            GUILD_AGE_RANGE_DAYS => Self::AgeDays(range()),
            GUILD_MEMBER_COUNT_RANGE => Self::MemberCount(range()),
            GUILD_HAS_FEATURE => Self::Features(
                first?
                    .as_array()?
                    .iter()
                    .filter_map(|feature| feature.as_str().map(str::to_owned))
                    .collect(),
            ),
            GUILD_HUB_TYPES => Self::HubTypes(
                first?
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_i64)
                    .collect(),
            ),
            GUILD_HAS_VANITY_URL => Self::VanityUrl(first?.as_bool()?),
            GUILD_IN_RANGE_BY_HASH => Self::InHashRange {
                key: match field(HASH_KEY)? {
                    Value::String(key) => key.clone(),
                    key => key.to_string(),
                },
                target: field(TARGET).and_then(id_value).unwrap_or(0),
            },
            // Dropped filters never exclude a guild, so unknown kinds stay permissive.
            _ => return None,
        })
    }

    fn matches(&self, guild: &Guild) -> bool {
        match self {
            Self::GuildIds(ids) => ids.contains(&guild.id),
            Self::IdRange(range) => range.contains(guild.id),
            Self::AgeDays(range) => range.contains(age_days(guild.id)),
            Self::MemberCount(range) => guild
                .member_count
                .is_some_and(|count| range.contains(u64::from(count))),
            Self::Features(features) => features.iter().any(|feature| guild.has_feature(feature)),
            Self::HubTypes(types) => guild.hub_type().is_some_and(|kind| types.contains(&kind)),
            Self::VanityUrl(vanity) => *vanity == guild.vanity_url_code().is_some(),
            Self::InHashRange { key, target } => {
                let hash = u64::from(murmur3(format!("{key}:{}", guild.id).as_bytes()));
                hash * 2 % u64::from(POSITIONS) < *target
            }
        }
    }
}

fn age_days(id: u64) -> u64 {
    let created = (id >> 22) + DISCORD_EPOCH_MS;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    now.saturating_sub(created) / DAY_MS
}

#[derive(Clone)]
struct Population {
    buckets: Vec<(i32, Vec<(u32, u32)>)>,
    filters: Vec<Filter>,
}

impl Population {
    fn parse(value: &Value) -> Option<Self> {
        let [buckets, filters] = value.as_array()?.as_slice() else {
            return None;
        };
        let range = |range: &Value| {
            let start = u32::try_from(range.get("s")?.as_u64()?).ok()?;
            let end = u32::try_from(range.get("e")?.as_u64()?).ok()?;
            Some((start, end))
        };
        let bucket = |bucket: &Value| match bucket.as_array()?.as_slice() {
            [id, ranges] => Some((
                i32::try_from(id.as_i64()?).ok()?,
                ranges.as_array()?.iter().filter_map(range).collect(),
            )),
            _ => None,
        };
        Some(Self {
            buckets: buckets.as_array()?.iter().filter_map(bucket).collect(),
            filters: filters
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Filter::parse)
                .collect(),
        })
    }

    fn list(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Self::parse)
            .collect()
    }
}

fn assign(populations: &[Population], guild: &Guild, position: u32) -> Option<i32> {
    let population = populations.iter().find(|population| {
        population
            .filters
            .iter()
            .all(|filter| filter.matches(guild))
    })?;
    let bucket = population
        .buckets
        .iter()
        .find(|(_, ranges)| {
            ranges
                .iter()
                .any(|&(start, end)| (start..end).contains(&position))
        })
        .map_or(CONTROL, |(bucket, _)| *bucket);
    (bucket != NOT_ELIGIBLE).then_some(bucket)
}

#[derive(Clone)]
struct Rollout {
    hash_key: Option<String>,
    populations: Vec<Population>,
    overrides: HashMap<u64, i32>,
    overrides_formatted: Vec<Vec<Population>>,
    holdout: Option<(String, i32)>,
    aa_mode: bool,
}

impl Rollout {
    fn parse(value: &Value) -> Option<(u32, Self)> {
        let fields = value.as_array()?;
        let field = |index: usize| fields.get(index).unwrap_or(&Value::Null);
        let mut overrides = HashMap::new();
        for entry in field(4).as_array().into_iter().flatten() {
            let Some(bucket) = entry
                .get("b")
                .and_then(Value::as_i64)
                .and_then(|bucket| i32::try_from(bucket).ok())
            else {
                continue;
            };
            for id in entry
                .get("k")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = id_value(id) {
                    overrides.insert(id, bucket);
                }
            }
        }
        let holdout_bucket = field(7)
            .as_i64()
            .and_then(|bucket| i32::try_from(bucket).ok());
        Some((
            u32::try_from(field(0).as_u64()?).ok()?,
            Self {
                hash_key: field(1).as_str().map(str::to_owned),
                populations: Population::list(field(3)),
                overrides,
                overrides_formatted: field(5)
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(Population::list)
                    .collect(),
                holdout: field(6).as_str().map(str::to_owned).zip(holdout_bucket),
                aa_mode: field(8).as_u64() == Some(1),
            },
        ))
    }
}

#[derive(Clone, Copy)]
struct Assignment {
    variant: Option<i64>,
    flags: u32,
}

#[derive(Clone, Default)]
pub struct ApexAssignments(HashMap<u32, Assignment>);

impl ApexAssignments {
    fn parse(value: &Value) -> Self {
        let assignment = |entry: &Value| {
            let fields = entry.as_array()?;
            let hash = u32::try_from(fields.first()?.as_u64()?).ok()?;
            let flags = fields
                .get(2)
                .and_then(Value::as_u64)
                .and_then(|flags| u32::try_from(flags).ok())
                .unwrap_or(0);
            Some((
                hash,
                Assignment {
                    variant: fields.get(1).and_then(Value::as_i64),
                    flags,
                },
            ))
        };
        Self(
            value
                .get("assignments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(assignment)
                .collect(),
        )
    }

    fn get(&self, name: &str) -> Option<Assignment> {
        self.0.get(&murmur3(name.as_bytes())).copied()
    }
}

impl<'de> Deserialize<'de> for ApexAssignments {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&Value::deserialize(d)?))
    }
}

#[derive(Clone, Default)]
pub struct Experiments {
    rollouts: HashMap<u32, Rollout>,
    user: ApexAssignments,
}

impl Experiments {
    pub fn parse(guild_experiments: &Value, apex_experiments: &Value, user_id: u64) -> Self {
        let users = &apex_experiments["assignments"]["1"];
        let user = users
            .get(user_id.to_string())
            .or_else(|| users.as_object()?.values().next());
        Self {
            rollouts: guild_experiments
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Rollout::parse)
                .collect(),
            user: user.map(ApexAssignments::parse).unwrap_or_default(),
        }
    }

    pub fn guild_treatment(&self, name: &str, guild: &Guild) -> Option<i32> {
        let (bucket, aa_mode) = self.guild_bucket(name, guild)?;
        (!aa_mode).then_some(bucket)
    }

    fn guild_bucket(&self, name: &str, guild: &Guild) -> Option<(i32, bool)> {
        let rollout = self.rollouts.get(&murmur3(name.as_bytes()))?;
        if let Some(&bucket) = rollout.overrides.get(&guild.id) {
            return (bucket != NOT_ELIGIBLE).then_some((bucket, false));
        }
        let unit = format!(
            "{}:{}",
            rollout.hash_key.as_deref().unwrap_or(name),
            guild.id
        );
        let position = murmur3(unit.as_bytes()) % POSITIONS;
        if let Some(bucket) = rollout
            .overrides_formatted
            .iter()
            .find_map(|populations| assign(populations, guild, position))
        {
            return Some((bucket, false));
        }
        let bucket = assign(&rollout.populations, guild, position)?;
        if let Some((holdout, control)) = &rollout.holdout
            && holdout != name
            && self
                .guild_bucket(holdout, guild)
                .is_some_and(|(bucket, _)| bucket == *control)
        {
            return None;
        }
        Some((bucket, rollout.aa_mode))
    }

    pub fn user_variant(&self, name: &str) -> Option<i64> {
        let assignment = self.user.get(name)?;
        assignment
            .variant
            .filter(|_| assignment.flags & USE_AS_ELIGIBILITY == 0)
    }

    pub fn guild_variant(&self, name: &str, guild: &Guild) -> Option<i64> {
        apex_variant(
            self.user.get(name),
            guild
                .experiments
                .as_ref()
                .and_then(|experiments| experiments.get(name))
                .filter(|assignment| assignment.variant.is_some()),
        )
    }
}

fn apex_variant(user: Option<Assignment>, guild: Option<Assignment>) -> Option<i64> {
    let flagged = |assignment: Option<Assignment>, flag: u32| {
        assignment.filter(|assignment| assignment.flags & flag != 0)
    };
    let assignment = flagged(user, IS_OVERRIDE)
        .or_else(|| flagged(guild, IS_OVERRIDE))
        .or_else(|| flagged(user, USE_AS_ELIGIBILITY).and(guild))?;
    assignment
        .variant
        .filter(|_| assignment.flags & USE_AS_ELIGIBILITY == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignment(variant: i64, flags: u32) -> Option<Assignment> {
        Some(Assignment {
            variant: Some(variant),
            flags,
        })
    }

    #[test]
    fn range_filter_bounds_are_exclusive() {
        let range = Range {
            min: Some(1000),
            max: Some(2000),
        };
        assert!(!range.contains(1000));
        assert!(range.contains(1001));
        assert!(range.contains(1999));
        assert!(!range.contains(2000));
    }

    #[test]
    fn apex_guild_override_applies_without_a_user_assignment() {
        assert_eq!(apex_variant(None, assignment(3, IS_OVERRIDE)), Some(3));
        assert_eq!(
            apex_variant(assignment(1, 0), assignment(3, IS_OVERRIDE)),
            Some(3)
        );
    }

    #[test]
    fn apex_user_override_beats_guild_override() {
        assert_eq!(
            apex_variant(assignment(2, IS_OVERRIDE), assignment(3, IS_OVERRIDE)),
            Some(2)
        );
    }

    #[test]
    fn apex_guild_assignment_needs_the_user_eligibility_gate() {
        assert_eq!(apex_variant(None, assignment(3, 0)), None);
        assert_eq!(apex_variant(assignment(1, 0), assignment(3, 0)), None);
        assert_eq!(
            apex_variant(assignment(0, USE_AS_ELIGIBILITY), assignment(3, 0)),
            Some(3)
        );
    }

    #[test]
    fn apex_eligibility_only_results_are_discarded() {
        assert_eq!(
            apex_variant(assignment(2, IS_OVERRIDE | USE_AS_ELIGIBILITY), None),
            None
        );
        assert_eq!(
            apex_variant(None, assignment(3, IS_OVERRIDE | USE_AS_ELIGIBILITY)),
            None
        );
    }
}
