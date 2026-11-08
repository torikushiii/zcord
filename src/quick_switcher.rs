use std::cmp::Ordering;
use std::collections::VecDeque;

use crate::store::{Channel, ChannelKind, Guild};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Channel { guild_id: u64, channel_id: u64 },
    Guild(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Text,
    Announcement,
    Forum,
    Thread,
    Guild,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub target: Target,
    pub kind: ItemKind,
    pub name: String,
    pub context: Option<String>,
    pub guild_name: Option<String>,
    pub unread: bool,
    pub mentions: u32,
    pub matched: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    Previous,
    Mentions,
    Results,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub kind: SectionKind,
    pub items: Vec<Item>,
}

#[derive(Default)]
pub struct Recent(VecDeque<u64>);

impl Recent {
    pub fn visit(&mut self, channel_id: u64) {
        self.0.retain(|id| *id != channel_id);
        self.0.push_front(channel_id);
        self.0.truncate(50);
    }

    fn position(&self, channel_id: u64) -> Option<usize> {
        self.0.iter().position(|id| *id == channel_id)
    }
}

pub fn search(
    guilds: &[Guild],
    query: &str,
    recent: &Recent,
    current_guild_id: u64,
    current_channel_id: u64,
    limit: usize,
) -> Vec<Section> {
    let trimmed = query.trim();
    let (sigil, term) = match trimmed.chars().next() {
        Some(sigil @ ('#' | '*' | '@' | '!' | '$')) => {
            (Some(sigil), trimmed[sigil.len_utf8()..].trim())
        }
        _ => (None, trimmed),
    };

    if matches!(sigil, Some('@' | '!' | '$')) {
        return Vec::new();
    }

    if term.is_empty() {
        if sigil == Some('*') {
            let items: Vec<Item> = guilds.iter().take(limit).map(guild_item).collect();
            return section(SectionKind::Results, items);
        }
        return empty_term_sections(guilds, recent, current_channel_id, limit);
    }

    let want_channels = matches!(sigil, None | Some('#'));
    let want_guilds = matches!(sigil, None | Some('*'));

    if limit == 0 {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    if want_channels {
        candidates.extend(channel_candidates(guilds));
        candidates.extend(thread_candidates(guilds));
    }
    if want_guilds {
        candidates.extend(guild_candidates(guilds));
    }

    let mut ranked = Vec::new();
    for mut candidate in candidates {
        let Some(found) = name_match(&candidate.item.name, term) else {
            continue;
        };
        candidate.item.matched = found.matched.clone();
        ranked.push(Ranked { candidate, found });
    }

    ranked.sort_by(|a, b| compare_ranked(a, b, recent, current_guild_id));
    let items = ranked
        .into_iter()
        .take(limit)
        .map(|ranked| ranked.candidate.item)
        .collect();
    section(SectionKind::Results, items)
}

fn section(kind: SectionKind, items: Vec<Item>) -> Vec<Section> {
    if items.is_empty() {
        Vec::new()
    } else {
        vec![Section { kind, items }]
    }
}

fn empty_term_sections(
    guilds: &[Guild],
    recent: &Recent,
    current_channel_id: u64,
    limit: usize,
) -> Vec<Section> {
    let mut sections = Vec::new();
    if limit == 0 {
        return sections;
    }

    let candidates = channel_candidates(guilds)
        .into_iter()
        .chain(thread_candidates(guilds))
        .collect::<Vec<_>>();

    let mut previous = Vec::new();
    for channel_id in &recent.0 {
        if previous.len() >= limit {
            break;
        }
        if *channel_id == current_channel_id {
            continue;
        }
        let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.channel_id() == Some(*channel_id))
        else {
            continue;
        };
        if previous
            .iter()
            .any(|item: &Item| item.target == candidate.item.target)
        {
            continue;
        }
        previous.push(candidate.item.clone());
    }

    let previous_targets = previous.iter().map(|item| item.target).collect::<Vec<_>>();
    let remaining = limit - previous.len();
    sections.extend(section(SectionKind::Previous, previous));
    if remaining == 0 {
        return sections;
    }

    let mut mentioned = candidates
        .iter()
        .filter(|candidate| {
            candidate.item.mentions > 0 && !previous_targets.contains(&candidate.item.target)
        })
        .collect::<Vec<_>>();
    mentioned.sort_by(|a, b| {
        b.item
            .mentions
            .cmp(&a.item.mentions)
            .then_with(|| a.guild_index.cmp(&b.guild_index))
            .then_with(|| a.item.name.cmp(&b.item.name))
            .then_with(|| target_id(&a.item.target).cmp(&target_id(&b.item.target)))
    });
    let items = mentioned
        .into_iter()
        .take(remaining)
        .map(|candidate| candidate.item.clone())
        .collect();
    sections.extend(section(SectionKind::Mentions, items));
    sections
}

struct Candidate {
    item: Item,
    guild_index: usize,
}

impl Candidate {
    fn channel_id(&self) -> Option<u64> {
        match self.item.target {
            Target::Channel { channel_id, .. } => Some(channel_id),
            Target::Guild(_) => None,
        }
    }

    fn guild_id(&self) -> u64 {
        match self.item.target {
            Target::Channel { guild_id, .. } => guild_id,
            Target::Guild(guild_id) => guild_id,
        }
    }
}

fn channel_candidates(guilds: &[Guild]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (guild_index, guild) in guilds.iter().enumerate() {
        for category in &guild.categories {
            for channel in &category.channels {
                let Some(kind) = item_kind(channel.kind) else {
                    continue;
                };
                let context = if category.id == 0 && category.name.is_empty() {
                    None
                } else {
                    Some(category.name.clone())
                };
                candidates.push(Candidate {
                    item: channel_item(guild, channel, kind, context),
                    guild_index,
                });
            }
        }
    }
    candidates
}

fn thread_candidates(guilds: &[Guild]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (guild_index, guild) in guilds.iter().enumerate() {
        for thread in &guild.threads {
            if !thread.kind.is_selectable() {
                continue;
            }
            let context = thread
                .parent_id
                .and_then(|parent_id| guild.channel(parent_id))
                .map(|parent| parent.name.clone());
            candidates.push(Candidate {
                item: channel_item(guild, thread, ItemKind::Thread, context),
                guild_index,
            });
        }
    }
    candidates
}

fn guild_candidates(guilds: &[Guild]) -> Vec<Candidate> {
    guilds
        .iter()
        .enumerate()
        .map(|(guild_index, guild)| Candidate {
            item: guild_item(guild),
            guild_index,
        })
        .collect()
}

fn channel_item(guild: &Guild, channel: &Channel, kind: ItemKind, context: Option<String>) -> Item {
    Item {
        target: Target::Channel {
            guild_id: guild.id,
            channel_id: channel.id,
        },
        kind,
        name: channel.name.clone(),
        context,
        guild_name: Some(guild.name.clone()),
        unread: channel.unread,
        mentions: channel.mentions,
        matched: Vec::new(),
    }
}

fn guild_item(guild: &Guild) -> Item {
    Item {
        target: Target::Guild(guild.id),
        kind: ItemKind::Guild,
        name: guild.name.clone(),
        context: None,
        guild_name: None,
        unread: guild.unread,
        mentions: guild.mentions,
        matched: Vec::new(),
    }
}

fn item_kind(kind: ChannelKind) -> Option<ItemKind> {
    match kind {
        ChannelKind::Text => Some(ItemKind::Text),
        ChannelKind::Announcement => Some(ItemKind::Announcement),
        ChannelKind::Forum => Some(ItemKind::Forum),
        ChannelKind::Voice => None,
    }
}

fn target_id(target: &Target) -> u64 {
    match target {
        Target::Channel { channel_id, .. } => *channel_id,
        Target::Guild(guild_id) => *guild_id,
    }
}

struct Ranked {
    candidate: Candidate,
    found: Match,
}

fn compare_ranked(a: &Ranked, b: &Ranked, recent: &Recent, current_guild_id: u64) -> Ordering {
    a.found
        .tier
        .cmp(&b.found.tier)
        .then_with(|| {
            if a.found.tier == 4 {
                a.found.span.cmp(&b.found.span)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| recency(&a.candidate, recent).cmp(&recency(&b.candidate, recent)))
        .then_with(|| {
            current_guild(&a.candidate, current_guild_id)
                .cmp(&current_guild(&b.candidate, current_guild_id))
        })
        .then_with(|| b.candidate.item.mentions.cmp(&a.candidate.item.mentions))
        .then_with(|| b.candidate.item.unread.cmp(&a.candidate.item.unread))
        .then_with(|| {
            a.candidate
                .item
                .name
                .chars()
                .count()
                .cmp(&b.candidate.item.name.chars().count())
        })
        .then_with(|| a.candidate.item.name.cmp(&b.candidate.item.name))
        .then_with(|| target_id(&a.candidate.item.target).cmp(&target_id(&b.candidate.item.target)))
}

fn recency(candidate: &Candidate, recent: &Recent) -> usize {
    candidate
        .channel_id()
        .and_then(|channel_id| recent.position(channel_id))
        .unwrap_or(usize::MAX)
}

fn current_guild(candidate: &Candidate, current_guild_id: u64) -> u8 {
    if candidate.guild_id() == current_guild_id {
        0
    } else {
        1
    }
}

struct Match {
    tier: u8,
    span: usize,
    matched: Vec<usize>,
}

fn name_match(name: &str, term: &str) -> Option<Match> {
    let name = name.chars().collect::<Vec<_>>();
    let term = term.chars().collect::<Vec<_>>();
    if name.is_empty() || term.is_empty() {
        return None;
    }

    if name.len() == term.len()
        && name
            .iter()
            .zip(&term)
            .all(|(name, term)| chars_eq(*name, *term))
    {
        return Some(Match {
            tier: 0,
            span: 0,
            matched: (0..name.len()).collect(),
        });
    }

    if name.len() >= term.len()
        && name
            .iter()
            .zip(&term)
            .all(|(name, term)| chars_eq(*name, *term))
    {
        return Some(Match {
            tier: 1,
            span: 0,
            matched: (0..term.len()).collect(),
        });
    }

    if name.len() >= term.len() {
        let mut contains = None;
        let mut word = None;
        for start in 0..=name.len() - term.len() {
            let contiguous = name[start..start + term.len()]
                .iter()
                .zip(&term)
                .all(|(name, term)| chars_eq(*name, *term));
            if !contiguous {
                continue;
            }
            if contains.is_none() {
                contains = Some(start);
            }
            if start == 0 || !name[start - 1].is_alphanumeric() {
                word = Some(start);
                break;
            }
        }
        if let Some(start) = word {
            return Some(Match {
                tier: 2,
                span: 0,
                matched: (start..start + term.len()).collect(),
            });
        }
        if let Some(start) = contains {
            return Some(Match {
                tier: 3,
                span: 0,
                matched: (start..start + term.len()).collect(),
            });
        }
    }

    let mut best: Option<(usize, usize, Vec<usize>)> = None;
    for start in 0..name.len() {
        if !chars_eq(name[start], term[0]) {
            continue;
        }
        let mut matched = vec![start];
        let mut term_index = 1;
        let mut name_index = start + 1;
        while term_index < term.len() && name_index < name.len() {
            if chars_eq(name[name_index], term[term_index]) {
                matched.push(name_index);
                term_index += 1;
            }
            name_index += 1;
        }
        if term_index < term.len() {
            continue;
        }
        let span = matched[matched.len() - 1] - start;
        let tighter = match &best {
            None => true,
            Some((best_span, best_start, _)) => {
                span < *best_span || (span == *best_span && start < *best_start)
            }
        };
        if tighter {
            best = Some((span, start, matched));
        }
    }

    best.map(|(span, _, matched)| Match {
        tier: 4,
        span,
        matched,
    })
}

fn chars_eq(a: char, b: char) -> bool {
    if is_separator(a) && is_separator(b) {
        return true;
    }
    a.to_lowercase().eq(b.to_lowercase())
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '-' | '_')
}
