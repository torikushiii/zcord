mod popover;
mod results;
mod search_box;

pub(crate) use popover::popover;
pub(crate) use results::render_results;
pub(crate) use search_box::search_box;

use std::time::{Duration, Instant};

use gpui::{prelude::*, *};
use gpui_component::input::{Backspace, Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;

use super::chat::{self, render_search_message};
use super::{DiscordApp, messages::message_body::label, titlebar};
use crate::discord::{SearchOutcome, SearchQuery, SearchScope, SearchSort, model};
use crate::store::Message;
use crate::text::search_query::{self, HAS_OPTIONS};
use crate::theme::{self, WithoutTextSelection, rpx};

pub(crate) const RESULTS_WIDTH: f32 = 419.;
const PAGE: usize = 25;
const MAX_OFFSET: usize = 9975;
const INDEX_RETRIES: u8 = 5;
const USER_SUGGESTIONS: usize = 7;
const CHANNEL_SUGGESTIONS: usize = 10;
const MEMBER_LIMIT: u32 = USER_SUGGESTIONS as u32;
const MEMBER_QUERY_DELAY: Duration = Duration::from_millis(250);

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
    _events: Subscription,
}

#[derive(Clone)]
struct Chip {
    key: &'static str,
    label: String,
    value: ChipValue,
}

#[derive(Clone)]
enum ChipValue {
    User(u64),
    Channel(u64),
    Has(&'static str),
}

struct Results {
    query: SearchQuery,
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
    Filter(&'static Filter),
    User {
        key: &'static str,
        id: u64,
        name: String,
        username: String,
    },
    Channel {
        id: u64,
        name: String,
    },
    Has {
        name: &'static str,
        api: &'static str,
    },
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

impl MessageSearch {
    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
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
            _events: events,
        });
        cx.notify();
    }

    /// Clears what's in the box, then closes the popover and results; `false` when nothing was
    /// open.
    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(search) = self.active_search_mut() else {
            return false;
        };
        if !search.text(cx).is_empty() || !search.chips.is_empty() {
            self.clear_search(window, cx);
            return true;
        }
        if search.popover {
            search.popover = false;
        } else if search.results.take().is_none() {
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
        if text.trim().is_empty() && search.chips.is_empty() {
            search.results = None;
        }
        let partial = search_query::active_filter(&text)
            .filter(|(key, partial)| {
                !partial.is_empty()
                    && (key.eq_ignore_ascii_case("from") || key.eq_ignore_ascii_case("mentions"))
            })
            .map(|(_, partial)| partial.to_owned());
        search.member_query = None;
        if let (Some(guild_id), Some(session), Some(partial)) = (guild_id, session, partial) {
            search.member_query = Some(cx.spawn(async move |_, cx| {
                cx.background_executor().timer(MEMBER_QUERY_DELAY).await;
                session.query_members(guild_id, &partial, MEMBER_LIMIT);
            }));
        }
        cx.notify();
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
        let Some((key, partial)) = search_query::active_filter(&text) else {
            return FILTERS
                .iter()
                .filter(|filter| filter.key != "in" || guild_id.is_some())
                .map(Row::Filter)
                .collect();
        };
        let partial = partial.trim_start_matches(['@', '#', '"']).to_lowercase();
        match filter_key(key) {
            Some(key @ ("from" | "mentions")) => self
                .store
                .search_users(
                    guild_id,
                    self.selected_channel_id,
                    &partial,
                    USER_SUGGESTIONS,
                )
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
                    })
                })
                .collect(),
            Some("in") => self
                .guild()
                .into_iter()
                .flat_map(|guild| guild.channels())
                .filter(|channel| channel.kind.is_selectable())
                .filter(|channel| channel.name.to_lowercase().contains(&partial))
                .take(CHANNEL_SUGGESTIONS)
                .map(|channel| Row::Channel {
                    id: channel.id,
                    name: channel.name.clone(),
                })
                .collect(),
            Some("has") => HAS_OPTIONS
                .iter()
                .filter(|(name, _)| name.starts_with(&partial))
                .map(|(name, api)| Row::Has { name, api })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Moves the popover's highlight; `false` when the popover isn't open.
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
        let next = match search.highlighted {
            Some(current) => (current as isize + step).rem_euclid(count as isize) as usize,
            None if step > 0 => 0,
            None => count - 1,
        };
        search.highlighted = Some(next);
        cx.notify();
        true
    }

    /// While a filter is being typed its first suggestion is the default choice.
    fn chosen_row(&self, cx: &App) -> Option<Row> {
        let search = self.active_search().filter(|search| search.popover)?;
        let rows = self.search_rows(cx);
        let typing = search_query::active_filter(&search.text(cx)).is_some();
        let index = search.highlighted.or(typing.then_some(0))?;
        rows.into_iter().nth(index)
    }

    pub(super) fn accept_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.chosen_row(cx) else {
            return false;
        };
        self.apply_search_row(row, window, cx);
        true
    }

    fn apply_search_row(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let text = search.text(cx);
        let base = text[..last_token_start(&text)].trim_end().to_owned();
        let mut chip = |key, label, value| {
            search.chips.push(Chip { key, label, value });
            base.clone()
        };
        let insert = match row {
            Row::Filter(filter) => {
                let start = last_token_start(&text);
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
                format!("{spaced}{}:", filter.key)
            }
            Row::User {
                key, id, username, ..
            } => chip(key, username, ChipValue::User(id)),
            Row::Channel { id, name } => chip("in", name, ChipValue::Channel(id)),
            Row::Has { name, api } => chip("has", name.to_owned(), ChipValue::Has(api)),
        };
        let input = search.input.clone();
        input.update(cx, |input, cx| {
            input.set_value(insert, window, cx);
            input.focus(window, cx);
        });
        if let Some(search) = self.active_search_mut() {
            search.highlighted = None;
            search.popover = true;
        }
        cx.notify();
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
        for chip in &search.chips {
            match chip.value {
                ChipValue::User(id) if chip.key == "from" => author_ids.push(id),
                ChipValue::User(id) => mentions.push(id),
                ChipValue::Channel(id) => channel_ids.push(id),
                ChipValue::Has(api) => has.push(api.to_owned()),
            }
        }
        Ok(SearchQuery {
            author_ids,
            mentions,
            channel_ids,
            content: parsed.content,
            has,
            min_id: parsed.min_id,
            max_id: parsed.max_id,
            pinned: parsed.pinned,
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
        let resolved = self.resolve_search(search, &text);
        let sort = search
            .results
            .as_ref()
            .map_or(SearchSort::default(), |results| results.query.sort);
        let Some(search) = self.active_search_mut() else {
            return;
        };
        search.popover = false;
        let (query, status) = match resolved {
            Ok(query) => (query, Status::Loading),
            Err(error) => (SearchQuery::default(), Status::Failed(error)),
        };
        let run = matches!(status, Status::Loading);
        search.results = Some(Results {
            query: SearchQuery { sort, ..query },
            messages: Vec::new(),
            channels: Vec::new(),
            total: 0,
            status,
        });
        if run {
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

    /// Returns how long to wait before asking again while Discord indexes.
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
