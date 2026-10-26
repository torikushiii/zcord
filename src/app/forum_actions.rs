use super::*;

impl DiscordApp {
    pub(crate) fn open_thread(
        &mut self,
        thread_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_channel(thread_id, window, cx);
    }

    pub(crate) fn back_to_forum(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(forum_id) = self.thread_parent() {
            self.select_channel(forum_id, window, cx);
        }
    }

    pub(crate) fn forum_loading(&self) -> bool {
        self.selected_forum()
            .and_then(|forum_id| self.forum_pages.get(&forum_id))
            .is_some_and(|page| page.loading)
    }

    pub(crate) fn forum_has_more(&self) -> bool {
        self.selected_forum()
            .and_then(|forum_id| self.forum_pages.get(&forum_id))
            .is_some_and(|page| page.has_more)
    }

    pub(crate) fn load_forum(&mut self, forum_id: u64, cx: &mut Context<Self>) {
        self.forum_pages.entry(forum_id).or_default();
        let page = self.forum_pages.get(&forum_id).expect("forum page");
        if page.loaded || page.loading {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            self.forum_pages.entry(forum_id).or_default().loaded = true;
            return;
        };
        self.forum_pages.entry(forum_id).or_default().loading = true;
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session.fetch_forum_posts(Some(guild_id), forum_id, 0).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = app.forum_pages.entry(forum_id).or_default();
                page.loading = false;
                match result {
                    Ok(Ok(search)) => {
                        page.loaded = true;
                        page.offset = search.threads.len();
                        page.has_more = search.has_more;
                        let change = app.store.apply_forum_search(forum_id, search);
                        if matches!(change, Change::Structure) {
                            app.sync_channel_rows();
                        }
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
        let offset = page.offset;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.forum_pages.entry(forum_id).or_default().loading = true;
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_forum_posts(Some(guild_id), forum_id, offset)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = app.forum_pages.entry(forum_id).or_default();
                page.loading = false;
                match result {
                    Ok(Ok(search)) => {
                        page.offset = offset + search.threads.len();
                        page.has_more = search.has_more;
                        let change = app.store.apply_forum_search(forum_id, search);
                        if matches!(change, Change::Structure) {
                            app.sync_channel_rows();
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load more forum posts: {error}"),
                    Err(error) => log::warn!("Forum task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
