use super::decoder::Core;
use super::sources::{SourceKind, SpoolJob, spool_pool};
use super::*;

pub(super) const MAX_STALE_SKIPS: u8 = 16;

pub(super) enum Command {
    Release {
        keys: Vec<u64>,
        reply: tokio::sync::oneshot::Sender<()>,
    },
    First {
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        stamp: Arc<Mutex<Instant>>,
        reply: tokio::sync::oneshot::Sender<Result<Option<Arc<RenderImage>>>>,
    },
    Advance {
        key: u64,
        generation: u64,
        plays: Option<NonZeroU32>,
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        spool: Option<SpoolTarget>,
        shared: Arc<Mutex<Demand>>,
    },
}

pub(super) struct Pool {
    pub(super) workers: Vec<mpsc::Sender<Command>>,
    next: AtomicUsize,
}

impl Pool {
    pub(super) fn send(&self, key: usize, command: Command) -> Result<()> {
        self.workers[key % self.workers.len()]
            .send(command)
            .map_err(|_| anyhow::anyhow!("animation worker unavailable"))
    }
}

pub(super) fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let mut workers = Vec::new();
        for index in 0..WORKERS {
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name(format!("media-stream-{index}"))
                .spawn(move || worker(receiver));
            if spawned.is_err() {
                log::warn!("Could not start animation worker {index}");
            }
            workers.push(sender);
        }
        Pool {
            workers,
            next: AtomicUsize::new(0),
        }
    })
}

pub(crate) async fn first(
    path: PathBuf,
    size: u32,
    stamp: Arc<Mutex<Instant>>,
) -> Result<Option<Arc<RenderImage>>> {
    first_source(path, size, stamp, SourceKind::Image).await
}

#[cfg(windows)]
pub(crate) async fn first_video(
    path: PathBuf,
    size: u32,
    max_fps: u32,
    stamp: Arc<Mutex<Instant>>,
) -> Result<Option<Arc<RenderImage>>> {
    first_source(path, size, stamp, SourceKind::Video { max_fps }).await
}

async fn first_source(
    path: PathBuf,
    size: u32,
    stamp: Arc<Mutex<Instant>>,
    kind: SourceKind,
) -> Result<Option<Arc<RenderImage>>> {
    let (reply, receiver) = tokio::sync::oneshot::channel();
    let pool = pool();
    pool.send(
        pool.next.fetch_add(1, Ordering::Relaxed),
        Command::First {
            path,
            size,
            kind,
            stamp,
            reply,
        },
    )?;
    receiver.await.context("animation worker stopped")?
}

pub(super) struct Session {
    pub(super) core: Core,
    generation: u64,
    shared: Weak<Mutex<Demand>>,
}

pub(super) fn worker(receiver: mpsc::Receiver<Command>) {
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    loop {
        sweep_sessions(&mut sessions, Instant::now());
        let command = match receiver.recv_timeout(SWEEP) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match command {
            Command::Release { keys, reply } => {
                for key in keys {
                    sessions.remove(&key);
                }
                let _ = reply.send(());
            }
            Command::First {
                path,
                size,
                kind,
                stamp,
                reply,
            } => {
                let result = if still_wanted(&stamp) {
                    Core::open_kind(path, size, kind, None).and_then(|mut core| {
                        let frame = core.step(Duration::ZERO, || still_wanted(&stamp))?;
                        if !still_wanted(&stamp) {
                            return Ok(None);
                        }
                        Ok(Some(frame.context("animation has no valid first frame")?.0))
                    })
                } else {
                    Ok(None)
                };
                let _ = reply.send(result);
            }
            Command::Advance {
                key,
                generation,
                plays,
                path,
                size,
                kind,
                spool,
                shared,
            } => {
                advance(
                    &mut sessions,
                    key,
                    generation,
                    plays,
                    path,
                    size,
                    kind,
                    spool,
                    &shared,
                );
            }
        }
    }
}

pub(super) fn sweep_sessions(sessions: &mut HashMap<u64, Session>, now: Instant) {
    sessions.retain(|_, session| {
        session.shared.upgrade().is_some_and(|shared| {
            let shared = shared.lock().unwrap_or_else(|p| p.into_inner());
            shared.generation == session.generation && !shared.idle(now)
        })
    });
}

pub(super) fn advance(
    sessions: &mut HashMap<u64, Session>,
    id: u64,
    generation: u64,
    plays: Option<NonZeroU32>,
    path: PathBuf,
    size: u32,
    kind: SourceKind,
    spool: Option<SpoolTarget>,
    shared: &Arc<Mutex<Demand>>,
) {
    let (target, slot) = {
        let mut demand = shared.lock().unwrap_or_else(|p| p.into_inner());
        if generation != demand.generation {
            return;
        }
        if !demand.queued
            || demand.terminal
            || !demand.wanted(Instant::now())
            || demand.ready.is_some()
        {
            demand.queued = false;
            return;
        }
        let Some(slot) = Slot::reserve(&demand.slots) else {
            demand.queued = false;
            return;
        };
        (demand.target, slot)
    };
    let result = (|| {
        if sessions
            .get(&id)
            .is_some_and(|session| session.generation != generation)
        {
            sessions.remove(&id);
        }
        if let std::collections::hash_map::Entry::Vacant(e) = sessions.entry(id) {
            e.insert(Session {
                core: Core::open_kind(
                    path.clone(),
                    size,
                    kind,
                    spool.as_ref().map(|spool| spool.file.clone()),
                )?
                .with_plays(plays),
                generation,
                shared: Arc::downgrade(shared),
            });
        }
        let session = sessions.get_mut(&id).context("missing animation session")?;
        let frame = session.core.step(target, || {
            let demand = shared.lock().unwrap_or_else(|p| p.into_inner());
            demand.generation == generation && demand.wanted(Instant::now())
        })?;
        let spool_file = session.core.wants_spool();
        Ok::<_, anyhow::Error>((frame, session.core.terminal, spool_file))
    })();
    let mut demand = shared.lock().unwrap_or_else(|p| p.into_inner());
    if generation != demand.generation {
        sessions.remove(&id);
        return;
    }
    demand.queued = false;
    match result {
        Ok((frame, terminal, spool_file)) => {
            if let Some(file) = spool_file
                && spool_pool()
                    .send(SpoolJob {
                        source: path,
                        size,
                        file,
                        service: spool.and_then(|spool| spool.service),
                        shared: Arc::downgrade(shared),
                    })
                    .is_err()
            {
                log::debug!("Could not queue GIF spooling");
            }
            demand.terminal = terminal;
            if terminal {
                sessions.remove(&id);
            }
            let stale = !terminal
                && demand.follows_clock
                && demand.stale_skips < MAX_STALE_SKIPS
                && frame.as_ref().is_some_and(|(_, _, end)| *end <= target);
            if stale {
                demand.stale_skips += 1;
            } else if demand.wanted(Instant::now()) {
                demand.stale_skips = 0;
                demand.ready = frame.map(|(image, start, end)| Prepared {
                    image,
                    start,
                    end,
                    slot,
                });
            }
        }
        Err(_) => {
            demand.terminal = true;
            sessions.remove(&id);
        }
    }
}
