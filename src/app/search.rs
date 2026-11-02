use std::time::{Duration, Instant};

use gpui::{prelude::*, *};
use gpui_component::input::{Backspace, Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;

use super::chat::{self, render_search_message};
use super::{DiscordApp, message_body::label, titlebar};
use crate::discord::{SearchOutcome, SearchQuery, SearchScope, SearchSort, model};
use crate::store::Message;
use crate::text::search_query::{self, HAS_OPTIONS};
use crate::theme::{self, WithoutTextSelection, rpx};

pub(crate) const RESULTS_WIDTH: f32 = 419.;
const POPOVER_WIDTH: f32 = 356.;
const ROW_HEIGHT: f32 = 44.;
const RESULT_LIMIT: u64 = 10_000;
const PAGER_SPAN: u64 = 2;
const PAGE: usize = 25;
const MAX_OFFSET: usize = 9975;
const INDEX_RETRIES: u8 = 5;
const USER_SUGGESTIONS: usize = 7;
const CHANNEL_SUGGESTIONS: usize = 10;
const MEMBER_LIMIT: u32 = USER_SUGGESTIONS as u32;
const MEMBER_QUERY_DELAY: Duration = Duration::from_millis(250);
const SPINNER_SIZE: f32 = 16.;
const SORT_MENU_WIDTH: f32 = 170.;
const CARD_WIDTH: f32 = RESULTS_WIDTH - 1. - 32. - 2.;
/// The press that closes the menu also clicks its button, which must not reopen it.
const SORT_REOPEN_GUARD: Duration = Duration::from_millis(200);
const SORTS: [(SearchSort, &str); 3] = [
    (SearchSort::Newest, "Newest"),
    (SearchSort::Oldest, "Oldest"),
    (SearchSort::Relevant, "Most Relevant"),
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
            Some(SearchScope::Channel(self.selected_channel_id))
        } else {
            Some(SearchScope::Guild(self.selected_guild_id))
        }
    }

    fn active_search(&self) -> Option<&MessageSearch> {
        self.message_search
            .as_ref()
            .filter(|search| Some(search.scope) == self.search_scope())
    }

    fn active_search_mut(&mut self) -> Option<&mut MessageSearch> {
        let scope = self.search_scope();
        self.message_search
            .as_mut()
            .filter(|search| Some(search.scope) == scope)
    }

    pub(super) fn sync_search(&mut self) {
        if self.message_search.is_some() && self.active_search().is_none() {
            self.message_search = None;
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
        self.message_search = Some(MessageSearch {
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
            Some(SearchScope::Channel(_)) => None,
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
            SearchScope::Channel(_) => None,
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
                let spaced = if text.is_empty() || text.ends_with(char::is_whitespace) {
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
            SearchScope::Channel(_) => None,
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
            SearchScope::Channel(_) => 0,
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
        results.status = Status::Ready;
    }

    fn open_search_result(
        &mut self,
        channel_id: u64,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if channel_id != self.selected_channel_id {
            self.select_channel(channel_id, window, cx);
        }
        if channel_id == self.selected_channel_id {
            self.jump_to_message(message_id, window, cx);
        }
    }
}

fn search_placeholder(app: &DiscordApp) -> String {
    let name = if app.is_home() {
        app.store.private_channel_name(app.selected_channel_id)
    } else {
        app.guild()
            .map_or_else(String::new, |guild| guild.name.clone())
    };
    format!("Search {name}")
}

pub(crate) fn search_box(app: &DiscordApp, cx: &App) -> AnyElement {
    let open = app.entity.clone();
    let clear = app.entity.clone();
    let search = app.active_search();
    let typed = search.is_some_and(|search| !search.text(cx).is_empty());
    let chips = search.map_or(&[][..], |search| &search.chips);
    let empty = !typed && chips.is_empty();
    let backspace = app.entity.clone();
    let placeholder = search_placeholder(app);
    div()
        .id("search-box")
        .flex()
        .items_center()
        .w_full()
        .h(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(rgba(theme::border_emphasis()))
        .overflow_hidden()
        .cursor_text()
        .text_size(rpx(14.))
        .line_height(rpx(22.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::faint()))
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| app.open_search(window, cx)).ok();
        })
        .capture_action(move |_: &Backspace, _, cx| {
            let removed = backspace
                .update(cx, |app, cx| app.remove_last_chip(cx))
                .unwrap_or(false);
            if removed {
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .px(rpx(6.))
                .children(chips.iter().flat_map(box_chip))
                .child(div().relative().flex_1().min_w_0().map(|this| {
                    let field = search.map(|search| {
                        Input::new(&search.input)
                            .appearance(false)
                            .px_0()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::text()))
                    });
                    this.when(empty, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .child(div().min_w_0().truncate().child(placeholder)),
                        )
                    })
                    .when(search.is_none(), |this| this.h(rpx(22.)))
                    .children(field)
                })),
        )
        .map(|this| {
            if empty {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(24.))
                        .child(theme::icon("search", 16., theme::muted())),
                )
            } else {
                this.child(
                    theme::icon_button("search-clear", "x", 16., theme::muted())
                        .flex_none()
                        .size(rpx(24.))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            clear
                                .update(cx, |app, cx| app.clear_search(window, cx))
                                .ok();
                        }),
                )
            }
        })
        .into_any_element()
}

fn box_chip(chip: &Chip) -> [Div; 2] {
    let pill = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(22.))
            .bg(rgb(theme::surface_highest()))
            .text_color(rgb(theme::strong()))
    };
    [
        pill()
            .px(rpx(4.))
            .rounded_l(rpx(4.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(format!("{}:", chip.key)),
        pill()
            .max_w(rpx(120.))
            .ml(rpx(-2.))
            .mr(rpx(2.))
            .pr(rpx(4.))
            .rounded_r(rpx(4.))
            .child(div().min_w_0().truncate().child(chip.label.clone())),
    ]
}

fn row_chip(app: &DiscordApp, chip: &Chip, cx: &App) -> Div {
    let user = match chip.value {
        ChipValue::User(id) => Some(id),
        _ => None,
    };
    let pill = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(24.))
            .bg(rgba(theme::control_selected()))
            .text_color(rgb(theme::strong()))
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            pill()
                .pl(rpx(6.))
                .pr(rpx(2.))
                .rounded_l(rpx(4.))
                .child(format!("{}:", chip.key)),
        )
        .child(
            pill()
                .gap(rpx(4.))
                .max_w(rpx(160.))
                .pl(rpx(2.))
                .pr(rpx(6.))
                .mr(rpx(4.))
                .rounded_r(rpx(4.))
                .children(
                    user.map(|id| theme::user_avatar(app.store.user(id), id, 16., None, false, cx)),
                )
                .child(div().min_w_0().truncate().child(chip.label.clone())),
        )
}

fn search_row(
    app: &DiscordApp,
    search: &MessageSearch,
    text: String,
    cx: &App,
) -> impl IntoElement {
    let run = app.entity.clone();
    div()
        .id("search-run")
        .flex()
        .flex_none()
        .items_center()
        .min_h(rpx(32.))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(move |_, _, cx| {
            run.update(cx, |app, cx| app.run_search(cx)).ok();
        })
        .child(theme::icon("search", 20., theme::faint()).mr(rpx(8.)))
        .child(div().flex_none().mr(rpx(4.)).child("Search for"))
        .children(search.chips.iter().map(|filter| row_chip(app, filter, cx)))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
}

fn has_icon(name: &str) -> &'static str {
    match name {
        "link" => "link",
        "embed" => "app-window",
        "poll" => "chart-column",
        "video" => "video",
        "image" => "image",
        "sound" => "volume-2",
        "sticker" => "sticker",
        "forward" => "forward",
        _ => "file",
    }
}

fn popover_row(
    app: &DiscordApp,
    index: usize,
    row: Row,
    highlighted: bool,
    cx: &App,
) -> AnyElement {
    let choose = app.entity.clone();
    let chosen = row.clone();
    let base = div()
        .id(("search-row", index))
        .flex()
        .items_center()
        .gap(rpx(8.))
        .min_h(rpx(32.))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(highlighted, |this| this.bg(rgba(theme::selected())))
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            choose
                .update(cx, |app, cx| {
                    app.apply_search_row(chosen.clone(), window, cx)
                })
                .ok();
        });
    let title = |text: String| {
        div()
            .min_w_0()
            .truncate()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .child(text)
    };
    match row {
        Row::Filter(filter) => base
            .min_h(rpx(ROW_HEIGHT))
            .child(theme::icon(filter.icon, 20., theme::faint()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(title(filter.title.to_owned()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(4.))
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::faint()))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::muted()))
                                    .child(format!("{}:", filter.key)),
                            )
                            .child(filter.hint),
                    ),
            ),
        Row::User {
            id, name, username, ..
        } => base
            .child(theme::user_avatar(
                app.store.user(id),
                id,
                24.,
                None,
                false,
                cx,
            ))
            .child(title(name))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(12.))
                    .text_color(rgb(theme::muted()))
                    .child(username),
            ),
        Row::Channel { name, .. } => base
            .child(theme::icon("hash", 20., theme::muted()))
            .child(title(name)),
        Row::Has { name, .. } => base
            .child(theme::icon(has_icon(name), 20., theme::faint()))
            .child(title(name.to_owned())),
    }
    .into_any_element()
}

pub(crate) fn popover(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app.active_search().filter(|search| search.popover)?;
    let text = search.text(cx);
    let active = search_query::active_filter(&text);
    // Date and `pinned:` filters are typed by hand, so they have nothing to suggest.
    if active.is_some_and(|(key, _)| filter_key(key).is_none()) {
        return None;
    }
    let heading = match active.and_then(|(key, _)| filter_key(key)) {
        Some("from") => "From User",
        Some("mentions") => "Mentions User",
        Some("in") => "In Channel",
        Some("has") => "Has",
        _ => "Filters",
    };
    let rows = app.search_rows(cx);
    let close = app.entity.clone();
    let empty = rows.is_empty();
    let ready = active.is_none() && (!text.trim().is_empty() || !search.chips.is_empty());
    Some(
        div()
            .id("search-popover")
            .absolute()
            .top(rpx(titlebar::HEIGHT + chat::HEADER_HEIGHT - 1.))
            .right(rpx(12.))
            .flex()
            .flex_col()
            .w(rpx(POPOVER_WIDTH))
            .px(rpx(8.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::outline()))
            .shadow_lg()
            .occlude()
            .without_text_selection()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            search.popover = false;
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .when(ready, |this| {
                this.child(search_row(app, search, text.trim().to_owned(), cx))
                    .child(
                        div()
                            .h(px(1.))
                            .my(rpx(8.))
                            .mx(rpx(-8.))
                            .bg(rgba(theme::outline())),
                    )
            })
            .child(
                div()
                    .px(rpx(8.))
                    .py(rpx(4.))
                    .mb(rpx(4.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::muted()))
                    .child(heading),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(4.))
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        popover_row(app, index, row, search.highlighted == Some(index), cx)
                    })),
            )
            .when(empty, |this| {
                this.child(
                    div()
                        .px(rpx(8.))
                        .py(rpx(6.))
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::muted()))
                        .child("No matches"),
                )
            })
            .into_any_element(),
    )
}

fn sort_control(app: &DiscordApp, search: &MessageSearch, sort: SearchSort) -> impl IntoElement {
    let toggle = app.entity.clone();
    let close = app.entity.clone();
    let open = search.sort_menu;
    let option = |choice: SearchSort, text: &'static str| {
        let entity = app.entity.clone();
        div()
            .id(text)
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(12.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(6.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            search.sort_menu = false;
                        }
                        app.set_search_sort(choice, cx);
                        cx.notify();
                    })
                    .ok();
            })
            .child(text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(20.))
                    .rounded_full()
                    .map(|this| {
                        if choice == sort {
                            this.bg(rgb(theme::BLURPLE))
                                .child(div().size(rpx(8.)).rounded_full().bg(white()))
                        } else {
                            this.border_2().border_color(rgb(theme::muted()))
                        }
                    }),
            )
    };
    let menu = open.then(|| {
        div().absolute().top(rpx(36.)).right_0().size_0().child(
            deferred(
                anchored().anchor(Anchor::TopRight).child(
                    div()
                        .id("search-sort-menu")
                        .flex()
                        .flex_col()
                        .w(rpx(SORT_MENU_WIDTH))
                        .p(rpx(8.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::surface_high()))
                        .border_1()
                        .border_color(rgba(theme::outline()))
                        .shadow_lg()
                        .occlude()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .on_mouse_down_out(move |_, _, cx| {
                            close
                                .update(cx, |app, cx| {
                                    if let Some(search) = app.active_search_mut() {
                                        search.sort_menu = false;
                                        search.sort_closed = Some(Instant::now());
                                        cx.notify();
                                    }
                                })
                                .ok();
                        })
                        .children(SORTS.into_iter().map(|(choice, text)| option(choice, text))),
                ),
            )
            .with_priority(1),
        )
    });
    div().relative().flex_none().children(menu).child(
        div()
            .id("search-sort")
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgba(theme::control_bg()))
            .border_1()
            .border_color(rgba(theme::border_soft()))
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .on_click(move |_, _, cx| {
                toggle
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            let guarded = search
                                .sort_closed
                                .is_some_and(|at| at.elapsed() < SORT_REOPEN_GUARD);
                            search.sort_menu = !guarded && !search.sort_menu;
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .child(theme::icon("arrow-up-down", 16., theme::strong()))
            .child("Sort"),
    )
}

fn with_commas(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn note(text: String) -> Div {
    div()
        .py(rpx(32.))
        .px(rpx(16.))
        .text_center()
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(text)
}

fn result_card(app: &DiscordApp, channel_id: u64, message: &Message, cx: &App) -> impl IntoElement {
    let open = app.entity.clone();
    let id = message.id;
    let group = SharedString::from(format!("search-result-{id}"));
    div()
        .id(("search-result", id))
        .group(group.clone())
        .relative()
        .flex_none()
        .mb(rpx(8.))
        .py(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::bg()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .overflow_hidden()
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| {
                app.open_search_result(channel_id, id, window, cx)
            })
            .ok();
        })
        .child(render_search_message(app, message, CARD_WIDTH, cx))
        .child(
            div()
                .absolute()
                .top(rpx(8.))
                .right(rpx(8.))
                .flex()
                .items_center()
                .h(rpx(24.))
                .p(rpx(4.))
                .rounded(rpx(3.))
                .bg(rgb(theme::bg()))
                .opacity(0.)
                .group_hover(group, |style| style.opacity(1.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child("Jump"),
        )
}

fn channel_heading(app: &DiscordApp, channel_id: u64, key: u64) -> AnyElement {
    let name = app.store.channel_name(channel_id).unwrap_or_default();
    let category = app.guild().and_then(|guild| {
        guild
            .categories
            .iter()
            .find(|category| {
                category
                    .channels
                    .iter()
                    .any(|channel| channel.id == channel_id)
            })
            .map(|category| category.name.clone())
            .filter(|name| !name.is_empty())
    });
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(20.))
        .mb(rpx(8.))
        .min_w_0()
        .child(theme::icon("hash", 16., theme::strong()).mr(rpx(4.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(label(("search-channel", key), name)),
        )
        .children(category.map(|category| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .max_w(rpx(160.))
                .gap(rpx(4.))
                .ml(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child(theme::icon("chevron-right", 12., theme::text()))
                .child(div().min_w_0().truncate().child(category))
        }))
        .into_any_element()
}

fn pager(app: &DiscordApp, page: u64, pages: u64) -> Div {
    let button =
        |key: &'static str, slot: u64, text: String, target: u64, on: bool, enabled: bool| {
            let entity = app.entity.clone();
            div()
                .id((key, slot))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(rpx(32.))
                .min_w(rpx(32.))
                .px(rpx(8.))
                .rounded(rpx(8.))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if enabled {
                    theme::strong()
                } else {
                    theme::faint()
                }))
                .when(on, |this| this.bg(rgba(theme::control_selected())))
                .when(enabled && !on, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| app.set_search_page(target, cx))
                                .ok();
                        })
                })
                .child(text)
        };
    let first = page.saturating_sub(PAGER_SPAN).max(1);
    let last = (page + PAGER_SPAN).min(pages);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .h(rpx(56.))
        .px(rpx(16.))
        .child(button(
            "search-back",
            0,
            "Back".to_owned(),
            page.saturating_sub(1),
            false,
            page > 1,
        ))
        .children((first..=last).map(|number| {
            button(
                "search-page",
                number,
                number.to_string(),
                number,
                number == page,
                true,
            )
        }))
        .child(button(
            "search-next",
            0,
            "Next".to_owned(),
            page + 1,
            false,
            page < pages,
        ))
}

pub(crate) fn render_results(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app.active_search()?;
    let results = search.results.as_ref()?;
    let close = app.entity.clone();
    let heading = match results.status {
        Status::Loading => "Searching\u{2026}".to_owned(),
        Status::Indexing => "Indexing\u{2026}".to_owned(),
        _ if results.total == 1 => "1 Result".to_owned(),
        _ => format!("{} Results", with_commas(results.total)),
    };
    let busy = matches!(results.status, Status::Loading | Status::Indexing);
    let sort = results.query.sort;
    let show_channels = matches!(search.scope, SearchScope::Guild(_));
    let mut groups: Vec<(u64, Vec<&Message>)> = Vec::new();
    for (channel_id, message) in &results.messages {
        match groups.last_mut() {
            Some((channel, messages)) if channel == channel_id => messages.push(message),
            _ => groups.push((*channel_id, vec![message])),
        }
    }
    let list = groups.into_iter().map(|(channel_id, messages)| {
        let key = messages.first().map_or(channel_id, |message| message.id);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .mb(rpx(24.))
            .when(show_channels, |this| {
                this.child(channel_heading(app, channel_id, key))
            })
            .children(
                messages
                    .into_iter()
                    .map(|message| result_card(app, channel_id, message, cx)),
            )
    });
    let body = match &results.status {
        Status::Failed(error) => Some(note(error.clone())),
        Status::Ready if results.messages.is_empty() => Some(note("No results found.".to_owned())),
        _ => None,
    };
    let page = (results.query.offset / PAGE) as u64 + 1;
    let pages = results.total.min(RESULT_LIMIT).div_ceil(PAGE as u64);
    Some(
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::bg()))
            .border_l_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(rpx(8.))
                    .h(rpx(48.))
                    .px(rpx(16.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap(rpx(6.))
                            .min_w_0()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(div().min_w_0().truncate().child(heading))
                            .when(busy, |this| {
                                this.child(theme::spinner(SPINNER_SIZE, theme::muted()))
                            }),
                    )
                    .child(sort_control(app, search, sort))
                    .child(
                        theme::icon_button("search-close", "x", 20., theme::muted()).on_click(
                            move |_, _, cx| {
                                close
                                    .update(cx, |app, cx| {
                                        if let Some(search) = app.active_search_mut() {
                                            search.popover = false;
                                            search.results = None;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .id(("search-results", page))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px(rpx(16.))
                    .pt(rpx(16.))
                    .children(list)
                    .children(body),
            )
            .when(pages > 1, |this| this.child(pager(app, page, pages)))
            .into_any_element(),
    )
}
