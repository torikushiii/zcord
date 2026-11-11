mod filters;
mod popover;
mod results;
mod search_box;

pub(crate) use filters::filters_dialog;
pub(crate) use popover::popover;
pub(crate) use results::render_results;
pub(crate) use search_box::search_box;

use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use gpui::{prelude::*, *};
use gpui_component::input::{Backspace, Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;

use super::chat::{self, render_search_message};
use super::{DiscordApp, messages::message_body::label, titlebar};
use crate::discord::{SearchOutcome, SearchQuery, SearchScope, SearchSort, model};
use crate::settings::{SavedFilter, SavedFilterValue, SavedSearch};
use crate::store::Message;
use crate::text::search_query::{self, HAS_OPTIONS};
use crate::theme::{self, WithoutTextSelection, rpx};

pub(crate) const RESULTS_WIDTH: f32 = 419.;
const PAGE: usize = 25;
const MAX_OFFSET: usize = 9975;
const INDEX_RETRIES: u8 = 5;
const USER_SUGGESTIONS: usize = 8;
const CHANNEL_SUGGESTIONS: usize = 10;
const WORD_SUGGESTIONS: usize = 3;
const HISTORY_LIMIT: usize = 5;
const MEMBER_LIMIT: u32 = USER_SUGGESTIONS as u32;
const MEMBER_QUERY_DELAY: Duration = Duration::from_millis(250);
const AUTHOR_TYPES: [&str; 3] = ["user", "bot", "webhook"];
const CHIP_KEYS: [&str; 9] = [
    "from", "mentions", "in", "has", "before", "after", "during", "author", "pinned",
];

pub(crate) struct MessageSearch {
    scope: SearchScope,
    input: Entity<InputState>,
    popover: bool,
    highlighted: Option<usize>,
    chips: Vec<Chip>,
    results: Option<Results>,
    sort_menu: bool,
    sort_closed: Option<Instant>,
    request: u64,
    member_query: Option<Task<()>>,
    filters: Option<filters::FiltersDialog>,
    _events: Subscription,
}

#[derive(Clone, PartialEq)]
struct Chip {
    key: &'static str,
    label: String,
    value: ChipValue,
}

#[derive(Clone, Copy, PartialEq)]
enum ChipValue {
    User(u64),
    Channel(u64),
    Has(&'static str),
    Date(NaiveDate),
    Author(&'static str),
    Pinned(bool),
}

impl Chip {
    fn saved(&self) -> SavedFilter {
        SavedFilter {
            key: self.key.to_owned(),
            label: self.label.clone(),
            value: match self.value {
                ChipValue::User(id) => SavedFilterValue::User(id),
                ChipValue::Channel(id) => SavedFilterValue::Channel(id),
                ChipValue::Has(api) => SavedFilterValue::Has(api.to_owned()),
                ChipValue::Date(date) => {
                    SavedFilterValue::Date(date.format("%Y-%m-%d").to_string())
                }
                ChipValue::Author(kind) => SavedFilterValue::Author(kind.to_owned()),
                ChipValue::Pinned(pinned) => SavedFilterValue::Pinned(pinned),
            },
        }
    }

    fn from_saved(saved: &SavedFilter) -> Option<Self> {
        let key = CHIP_KEYS.into_iter().find(|key| *key == saved.key)?;
        let value = match &saved.value {
            SavedFilterValue::User(id) => ChipValue::User(*id),
            SavedFilterValue::Channel(id) => ChipValue::Channel(*id),
            SavedFilterValue::Has(api) => ChipValue::Has(
                HAS_OPTIONS
                    .iter()
                    .find(|(_, known)| known == api)
                    .map(|(_, known)| *known)?,
            ),
            SavedFilterValue::Date(date) => {
                ChipValue::Date(NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?)
            }
            SavedFilterValue::Author(kind) => {
                ChipValue::Author(AUTHOR_TYPES.into_iter().find(|known| known == kind)?)
            }
            SavedFilterValue::Pinned(pinned) => ChipValue::Pinned(*pinned),
        };
        Some(Self {
            key,
            label: saved.label.clone(),
            value,
        })
    }
}

struct Results {
    query: SearchQuery,
    terms: Rc<[String]>,
    messages: Vec<(u64, Message)>,
    channels: Vec<model::Channel>,
    total: u64,
    status: Status,
}

enum Status {
    Loading,
    Indexing,
    Ready,
    Failed(String),
}

#[derive(Clone)]
enum Row {
    Run,
    AddFilters,
    Filter(&'static Filter),
    MoreFilters,
    History(usize),
    User {
        key: &'static str,
        id: u64,
        name: String,
        username: String,
        suggested: bool,
    },
    Channel {
        id: u64,
        name: String,
        suggested: bool,
    },
    Has {
        name: &'static str,
        api: &'static str,
    },
}

impl Row {
    fn group(&self) -> Option<&'static str> {
        match self {
            Row::Run | Row::AddFilters => None,
            Row::Filter(_) | Row::MoreFilters => Some("Filters"),
            Row::History(_) => Some("History"),
            Row::User { key: "from", .. } => Some("From User"),
            Row::User { .. } => Some("Mentions User"),
            Row::Channel { .. } => Some("In Channel"),
            Row::Has { .. } => Some("Message Contains"),
        }
    }
}

struct Filter {
    key: &'static str,
    icon: &'static str,
    title: &'static str,
    hint: &'static str,
}

const FILTERS: [Filter; 4] = [
    Filter {
        key: "from",
        icon: "user",
        title: "From a specific user",
        hint: "user",
    },
    Filter {
        key: "in",
        icon: "hash",
        title: "Sent in a specific channel",
        hint: "channel",
    },
    Filter {
        key: "has",
        icon: "paperclip",
        title: "Includes a specific type of data",
        hint: "link, embed or file",
    },
    Filter {
        key: "mentions",
        icon: "at-sign",
        title: "Mentions a specific user",
        hint: "user",
    },
];

fn filter_key(key: &str) -> Option<&'static str> {
    FILTERS
        .iter()
        .map(|filter| filter.key)
        .find(|known| known.eq_ignore_ascii_case(key))
}

fn last_token_start(text: &str) -> usize {
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        match character {
            '"' => quoted = !quoted,
            character if character.is_whitespace() && !quoted => {
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    start
}

fn trailing_word(text: &str) -> Option<&str> {
    if text.chars().last().is_none_or(char::is_whitespace) {
        return None;
    }
    let word = &text[last_token_start(text)..];
    (!word.is_empty() && !word.contains(':') && !word.starts_with('"')).then_some(word)
}

fn history_key(scope: SearchScope) -> String {
    match scope {
        SearchScope::Guild(id) => format!("guild:{id}"),
        SearchScope::Channel(id) => format!("channel:{id}"),
        SearchScope::Dms => "dms".to_owned(),
    }
}

fn filter_count(search: &MessageSearch, cx: &App) -> usize {
    let typed = search_query::parse(&search.text(cx)).map_or(0, |parsed| {
        parsed.from.len()
            + parsed.mentions.len()
            + parsed.channels.len()
            + parsed.has.len()
            + usize::from(parsed.min_id.is_some() || parsed.max_id.is_some())
            + usize::from(parsed.pinned.is_some())
    });
    search.chips.len() + typed
}

impl MessageSearch {
    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn chosen_index(&self, cx: &App) -> Option<usize> {
        self.highlighted.or_else(|| {
            search_query::active_filter(&self.text(cx))
                .is_some()
                .then_some(0)
        })
    }
}

impl DiscordApp {
    fn search_scope(&self) -> Option<SearchScope> {
        if self.showing_home_page() {
            None
        } else if self.is_home() {
            Some(if crate::settings::display().is_cross_dm_search_enabled {
                SearchScope::Dms
            } else {
                SearchScope::Channel(self.selected_channel_id)
            })
        } else {
            Some(SearchScope::Guild(self.selected_guild_id))
        }
    }

    fn active_search(&self) -> Option<&MessageSearch> {
        self.menus
            .message_search
            .as_ref()
            .filter(|search| Some(search.scope) == self.search_scope())
    }

    fn active_search_mut(&mut self) -> Option<&mut MessageSearch> {
        let scope = self.search_scope();
        self.menus
            .message_search
            .as_mut()
            .filter(|search| Some(search.scope) == scope)
    }

    pub(super) fn sync_search(&mut self) {
        if self.menus.message_search.is_some() && self.active_search().is_none() {
            self.menus.message_search = None;
        }
    }

    pub(crate) fn search_results_open(&self) -> bool {
        self.active_search()
            .is_some_and(|search| search.results.is_some())
    }

    pub(crate) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.search_scope() else {
            return;
        };
        if let Some(search) = self.active_search_mut() {
            search.popover = true;
            search.highlighted = None;
            let input = search.input.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx));
        let events = cx.subscribe_in(&input, window, |app, _, event, window, cx| match event {
            InputEvent::Change => app.search_changed(cx),
            InputEvent::PressEnter { .. } => app.search_enter(window, cx),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.menus.message_search = Some(MessageSearch {
            scope,
            input,
            popover: true,
            highlighted: None,
            chips: Vec::new(),
            results: None,
            sort_menu: false,
            sort_closed: None,
            request: 0,
            member_query: None,
            filters: None,
            _events: events,
        });
        cx.notify();
    }

    pub(crate) fn find_in_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(window, cx);
        if !matches!(self.search_scope(), Some(SearchScope::Guild(_)))
            || self.selected_forum().is_some()
        {
            return;
        }
        let Some((id, name)) = self
            .channel()
            .map(|channel| (channel.id, channel.name.clone()))
        else {
            return;
        };
        let Some(search) = self.active_search_mut() else {
            return;
        };
        if !search.chips.is_empty() || !search.text(cx).is_empty() {
            return;
        }
        search.chips.push(Chip {
            key: "in",
            label: name,
            value: ChipValue::Channel(id),
        });
        cx.notify();
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(search) = self.active_search_mut() else {
            return false;
        };
        if search.filters.take().is_some() {
            cx.notify();
            return true;
        }
        if search.popover {
            search.popover = false;
            cx.notify();
            return true;
        }
        if !search.text(cx).is_empty() || !search.chips.is_empty() {
            self.clear_search(window, cx);
            return true;
        }
        if search.results.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.active_search_mut() {
            search.results = None;
            search.popover = false;
            search.chips.clear();
            let input = search.input.clone();
            input.update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        }
    }

    fn remove_last_chip(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(search) = self.active_search_mut() else {
            return false;
        };
        if !search.text(cx).is_empty() || search.chips.pop().is_none() {
            return false;
        }
        search.popover = true;
        cx.notify();
        true
    }

    fn search_changed(&mut self, cx: &mut Context<Self>) {
        let guild_id = match self.active_search().map(|search| search.scope) {
            Some(SearchScope::Guild(guild_id)) => Some(guild_id),
            Some(SearchScope::Channel(_) | SearchScope::Dms) => None,
            None => return,
        };
        let session = self.session.clone().filter(|_| self.live);
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let text = search.text(cx);
        search.popover = true;
        search.highlighted = None;
        let partial = search_query::active_filter(&text)
            .filter(|(key, partial)| {
                !partial.is_empty()
                    && (key.eq_ignore_ascii_case("from") || key.eq_ignore_ascii_case("mentions"))
            })
            .map(|(_, partial)| partial.to_owned())
            .or_else(|| trailing_word(&text).map(str::to_owned));
        search.member_query = None;
        if let (Some(guild_id), Some(session), Some(partial)) = (guild_id, session, partial) {
            search.member_query = Some(cx.spawn(async move |_, cx| {
                cx.background_executor().timer(MEMBER_QUERY_DELAY).await;
                session.query_members(guild_id, &partial, MEMBER_LIMIT);
            }));
        }
        cx.notify();
    }

    fn search_history(&self, scope: SearchScope, cx: &App) -> Vec<SavedSearch> {
        crate::settings::get(cx)
            .search_history
            .get(&history_key(scope))
            .cloned()
            .unwrap_or_default()
    }

    fn remember_search(&self, scope: SearchScope, entry: SavedSearch, cx: &mut App) {
        crate::settings::update(cx, |settings| {
            let history = settings
                .search_history
                .entry(history_key(scope))
                .or_default();
            history.retain(|saved| *saved != entry);
            history.insert(0, entry);
            history.truncate(HISTORY_LIMIT);
        });
    }

    pub(crate) fn clear_search_history(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.active_search().map(|search| search.scope) else {
            return;
        };
        crate::settings::update(cx, |settings| {
            settings.search_history.remove(&history_key(scope));
        });
        cx.notify();
    }

    fn user_rows(
        &self,
        guild_id: Option<u64>,
        key: &'static str,
        partial: &str,
        limit: usize,
        suggested: bool,
    ) -> Vec<Row> {
        self.store
            .search_users(guild_id, self.selected_channel_id, partial, limit)
            .into_iter()
            .filter_map(|id| {
                let user = self.store.user(id)?;
                let name = guild_id
                    .and_then(|guild_id| self.store.member_name(guild_id, id))
                    .unwrap_or(&user.name)
                    .to_owned();
                Some(Row::User {
                    key,
                    id,
                    name,
                    username: user.username.clone(),
                    suggested,
                })
            })
            .collect()
    }

    fn channel_rows(&self, partial: &str, limit: usize, suggested: bool) -> Vec<Row> {
        self.guild()
            .into_iter()
            .flat_map(|guild| guild.channels())
            .filter(|channel| channel.kind.is_selectable())
            .filter(|channel| channel.name.to_lowercase().contains(partial))
            .take(limit)
            .map(|channel| Row::Channel {
                id: channel.id,
                name: channel.name.clone(),
                suggested,
            })
            .collect()
    }

    fn search_rows(&self, cx: &App) -> Vec<Row> {
        let Some(search) = self.active_search() else {
            return Vec::new();
        };
        let guild_id = match search.scope {
            SearchScope::Guild(guild_id) => Some(guild_id),
            SearchScope::Channel(_) | SearchScope::Dms => None,
        };
        let text = search.text(cx);
        if let Some((key, partial)) = search_query::active_filter(&text) {
            let partial = partial.trim_start_matches(['@', '#', '"']).to_lowercase();
            return match filter_key(key) {
                Some("from" | "mentions") if partial.is_empty() => Vec::new(),
                Some(key @ ("from" | "mentions")) => {
                    self.user_rows(guild_id, key, &partial, USER_SUGGESTIONS, false)
                }
                Some("in") => self.channel_rows(&partial, CHANNEL_SUGGESTIONS, false),
                Some("has") => HAS_OPTIONS
                    .iter()
                    .filter(|(name, _)| name.starts_with(&partial))
                    .map(|(name, api)| Row::Has { name, api })
                    .collect(),
                _ => Vec::new(),
            };
        }
        let mut rows = Vec::new();
        let has_query = !text.trim().is_empty() || !search.chips.is_empty();
        if has_query {
            rows.extend([Row::Run, Row::AddFilters]);
        }
        match trailing_word(&text) {
            Some(word) => {
                let word = word.to_lowercase();
                rows.extend(self.user_rows(guild_id, "from", &word, WORD_SUGGESTIONS, true));
                if guild_id.is_some() {
                    rows.extend(self.channel_rows(&word, WORD_SUGGESTIONS, true));
                }
                rows.extend(self.user_rows(guild_id, "mentions", &word, WORD_SUGGESTIONS, true));
            }
            None => {
                rows.extend(
                    FILTERS
                        .iter()
                        .filter(|filter| filter.key != "in" || guild_id.is_some())
                        .map(Row::Filter),
                );
                rows.push(Row::MoreFilters);
                if !has_query {
                    let saved = self.search_history(search.scope, cx).len();
                    rows.extend((0..saved).map(Row::History));
                }
            }
        }
        rows
    }

    pub(super) fn move_search(&mut self, step: isize, cx: &mut Context<Self>) -> bool {
        if !self.active_search().is_some_and(|search| search.popover) {
            return false;
        }
        let count = self.search_rows(cx).len();
        let Some(search) = self.active_search_mut() else {
            return false;
        };
        if count == 0 {
            return true;
        }
        let next = match search.chosen_index(cx) {
            Some(current) => (current as isize + step).rem_euclid(count as isize) as usize,
            None if step > 0 => 0,
            None => count - 1,
        };
        search.highlighted = Some(next);
        cx.notify();
        true
    }

    fn chosen_row(&self, cx: &App) -> Option<Row> {
        let search = self.active_search().filter(|search| search.popover)?;
        let index = search.chosen_index(cx)?;
        self.search_rows(cx).into_iter().nth(index)
    }

    pub(super) fn accept_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.chosen_row(cx) else {
            return false;
        };
        self.apply_search_row(row, window, cx);
        true
    }

    fn apply_search_row(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            Row::Run => return self.run_search(cx),
            Row::AddFilters | Row::MoreFilters => return self.open_search_filters(window, cx),
            Row::History(index) => {
                let Some(scope) = self.active_search().map(|search| search.scope) else {
                    return;
                };
                if let Some(entry) = self.search_history(scope, cx).into_iter().nth(index) {
                    self.load_search(entry, window, cx);
                }
                return;
            }
            _ => {}
        }
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let text = search.text(cx);
        let start = last_token_start(&text);
        let chip = match row {
            Row::Filter(filter) => {
                let token = &text[start..];
                let completes_typed_key = !token.is_empty()
                    && filter
                        .key
                        .get(..token.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(token));
                let spaced = if completes_typed_key {
                    text[..start].to_owned()
                } else if text.is_empty() || text.ends_with(char::is_whitespace) {
                    text.clone()
                } else {
                    format!("{text} ")
                };
                let input = search.input.clone();
                search.highlighted = None;
                search.popover = true;
                input.update(cx, |input, cx| {
                    input.set_value(format!("{spaced}{}:", filter.key), window, cx);
                    input.focus(window, cx);
                });
                cx.notify();
                return;
            }
            Row::User {
                key, id, username, ..
            } => Chip {
                key,
                label: username,
                value: ChipValue::User(id),
            },
            Row::Channel { id, name, .. } => Chip {
                key: "in",
                label: name,
                value: ChipValue::Channel(id),
            },
            Row::Has { name, api } => Chip {
                key: "has",
                label: name.to_owned(),
                value: ChipValue::Has(api),
            },
            Row::Run | Row::AddFilters | Row::MoreFilters | Row::History(_) => return,
        };
        search.chips.push(chip);
        search.highlighted = None;
        let input = search.input.clone();
        let base = text[..start].trim_end().to_owned();
        input.update(cx, |input, cx| {
            input.set_value(base, window, cx);
            input.focus(window, cx);
        });
        self.run_search(cx);
    }

    fn load_search(&mut self, entry: SavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.active_search_mut() else {
            return;
        };
        search.chips = entry.filters.iter().filter_map(Chip::from_saved).collect();
        search.highlighted = None;
        let input = search.input.clone();
        input.update(cx, |input, cx| {
            input.set_value(entry.text, window, cx);
            input.focus(window, cx);
        });
        self.run_search(cx);
    }

    fn search_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accept_search(window, cx) {
            self.run_search(cx);
        }
    }

    fn resolve_search(&self, search: &MessageSearch, text: &str) -> Result<SearchQuery, String> {
        let parsed = search_query::parse(text)?;
        let guild_id = match search.scope {
            SearchScope::Guild(guild_id) => Some(guild_id),
            SearchScope::Channel(_) | SearchScope::Dms => None,
        };
        let user = |name: &String| {
            self.store
                .find_search_user(guild_id, self.selected_channel_id, name)
                .ok_or_else(|| format!("No user named {name}."))
        };
        let channel = |name: &String| {
            self.guild()
                .and_then(|guild| {
                    guild
                        .channels()
                        .find(|channel| channel.name.eq_ignore_ascii_case(name))
                })
                .map(|channel| channel.id)
                .ok_or_else(|| format!("No channel named {name}."))
        };
        let mut author_ids: Vec<u64> = parsed.from.iter().map(user).collect::<Result<_, _>>()?;
        let mut mentions: Vec<u64> = parsed.mentions.iter().map(user).collect::<Result<_, _>>()?;
        let mut channel_ids: Vec<u64> = parsed
            .channels
            .iter()
            .map(channel)
            .collect::<Result<_, _>>()?;
        let mut has = parsed.has;
        let (mut min_id, mut max_id, mut pinned) = (parsed.min_id, parsed.max_id, parsed.pinned);
        let mut author_types = Vec::new();
        for chip in &search.chips {
            match chip.value {
                ChipValue::User(id) if chip.key == "from" => author_ids.push(id),
                ChipValue::User(id) => mentions.push(id),
                ChipValue::Channel(id) => channel_ids.push(id),
                ChipValue::Has(api) => has.push(api.to_owned()),
                ChipValue::Date(date) => {
                    let (after, before) = search_query::date_bounds(chip.key, date)?;
                    min_id = min_id.max(after);
                    max_id = match (max_id, before) {
                        (Some(current), Some(before)) => Some(current.min(before)),
                        (current, before) => current.or(before),
                    };
                }
                ChipValue::Author(kind) => author_types.push(kind.to_owned()),
                ChipValue::Pinned(value) => pinned = Some(value),
            }
        }
        Ok(SearchQuery {
            author_ids,
            mentions,
            channel_ids,
            content: parsed.content,
            has,
            min_id,
            max_id,
            pinned,
            author_types,
            ..SearchQuery::default()
        })
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.active_search() else {
            return;
        };
        let text = search.text(cx);
        if text.trim().is_empty() && search.chips.is_empty() {
            return;
        }
        let scope = search.scope;
        let entry = SavedSearch {
            text: text.trim().to_owned(),
            filters: search.chips.iter().map(Chip::saved).collect(),
        };
        let resolved = self.resolve_search(search, &text);
        let sort = search
            .results
            .as_ref()
            .map_or(SearchSort::default(), |results| results.query.sort);
        let Some(search) = self.active_search_mut() else {
            return;
        };
        search.popover = false;
        search.highlighted = None;
        let (query, status) = match resolved {
            Ok(query) => (query, Status::Loading),
            Err(error) => (SearchQuery::default(), Status::Failed(error)),
        };
        let run = matches!(status, Status::Loading);
        let terms: Rc<[String]> = query
            .content
            .split_whitespace()
            .map(|word| word.trim_matches('"').to_owned())
            .filter(|word| !word.is_empty())
            .collect();
        search.results = Some(Results {
            query: SearchQuery { sort, ..query },
            terms,
            messages: Vec::new(),
            channels: Vec::new(),
            total: 0,
            status,
        });
        if run {
            self.remember_search(scope, entry, cx);
            self.fetch_search(0, cx);
        }
        cx.notify();
    }

    fn set_search_sort(&mut self, sort: SearchSort, cx: &mut Context<Self>) {
        let Some(results) = self
            .active_search_mut()
            .and_then(|search| search.results.as_mut())
            .filter(|results| results.query.sort != sort)
        else {
            return;
        };
        results.query.sort = sort;
        results.query.offset = 0;
        results.messages.clear();
        results.channels.clear();
        results.total = 0;
        results.status = Status::Loading;
        self.fetch_search(0, cx);
        cx.notify();
    }

    fn set_search_page(&mut self, page: u64, cx: &mut Context<Self>) {
        let Some(results) = self
            .active_search_mut()
            .and_then(|search| search.results.as_mut())
        else {
            return;
        };
        results.query.offset = (page.saturating_sub(1) as usize * PAGE).min(MAX_OFFSET);
        results.messages.clear();
        results.channels.clear();
        results.status = Status::Loading;
        self.fetch_search(0, cx);
        cx.notify();
    }

    fn fetch_search(&mut self, attempt: u8, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            self.search_failed("Connect to Discord to search.".to_owned());
            return;
        };
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let Some(query) = search.results.as_ref().map(|results| results.query.clone()) else {
            return;
        };
        search.request += 1;
        let (scope, request) = (search.scope, search.request);
        let response = Tokio::spawn(
            cx,
            async move { session.search_messages(scope, &query).await },
        );
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let retry = this
                .update(cx, |app, cx| {
                    app.search_response(scope, request, attempt, result, cx)
                })
                .ok()
                .flatten();
            let Some(retry_after) = retry else {
                return;
            };
            cx.background_executor().timer(retry_after).await;
            this.update(cx, |app, cx| {
                if app
                    .active_search()
                    .is_some_and(|search| search.request == request)
                {
                    app.fetch_search(attempt + 1, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn search_failed(&mut self, error: String) {
        if let Some(results) = self
            .active_search_mut()
            .and_then(|search| search.results.as_mut())
        {
            results.status = Status::Failed(error);
        }
    }

    fn search_response(
        &mut self,
        scope: SearchScope,
        request: u64,
        attempt: u8,
        result: Result<Result<SearchOutcome, crate::discord::Error>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) -> Option<Duration> {
        self.active_search()
            .filter(|search| search.scope == scope && search.request == request)?;
        cx.notify();
        match result {
            Ok(Ok(SearchOutcome::Page(page))) => {
                self.search_page(scope, page);
                None
            }
            Ok(Ok(SearchOutcome::Indexing { .. })) if attempt >= INDEX_RETRIES => {
                self.search_failed("Discord is still indexing. Try again soon.".to_owned());
                None
            }
            Ok(Ok(SearchOutcome::Indexing { retry_after })) => {
                let results = self.active_search_mut()?.results.as_mut()?;
                results.status = Status::Indexing;
                Some(retry_after)
            }
            Ok(Err(error)) => {
                log::warn!("Could not search messages: {error}");
                self.search_failed("Search failed. Try again.".to_owned());
                None
            }
            Err(error) => {
                log::warn!("Search task failed: {error}");
                self.search_failed("Search failed. Try again.".to_owned());
                None
            }
        }
    }

    fn search_page(&mut self, scope: SearchScope, page: model::SearchPage) {
        let guild_id = match scope {
            SearchScope::Guild(guild_id) => guild_id,
            SearchScope::Channel(_) | SearchScope::Dms => 0,
        };
        let authors: Vec<u64> = page
            .messages
            .iter()
            .filter(|message| message.webhook_id.is_none())
            .map(|message| message.author.id)
            .collect();
        let messages: Vec<_> = page
            .messages
            .into_iter()
            .map(|message| {
                let channel_id = message.channel_id;
                (channel_id, self.store.map_listed_message(guild_id, message))
            })
            .collect();
        if guild_id != 0
            && let Some(session) = self.session.clone()
        {
            let missing = self.store.missing_members(guild_id, authors);
            if !missing.is_empty() {
                session.request_members(guild_id, missing);
            }
        }
        let Some(results) = self
            .active_search_mut()
            .and_then(|search| search.results.as_mut())
        else {
            return;
        };
        results.total = page.total;
        results.messages.extend(messages);
        results.channels = page.channels;
        results.status = Status::Ready;
    }

    fn open_search_result(
        &mut self,
        channel_id: u64,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cross_dm = self
            .active_search()
            .is_some_and(|search| search.scope == SearchScope::Dms);
        if cross_dm && self.store.private_channel(channel_id).is_none() {
            let channel = self
                .active_search()
                .and_then(|search| search.results.as_ref())
                .and_then(|results| {
                    results.channels.iter().find(|channel| {
                        channel.id == channel_id
                            && channel.guild_id.is_none()
                            && matches!(channel.kind, Some(1 | 3))
                    })
                })
                .cloned();
            if let Some(channel) = channel {
                let change = self
                    .store
                    .apply(crate::discord::Event::ChannelUpsert(Box::new(channel)));
                self.apply_change(change, window, cx);
            }
        }
        if channel_id != self.selected_channel_id {
            if cross_dm {
                self.select_dm(channel_id, window, cx);
            } else {
                self.select_channel(channel_id, window, cx);
            }
        }
        if channel_id == self.selected_channel_id {
            self.jump_to_message(message_id, window, cx);
        }
    }
}
