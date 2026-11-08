use super::*;

/// A just-decoded image may still be waiting for its debounced repaint.
const FRESH_FOR: Duration = Duration::from_secs(1);
/// Must stay under `STALE_AFTER`, or jobs for visible tiles get cancelled between repaints.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

impl Cache {
    /// The size `spec` is served at for a request of `size`. A profile effect without a ready
    /// decode at that size takes the smallest ready larger one, so cards of different sizes
    /// share one copy instead of holding one each.
    pub(super) fn serving_size(&self, spec: &ImageSpec, size: u32) -> u32 {
        let Some(sizes) = self.entries.get(spec) else {
            return size;
        };
        if !matches!(spec, ImageSpec::Collectible { .. })
            || matches!(sizes.get(&size), Some(Entry::Ready { .. }))
        {
            return size;
        }
        sizes
            .iter()
            .filter(|&(&ready, entry)| ready > size && matches!(entry, Entry::Ready { .. }))
            .map(|(&ready, _)| ready)
            .min()
            .unwrap_or(size)
    }

    /// Drops animations not shown for `UNSEEN_DROP`; also says whether any are left to check
    /// again later.
    pub(super) fn purge_unseen(&mut self) -> (Vec<Arc<RenderImage>>, bool) {
        let mut purged = Vec::new();
        let mut remaining = false;
        let mut freed = 0;
        self.entries.retain(|spec, sizes| {
            sizes.retain(|_, entry| match entry {
                Entry::Ready {
                    image,
                    bytes,
                    ready_at,
                    seen,
                    ..
                } if Arc::strong_count(image) == 1 + usize::from(stream::selected(spec))
                    && ready_at.elapsed() >= FRESH_FOR
                    && seen.elapsed() >= UNSEEN_DROP =>
                {
                    freed += *bytes;
                    purged.push(image.clone());
                    false
                }
                Entry::Ready { .. } => {
                    remaining = true;
                    true
                }
                _ => true,
            });
            !sizes.is_empty()
        });
        self.bytes -= freed;
        (purged, remaining)
    }

    pub(super) fn request(&mut self, spec: &ImageSpec, size: u32, now: Instant) {
        let mut found = false;
        let mut promote = false;
        if let Some(entry) = self
            .entries
            .get_mut(spec)
            .and_then(|sizes| sizes.get_mut(&size))
        {
            found = true;
            if let Entry::Pending {
                requested,
                queued,
                wanted,
            } = entry
            {
                *requested = now;
                *wanted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = now;
                promote = *queued;
            }
        }
        if found {
            if promote {
                self.promote(spec, size);
            }
            return;
        }
        let spec = spec.clone();
        self.entries.entry(spec.clone()).or_default().insert(
            size,
            Entry::Pending {
                requested: now,
                queued: true,
                wanted: Arc::new(Mutex::new(now)),
            },
        );
        self.queue.insert(0, (spec, size));
    }

    fn promote(&mut self, spec: &ImageSpec, size: u32) {
        if let Some(position) = self
            .queue
            .iter()
            .position(|(queued_spec, queued_size)| queued_spec == spec && *queued_size == size)
        {
            let key = self.queue.remove(position);
            self.queue.insert(0, key);
        }
    }

    pub(super) fn next(&mut self) -> (Option<Key>, usize, Vec<Arc<RenderImage>>) {
        let mut dropped = 0;
        let mut index = 0;
        let selected = loop {
            if index >= self.queue.len() {
                break None;
            }
            let key = self.queue[index].clone();
            let lane = Lane::of(&key.0);
            let fresh = matches!(
                self.entries
                    .get(&key.0)
                    .and_then(|sizes| sizes.get(&key.1)),
                Some(Entry::Pending { requested, .. }) if requested.elapsed() <= STALE_AFTER
            );
            if fresh {
                if self.running[lane as usize] < lane.downloads() {
                    break Some((index, key, lane));
                }
                // Another lane may still have room; a full GIF lane must not block emoji.
                index += 1;
                continue;
            }
            self.queue.remove(index);
            let pending = matches!(
                self.entries.get(&key.0).and_then(|sizes| sizes.get(&key.1)),
                Some(Entry::Pending { .. })
            );
            if pending {
                let empty = if let Some(sizes) = self.entries.get_mut(&key.0) {
                    sizes.remove(&key.1);
                    sizes.is_empty()
                } else {
                    false
                };
                if empty {
                    self.entries.remove(&key.0);
                }
            }
            dropped += 1;
        };
        let Some((index, key, lane)) = selected else {
            return (None, dropped, Vec::new());
        };
        let evicted = self.evict();
        self.queue.remove(index);
        if let Some(Entry::Pending { queued, .. }) = self
            .entries
            .get_mut(&key.0)
            .and_then(|sizes| sizes.get_mut(&key.1))
        {
            *queued = false;
        }
        self.running[lane as usize] += 1;
        (Some(key), dropped, evicted)
    }

    pub(super) fn wanted_stamp(&self, key: &Key) -> Option<Arc<Mutex<Instant>>> {
        match self.entries.get(&key.0)?.get(&key.1)? {
            Entry::Pending { wanted, .. } => Some(wanted.clone()),
            _ => None,
        }
    }

    pub(super) fn needs_heartbeat(&self) -> bool {
        self.running.iter().any(|&running| running > 0) || !self.queue.is_empty()
    }

    pub(super) fn evict(&mut self) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        // The budget is soft: reserve one future decode, and if only fresh or pinned entries
        // are left, start anyway rather than stalling.
        while self.bytes + MAX_IMAGE > MAX_TOTAL {
            let candidate = self
                .entries
                .iter()
                .flat_map(|(spec, sizes)| {
                    sizes.iter().map(move |(size, entry)| (spec, size, entry))
                })
                .filter_map(|(spec, size, entry)| match entry {
                    Entry::Ready {
                        image,
                        used,
                        ready_at,
                        seen,
                        ..
                    } if Arc::strong_count(image) == 1 + usize::from(stream::selected(spec))
                        && ready_at.elapsed() >= FRESH_FOR
                        && seen.elapsed() >= VISIBLE_GRACE =>
                    {
                        Some((spec, size, *used))
                    }
                    _ => None,
                })
                .min_by_key(|(_, _, used)| *used)
                .map(|(spec, size, _)| (spec.clone(), *size));
            let Some((spec, size)) = candidate else {
                break;
            };
            let empty = if let Some(sizes) = self.entries.get_mut(&spec) {
                if let Some(Entry::Ready { image, bytes, .. }) = sizes.remove(&size) {
                    self.bytes -= bytes;
                    evicted.push(image);
                }
                sizes.is_empty()
            } else {
                false
            };
            if empty {
                self.entries.remove(&spec);
            }
        }
        evicted
    }
}

pub(super) fn schedule_refresh(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.refresh_pending {
            return;
        }
        state.refresh_pending = true;
    }
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
        cache.borrow_mut().refresh_pending = false;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

pub(crate) fn drop_images(images: Vec<Arc<RenderImage>>, cx: &App) {
    if images.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            // gpui's sprite atlas keeps every frame until drop_image is called.
            for image in images {
                if !stream::forget(image.id, cx) {
                    cx.drop_image(image, None);
                }
            }
        });
    })
    .detach();
}

/// Checks for unseen animations every `UNSEEN_DROP` while any are cached.
pub(super) fn schedule_purge(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.purge_scheduled {
            return;
        }
        state.purge_scheduled = true;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(UNSEEN_DROP).await;
            let (purged, remaining) = {
                let mut state = cache.borrow_mut();
                let (purged, remaining) = state.purge_unseen();
                state.purge_scheduled = remaining;
                (purged, remaining)
            };
            cx.update(|cx| drop_images(purged, cx));
            if !remaining {
                break;
            }
        }
    })
    .detach();
}

/// Repaint while work is in flight so visible still fallbacks keep refreshing their stamps.
pub(super) fn start_heartbeat(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.heartbeat_running || !state.needs_heartbeat() {
            return;
        }
        state.heartbeat_running = true;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(HEARTBEAT_INTERVAL).await;
            let busy = {
                let mut state = cache.borrow_mut();
                if state.needs_heartbeat() {
                    true
                } else {
                    state.heartbeat_running = false;
                    false
                }
            };
            if !busy {
                break;
            }
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}
