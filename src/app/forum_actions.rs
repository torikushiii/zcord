use super::*;
use crate::discord::{ForumQuery, ForumSort, model};
use gpui_component::input::{InputEvent, InputState};

const MENU_REOPEN_GUARD: Duration = Duration::from_millis(200);

impl DiscordApp {
    pub(crate) fn open_thread(
        &mut self,
        thread_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_forum().is_some() {
            self.open_thread_pane(thread_id, window, cx);
        } else {
            self.select_channel(thread_id, window, cx);
        }
    }

    pub(crate) fn back_to_forum(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(forum_id) = self.thread_parent() {
            self.select_channel(forum_id, window, cx);
        }
    }

    fn forum_page(&self) -> Option<&ForumPage> {
        self.selected_forum()
            .and_then(|forum_id| self.forum_pages.get(&forum_id))
    }

    pub(crate) fn forum_loading(&self) -> bool {
        self.forum_page().is_some_and(|page| page.loading)
    }

    pub(crate) fn forum_has_more(&self) -> bool {
        self.forum_page().is_some_and(|page| page.has_more)
    }

    pub(crate) fn forum_sort(&self, forum_id: u64) -> ForumSort {
        self.forum_pages
            .get(&forum_id)
            .and_then(|page| page.sort)
            .unwrap_or_else(|| self.store.forum_default_sort(forum_id))
    }

    pub(crate) fn forum_gallery(&self, forum_id: u64) -> bool {
        self.forum_pages
            .get(&forum_id)
            .and_then(|page| page.gallery)
            .unwrap_or_else(|| self.store.forum_default_gallery(forum_id))
    }

    pub(crate) fn forum_tag_filter(&self, forum_id: u64) -> &[u64] {
        self.forum_pages
            .get(&forum_id)
            .map_or(&[], |page| page.tags.as_slice())
    }

    pub(crate) fn forum_menu_open(&self) -> bool {
        self.forum_page().is_some_and(|page| page.menu)
    }

    pub(crate) fn forum_columns(&self) -> Rc<Cell<usize>> {
        self.forum_columns.clone()
    }

    pub(crate) fn toggle_forum_menu(&mut self, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let page = self.forum_pages.entry(forum_id).or_default();
        let guarded = page
            .menu_closed
            .is_some_and(|at| at.elapsed() < MENU_REOPEN_GUARD);
        page.menu = !guarded && !page.menu;
        cx.notify();
    }

    pub(crate) fn close_forum_menu(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self
            .selected_forum()
            .and_then(|forum_id| self.forum_pages.get_mut(&forum_id))
        else {
            return;
        };
        if page.menu {
            page.menu = false;
            page.menu_closed = Some(Instant::now());
            cx.notify();
        }
    }

    pub(crate) fn set_forum_sort(&mut self, sort: ForumSort, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        if self.forum_sort(forum_id) == sort {
            return;
        }
        self.forum_pages.entry(forum_id).or_default().sort = Some(sort);
        self.reload_forum(forum_id, cx);
    }

    pub(crate) fn set_forum_gallery(&mut self, gallery: bool, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        self.forum_pages.entry(forum_id).or_default().gallery = Some(gallery);
        cx.notify();
    }

    pub(crate) fn toggle_forum_tag(&mut self, tag_id: u64, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let tags = &mut self.forum_pages.entry(forum_id).or_default().tags;
        match tags.iter().position(|tag| *tag == tag_id) {
            Some(index) => {
                tags.remove(index);
            }
            None => tags.push(tag_id),
        }
        self.reload_forum(forum_id, cx);
    }

    pub(crate) fn forum_match_all(&self, forum_id: u64) -> bool {
        self.forum_pages
            .get(&forum_id)
            .is_some_and(|page| page.match_all)
    }

    pub(crate) fn set_forum_match_all(&mut self, match_all: bool, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let page = self.forum_pages.entry(forum_id).or_default();
        if page.match_all == match_all {
            return;
        }
        page.match_all = match_all;
        if page.tags.is_empty() {
            cx.notify();
        } else {
            self.reload_forum(forum_id, cx);
        }
    }

    /// Drops the viewer's sort, layout and tag-matching choices; the tag filter itself stays.
    pub(crate) fn reset_forum_view(&mut self, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let sorted = self
            .forum_pages
            .get(&forum_id)
            .is_some_and(|page| page.sort.is_some());
        let page = self.forum_pages.entry(forum_id).or_default();
        let matched = page.match_all && !page.tags.is_empty();
        page.sort = None;
        page.gallery = None;
        page.match_all = false;
        if sorted || matched {
            self.reload_forum(forum_id, cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn clear_forum_tags(&mut self, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let page = self.forum_pages.entry(forum_id).or_default();
        if page.tags.is_empty() {
            return;
        }
        page.tags.clear();
        self.reload_forum(forum_id, cx);
    }

    /// Restarts paging for the current sort and tag filter; posts already known stay listed.
    fn reload_forum(&mut self, forum_id: u64, cx: &mut Context<Self>) {
        let page = self.forum_pages.entry(forum_id).or_default();
        page.generation += 1;
        page.offset = 0;
        page.has_more = false;
        page.loading = false;
        page.loaded = false;
        self.load_forum(forum_id, cx);
        cx.notify();
    }

    pub(crate) fn load_forum(&mut self, forum_id: u64, cx: &mut Context<Self>) {
        let page = self.forum_pages.entry(forum_id).or_default();
        if page.loaded || page.loading {
            self.fill_forum_posts(forum_id, cx);
            return;
        }
        if self.session.is_none() || !self.live {
            page.loaded = true;
            return;
        }
        self.fetch_forum_page(forum_id, 0, cx);
    }

    pub(crate) fn load_more_forum(&mut self, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let Some(page) = self.forum_pages.get(&forum_id) else {
            return;
        };
        if !page.loaded || !page.has_more || page.loading {
            return;
        }
        self.fetch_forum_page(forum_id, page.offset, cx);
    }

    fn fetch_forum_page(&mut self, forum_id: u64, offset: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let query = ForumQuery {
            sort: self.forum_sort(forum_id),
            tags: self.forum_tag_filter(forum_id).to_vec(),
            match_all: self.forum_match_all(forum_id),
            offset,
        };
        let page = self.forum_pages.entry(forum_id).or_default();
        page.loading = true;
        let generation = page.generation;
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_forum_posts(Some(guild_id), forum_id, query)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = app.forum_pages.entry(forum_id).or_default();
                if page.generation != generation {
                    return;
                }
                page.loading = false;
                match result {
                    Ok(Ok(search)) => {
                        page.loaded = true;
                        page.offset = offset + search.threads.len();
                        page.has_more = search.has_more;
                        let change = app.store.apply_forum_search(forum_id, search);
                        if matches!(change, Change::Structure) {
                            app.sync_channel_rows();
                        }
                        app.fill_forum_posts(forum_id, cx);
                    }
                    Ok(Err(error)) => log::warn!("Could not load forum posts: {error}"),
                    Err(error) => log::warn!("Forum task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Requests what a card needs but the search page lacked: authors missing from the member
    /// cache, and starter messages of posts the search didn't return (pinned or older ones).
    fn fill_forum_posts(&mut self, forum_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let guild_id = self.selected_guild_id;
        let requested = &self.forum_pages.entry(forum_id).or_default().requested;
        let mut authors = Vec::new();
        let mut starters = Vec::new();
        for post in self.store.forum_posts_sorted(forum_id, ForumSort::Activity) {
            authors.push(
                post.first_message
                    .as_ref()
                    .map_or(post.owner_id, |message| message.author_id),
            );
            if post.first_message.is_none() && !requested.contains(&post.id) {
                starters.push(post.id);
            }
        }
        let missing = self.store.missing_members(guild_id, authors);
        if !missing.is_empty() {
            session.request_members(guild_id, missing);
        }
        if starters.is_empty() {
            return;
        }
        self.forum_pages
            .entry(forum_id)
            .or_default()
            .requested
            .extend(starters.iter().copied());
        for thread_id in starters {
            let session = session.clone();
            let request = Tokio::spawn(cx, async move {
                session
                    .fetch_messages_around(Some(guild_id), thread_id, thread_id, 1)
                    .await
            });
            cx.spawn(async move |this, cx| {
                let Ok(Ok(messages)) = request.await else {
                    return;
                };
                this.update(cx, |app, cx| {
                    let mut filled = false;
                    for message in &messages {
                        filled |= app.store.apply_forum_starter(message);
                    }
                    if filled {
                        let authors = messages.iter().map(|message| message.author.id);
                        let missing = app.store.missing_members(guild_id, authors);
                        if !missing.is_empty()
                            && let Some(session) = app.session.clone()
                        {
                            session.request_members(guild_id, missing);
                        }
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }
}

const SEARCH_DELAY: Duration = Duration::from_millis(300);
/// Post creation is untested against Discord, so New Post shows but stays disabled.
const NEW_POST_ENABLED: bool = false;

/// The forum's top card: it searches posts by title, and expands into the New Post form.
pub(crate) struct ForumHeader {
    pub(crate) forum_id: u64,
    pub(crate) input: Entity<InputState>,
    pub(crate) body: Entity<TextareaState>,
    pub(crate) creating: bool,
    pub(crate) tags: Vec<u64>,
    /// Post ids matching the search; `None` while the box is empty.
    pub(crate) results: Option<Vec<u64>>,
    pub(crate) searching: bool,
    pub(crate) posting: bool,
    pub(crate) error: Option<String>,
    search: Option<Task<()>>,
    _events: Subscription,
}

impl DiscordApp {
    pub(crate) fn ensure_forum_header(
        &mut self,
        forum_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .forum_header
            .as_ref()
            .is_some_and(|header| header.forum_id == forum_id)
        {
            return;
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search or create a post..."));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 12)
                .placeholder("Enter a message")
        });
        let events = cx.subscribe_in(&input, window, |app, _, event, window, cx| match event {
            InputEvent::Change => app.forum_query_changed(cx),
            InputEvent::PressEnter { .. } => app.start_forum_post(window, cx),
            _ => {}
        });
        self.forum_header = Some(ForumHeader {
            forum_id,
            input,
            body,
            creating: false,
            tags: Vec::new(),
            results: None,
            searching: false,
            posting: false,
            error: None,
            search: None,
            _events: events,
        });
    }

    fn forum_query_changed(&mut self, cx: &mut Context<Self>) {
        let Some(header) = &mut self.forum_header else {
            return;
        };
        if header.creating {
            return;
        }
        let query = header.input.read(cx).value().trim().to_owned();
        if query.is_empty() {
            header.search = None;
            header.searching = false;
            header.results = None;
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let forum_id = header.forum_id;
        header.searching = true;
        header.search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let result = Tokio::spawn(
                cx,
                async move { session.search_threads(forum_id, &query).await },
            )
            .await;
            this.update(cx, |app, cx| {
                let Some(header) = app
                    .forum_header
                    .as_mut()
                    .filter(|header| header.forum_id == forum_id)
                else {
                    return;
                };
                header.searching = false;
                match result {
                    Ok(Ok(page)) => {
                        header.results =
                            Some(page.threads.iter().map(|thread| thread.id).collect());
                        let search = model::ForumSearch {
                            threads: page.threads,
                            ..Default::default()
                        };
                        if matches!(
                            app.store.apply_forum_search(forum_id, search),
                            Change::Structure
                        ) {
                            app.sync_channel_rows();
                        }
                        app.fill_forum_posts(forum_id, cx);
                    }
                    Ok(Err(error)) => log::warn!("Could not search forum posts: {error}"),
                    Err(error) => log::warn!("Forum search task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Why New Post can't be used here, or `None` when it can.
    pub(crate) fn new_post_blocked(&self, forum_id: u64) -> Option<&'static str> {
        if !NEW_POST_ENABLED {
            Some("Creating posts isn't available in zcord yet")
        } else if !self.live || !self.store.can_send(forum_id) {
            Some("You don't have permission to create posts here")
        } else {
            None
        }
    }

    pub(crate) fn start_forum_post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(forum_id) = self.forum_header.as_ref().map(|header| header.forum_id) else {
            return;
        };
        if self.new_post_blocked(forum_id).is_some() {
            return;
        }
        let Some(header) = &mut self.forum_header else {
            return;
        };
        if header.creating {
            return;
        }
        header.creating = true;
        header.search = None;
        header.searching = false;
        header.results = None;
        header.error = None;
        let input = header.input.clone();
        input.update(cx, |input, cx| {
            input.set_placeholder("Post title", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn cancel_forum_post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(header) = &mut self.forum_header else {
            return;
        };
        header.creating = false;
        header.tags.clear();
        header.error = None;
        let (input, body) = (header.input.clone(), header.body.clone());
        input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder("Search or create a post...", window, cx);
        });
        body.update(cx, |body, cx| body.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn toggle_new_post_tag(&mut self, tag_id: u64, cx: &mut Context<Self>) {
        let Some(header) = &mut self.forum_header else {
            return;
        };
        match header.tags.iter().position(|tag| *tag == tag_id) {
            Some(index) => {
                header.tags.remove(index);
            }
            None => header.tags.push(tag_id),
        }
        cx.notify();
    }

    pub(crate) fn submit_forum_post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(forum_id) = self.forum_header.as_ref().map(|header| header.forum_id) else {
            return;
        };
        if self.new_post_blocked(forum_id).is_some() {
            return;
        }
        let requires_tag = self.store.forum_requires_tag(forum_id);
        let Some(header) = &mut self.forum_header else {
            return;
        };
        if header.posting {
            return;
        }
        let name = header.input.read(cx).value().trim().to_owned();
        let content = composer_tokens::to_wire(&header.body.read(cx).value())
            .trim()
            .to_owned();
        if name.is_empty() || content.is_empty() {
            header.error = Some("A post needs a title and a message.".to_owned());
            cx.notify();
            return;
        }
        if requires_tag && header.tags.is_empty() {
            header.error = Some("This forum requires at least one tag.".to_owned());
            cx.notify();
            return;
        }
        let tags = header.tags.clone();
        header.posting = true;
        header.error = None;
        let (content, _) = self.encode_emoji_text(&content);
        let request = Tokio::spawn(cx, async move {
            session
                .create_forum_post(forum_id, &name, &content, &tags)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                if let Some(header) = &mut app.forum_header {
                    header.posting = false;
                }
                match result {
                    Ok(Ok(thread)) => {
                        let id = thread.id;
                        let search = model::ForumSearch {
                            threads: vec![thread],
                            ..Default::default()
                        };
                        if matches!(
                            app.store.apply_forum_search(forum_id, search),
                            Change::Structure
                        ) {
                            app.sync_channel_rows();
                        }
                        app.cancel_forum_post(window, cx);
                        app.open_thread(id, window, cx);
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not create forum post: {error}");
                        if let Some(header) = &mut app.forum_header {
                            header.error = Some(post_error(&error));
                        }
                    }
                    Err(error) => log::warn!("Forum post task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// Discord's own reason when it gave one, since rejections (missing tag, permissions,
/// slowmode) are otherwise indistinguishable.
fn post_error(error: &crate::discord::Error) -> String {
    match error {
        crate::discord::Error::Rejected { message, .. } if !message.is_empty() => message.clone(),
        crate::discord::Error::RateLimited { retry_after } => format!(
            "You're posting too fast. Try again in {}s.",
            retry_after.as_secs().max(1)
        ),
        _ => "Couldn't create the post.".to_owned(),
    }
}
