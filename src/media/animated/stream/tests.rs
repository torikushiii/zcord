use super::*;
use super::{decoder::*, sources::*, worker::*};
use gpui::{Context, Entity, Render, TestAppContext, div, prelude::*};
use image::{
    Rgba, RgbaImage,
    codecs::gif::{GifEncoder, Repeat},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "zcord-stream-{}-{}.gif",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn gif(count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for index in 0..count {
            let delay = [10, 20, 30, 100][index % 4];
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(8, 4, Rgba([index as u8, 20, 200, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    bytes
}

fn gif_colored(count: usize, color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for _ in 0..count {
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(8, 4, Rgba(color)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
    }
    bytes
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(extension: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "zcord-spool-{}-{}.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("spool.tmp"));
    }
}

fn spec() -> ImageSpec {
    ImageSpec::Media {
        url: "https://media.tenor.com/test.gif".into(),
        width: 8,
        height: 4,
        animated: true,
    }
}

fn still() -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![process_frame(
        Frame::new(RgbaImage::new(8, 4)),
        (8, 4),
    )]))
}

fn playback(path: PathBuf, retired: &Retirements) -> Playback {
    Playback {
        key: next_key(),
        spec: spec(),
        kind: SourceKind::Image,
        path,
        spool: None,
        size: 8,
        still: Rc::new(FrameImage {
            image: still(),
            slot: None,
            retired: retired.clone(),
        }),
        current: None,
        end: Duration::from_millis(100),
        started: Some(Instant::now()),
        clock: None,
        plays: None,
        shared: Arc::new(Mutex::new(Demand::new(Instant::now()))),
    }
}

fn request(playback: &Playback, sessions: &mut HashMap<u64, Session>, target: Duration) {
    let generation = {
        let mut shared = playback.shared.lock().unwrap();
        shared.queued = true;
        shared.target = target;
        shared.requested = Instant::now();
        shared.generation
    };
    advance(
        sessions,
        playback.key,
        generation,
        playback.plays,
        playback.path.clone(),
        playback.size,
        playback.kind.clone(),
        playback.spool.clone(),
        &playback.shared,
    );
}

fn adopt(playback: &mut Playback, retired: &Retirements) {
    let prepared = playback.shared.lock().unwrap().ready.take().unwrap();
    playback.end = prepared.end;
    playback.current = Some(Rc::new(FrameImage {
        image: prepared.image,
        slot: Some(prepared.slot),
        retired: retired.clone(),
    }));
}

#[derive(Default)]
struct VideoStats {
    opened: AtomicUsize,
    closed: AtomicUsize,
    reads: AtomicUsize,
    copied: AtomicUsize,
    loops: AtomicUsize,
    opened_on: Mutex<Option<std::thread::ThreadId>>,
    closed_on: Mutex<Option<std::thread::ThreadId>>,
}

struct FakeVideo {
    index: usize,
    empty: bool,
    count: usize,
    error_at: Option<usize>,
    fail_restart: bool,
    ticks: i64,
    stats: Arc<VideoStats>,
}

impl Drop for FakeVideo {
    fn drop(&mut self) {
        self.stats.closed.fetch_add(1, Ordering::Relaxed);
        *self.stats.closed_on.lock().unwrap() = Some(std::thread::current().id());
    }
}

impl crate::media::video::animation::VideoReader for FakeVideo {
    fn read(&mut self, pixels: bool) -> Result<crate::media::video::animation::VideoRead> {
        use crate::media::video::animation::VideoRead;
        self.stats.reads.fetch_add(1, Ordering::Relaxed);
        if self.empty {
            self.empty = false;
            return Ok(VideoRead::Empty);
        }
        ensure!(Some(self.index) != self.error_at, "fake video decode error");
        if self.index >= self.count {
            return Ok(VideoRead::End);
        }
        let image = pixels.then(|| {
            self.stats.copied.fetch_add(1, Ordering::Relaxed);
            RgbaImage::from_pixel(8, 4, Rgba([self.index as u8, 11, 22, 255]))
        });
        let timestamp = self.index as i64 * self.ticks;
        self.index += 1;
        Ok(VideoRead::Sample {
            timestamp,
            duration: Some(self.ticks),
            pixels: image,
        })
    }

    fn restart(&mut self) -> Result<()> {
        ensure!(!self.fail_restart, "fake video reopen error");
        self.stats.loops.fetch_add(1, Ordering::Relaxed);
        self.index = 0;
        self.empty = true;
        Ok(())
    }
}

fn fake_video(
    count: usize,
    error_at: Option<usize>,
    rate: Option<(u32, u32)>,
    fail_restart: bool,
) -> (SourceKind, Arc<VideoStats>) {
    let stats = Arc::new(VideoStats::default());
    let captured = stats.clone();
    let kind = SourceKind::Fake(Arc::new(move || {
        captured.opened.fetch_add(1, Ordering::Relaxed);
        *captured.opened_on.lock().unwrap() = Some(std::thread::current().id());
        let reader = FakeVideo {
            index: 0,
            empty: true,
            count,
            error_at,
            fail_restart,
            ticks: rate.map_or(333_333, |(n, d)| {
                (10_000_000u64 * u64::from(d) / u64::from(n)).max(1) as i64
            }),
            stats: captured.clone(),
        };
        Ok(Box::new(crate::media::video::animation::VideoFrames::new(
            reader,
            rate,
            crate::media::video::animation::MAX_FPS,
        )))
    }));
    (kind, stats)
}

#[test]
fn video_source_loops_with_stretched_delays_and_skips_ahead_without_gif_delay_rules() {
    let (kind, stats) = fake_video(6, None, Some((30, 1)), false);
    let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
    let interval = Duration::from_nanos(66_666_600);
    let mut target = Duration::ZERO;
    for index in [0, 2, 4, 0, 2, 4] {
        let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
        assert_eq!(start, target);
        assert_eq!(end - start, interval);
        assert_eq!(Duration::from(frame.delay(0)), interval);
        assert_eq!(&frame.as_bytes(0).unwrap()[..4], &[index, 11, 22, 255]);
        target = end;
    }
    assert_eq!(stats.copied.load(Ordering::Relaxed), 6);
    assert_eq!(stats.loops.load(Ordering::Relaxed), 1);
    let far = Duration::from_secs(3600);
    let mut caught_up = false;
    for _ in 0..8 {
        let before = core.decoded;
        if let Some((_, start, end)) = core.step(far, || true).unwrap() {
            assert_eq!(end - start, interval);
            caught_up = end > far;
        }
        assert!(core.decoded - before <= DECODE_SLICE);
        if caught_up {
            break;
        }
    }
    assert!(caught_up);
}

#[test]
fn video_uses_shared_frame_slots_and_never_reads_ahead_of_the_bound() {
    let (kind, stats) = fake_video(1000, None, Some((30, 1)), false);
    let retired = Rc::default();
    let mut playback = playback(PathBuf::new(), &retired);
    playback.kind = kind;
    playback.end = Duration::ZERO;
    let _lease = DisplayLease::new(&playback.shared, true);
    let slots = playback.shared.lock().unwrap().slots.clone();
    let mut sessions = HashMap::new();
    let mut released = std::collections::HashSet::new();
    let mut handed = std::collections::HashSet::new();
    for _ in 0..650 {
        request(&playback, &mut sessions, playback.end);
        assert!(playback.shared.lock().unwrap().ready.is_some());
        let reads = stats.reads.load(Ordering::Relaxed);
        request(&playback, &mut sessions, playback.end);
        assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
        let old_scene = playback.current.clone();
        adopt(&mut playback, &retired);
        handed.insert(playback.current.as_ref().unwrap().image.id);
        assert!(slots.load(Ordering::Acquire) <= PLAYBACK_FRAMES);
        if old_scene.is_some() {
            request(&playback, &mut sessions, playback.end);
            assert!(playback.shared.lock().unwrap().ready.is_some());
            let reads = stats.reads.load(Ordering::Relaxed);
            request(&playback, &mut sessions, playback.end);
            assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
        }
        drop(old_scene);
        release_with(&retired, |image| assert!(released.insert(image.id)));
    }
    playback.shared.lock().unwrap().ready = None;
    playback.current = None;
    release_with(&retired, |image| assert!(released.insert(image.id)));
    assert_eq!(slots.load(Ordering::Acquire), 0);
    assert_eq!(released, handed);
    assert!(stats.loops.load(Ordering::Relaxed) >= 2);
}

#[test]
fn video_end_error_and_failed_reopen_keep_the_last_good_frame() {
    for (count, error, reopen_error, expected) in [
        (1, None, false, 0),
        (6, Some(3), false, 2),
        (6, None, true, 4),
    ] {
        let (kind, stats) = fake_video(count, error, Some((30, 1)), reopen_error);
        let retired = Rc::default();
        let mut playback = playback(PathBuf::new(), &retired);
        playback.kind = kind;
        playback.end = Duration::ZERO;
        let _lease = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        for _ in 0..8 {
            request(&playback, &mut sessions, playback.end);
            if playback.shared.lock().unwrap().ready.is_some() {
                adopt(&mut playback, &retired);
                release_with(&retired, |_| {});
            }
            if playback.shared.lock().unwrap().terminal {
                break;
            }
        }
        assert!(playback.shared.lock().unwrap().terminal);
        let last = playback.current.as_ref().unwrap().image.id;
        assert_eq!(
            playback
                .current
                .as_ref()
                .unwrap()
                .image
                .as_bytes(0)
                .unwrap()[0],
            expected
        );
        request(&playback, &mut sessions, playback.end);
        assert_eq!(playback.current.as_ref().unwrap().image.id, last);
        assert!(sessions.is_empty());
        assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
    }
    let (kind, _) = fake_video(0, None, Some((30, 1)), false);
    let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
    assert!(core.step(Duration::ZERO, || true).unwrap().is_none());
    assert!(core.terminal);
}

#[test]
fn video_idle_drop_reopens_the_source_and_inactive_scenes_remain_frozen() {
    let (kind, stats) = fake_video(100, None, Some((30, 1)), false);
    let retired = Rc::default();
    let mut playback = playback(PathBuf::new(), &retired);
    playback.kind = kind;
    playback.end = Duration::ZERO;
    let still_id = playback.still.image.id;
    let active = DisplayLease::new(&playback.shared, true);
    let mut sessions = HashMap::new();
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    let frame_id = playback.current.as_ref().unwrap().image.id;
    let inactive = DisplayLease::new(&playback.shared, false);
    drop(active);
    let reads = stats.reads.load(Ordering::Relaxed);
    request(&playback, &mut sessions, playback.end);
    assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
    assert!(!playback.expire(Instant::now() + IDLE * 10));
    assert_eq!(playback.current.as_ref().unwrap().image.id, frame_id);
    drop(inactive);
    let absent = playback.shared.lock().unwrap().absent_since;
    assert!(playback.expire(absent + IDLE));
    sweep_sessions(&mut sessions, absent + IDLE);
    release_with(&retired, |_| {});
    assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
    assert!(playback.current.is_none());
    assert_eq!(playback.still.image.id, still_id);
    let _active = DisplayLease::new(&playback.shared, true);
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    assert_eq!(stats.opened.load(Ordering::Relaxed), 2);
    assert_eq!(
        playback
            .current
            .as_ref()
            .unwrap()
            .image
            .as_bytes(0)
            .unwrap()[0],
        2
    );
}

#[test]
fn video_unknown_rate_uses_timestamps_to_cap_conversion_and_uploads() {
    let (kind, stats) = fake_video(6, None, None, false);
    let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
    let (frame, start, end) = core.step(Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(start, Duration::ZERO);
    assert_eq!(end, Duration::from_nanos(99_999_900));
    assert_eq!(frame.as_bytes(0).unwrap()[0], 0);
    assert_eq!(stats.reads.load(Ordering::Relaxed), 4);
    assert_eq!(stats.copied.load(Ordering::Relaxed), 1);
}

#[test]
fn video_fps_cap_retains_the_existing_rate_based_skip_intervals() {
    for (rate, samples, nanos) in [
        ((30, 1), 2, 66_666_600),
        ((60, 1), 4, 66_666_400),
        ((24, 1), 2, 83_333_200),
        ((16, 1), 2, 125_000_000),
        ((30_000, 1001), 2, 66_733_200),
    ] {
        let (kind, stats) = fake_video(100, None, Some(rate), false);
        let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
        let (_, start, end) = core.step(Duration::ZERO, || true).unwrap().unwrap();
        assert_eq!(end - start, Duration::from_nanos(nanos));
        assert_eq!(stats.reads.load(Ordering::Relaxed), samples + 1);
        assert_eq!(stats.copied.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn video_shutdown_acknowledges_decoder_destruction_on_its_own_worker() {
    let (kind, stats) = fake_video(100, None, Some((30, 1)), false);
    let id = still().id;
    let shared = Arc::new(Mutex::new(Demand::new(Instant::now())));
    let _lease = DisplayLease::new(&shared, true);
    shared.lock().unwrap().queued = true;
    pool()
        .send(
            id.0,
            Command::Advance {
                key: id.0 as u64,
                generation: 0,
                plays: None,
                path: PathBuf::new(),
                size: 8,
                kind,
                spool: None,
                shared: shared.clone(),
            },
        )
        .unwrap();
    let (reply, receiver) = tokio::sync::oneshot::channel();
    pool()
        .send(
            id.0,
            Command::Release {
                keys: vec![id.0 as u64],
                reply,
            },
        )
        .unwrap();
    receiver.blocking_recv().unwrap();
    assert_eq!(stats.opened.load(Ordering::Relaxed), 1);
    assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
    assert_eq!(
        *stats.opened_on.lock().unwrap(),
        *stats.closed_on.lock().unwrap()
    );
    assert_ne!(
        *stats.closed_on.lock().unwrap(),
        Some(std::thread::current().id())
    );
}

#[test]
fn streamed_frames_match_eager_order_pixels_and_browser_delays() {
    let bytes = gif(100);
    let fixture = Fixture::new(&bytes);
    let eager = decode(&bytes, 8).unwrap().0;
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    let mut target = Duration::ZERO;
    for index in 0..100 {
        let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
        assert_eq!(frame.frame_count(), 1);
        assert_eq!(frame.size(0), eager.size(index));
        assert_eq!(frame.as_bytes(0), eager.as_bytes(index));
        assert_eq!(frame.delay(0), eager.delay(index));
        assert_eq!(start, target);
        assert_eq!(end - start, Duration::from(eager.delay(index)));
        target = end;
    }
    let (frame, start, _) = core.step(target, || true).unwrap().unwrap();
    assert_eq!(start, target);
    assert_eq!(frame.as_bytes(0), eager.as_bytes(0));
}

#[test]
fn a_finite_play_count_holds_the_last_frame() {
    let bytes = gif(4);
    let fixture = Fixture::new(&bytes);
    let eager = decode(&bytes, 8).unwrap().0;
    let mut core = Core::open(fixture.0.clone(), 8)
        .unwrap()
        .with_plays(NonZeroU32::new(1));
    let far = Duration::from_secs(10);
    let mut last = None;
    while !core.terminal {
        if let Some((frame, _, _)) = core.step(far, || true).unwrap() {
            last = Some(frame);
        }
    }
    assert_eq!(last.unwrap().as_bytes(0), eager.as_bytes(3));
    assert!(core.step(far, || true).unwrap().is_none());
}

#[test]
fn a_clocked_playback_holds_back_frames_already_past_while_catching_up() {
    let fixture = Fixture::new(&gif(100));
    let retired = Retirements::default();
    let registered = playback(fixture.0.clone(), &retired);
    let both = [
        registered.clocked(Instant::now(), None),
        playback(fixture.0.clone(), &retired),
    ];
    let target = Duration::from_secs(5);
    for playback in &both {
        let _lease = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        request(playback, &mut sessions, target);
        let shared = playback.shared.lock().unwrap();
        if shared.follows_clock {
            assert!(shared.ready.is_none());
            assert_eq!(shared.stale_skips, 1);
        } else {
            assert!(shared.ready.as_ref().unwrap().end <= target);
        }
    }
    let clocked = &both[0];
    let _lease = DisplayLease::new(&clocked.shared, true);
    let mut sessions = HashMap::new();
    for _ in 0..MAX_STALE_SKIPS {
        if clocked.shared.lock().unwrap().ready.is_some() {
            break;
        }
        request(clocked, &mut sessions, target);
    }
    let shared = clocked.shared.lock().unwrap();
    assert!(shared.ready.as_ref().unwrap().end > target);
    assert_eq!(shared.stale_skips, 0);
}

#[test]
fn each_caller_clock_gets_its_own_playback() {
    let fixture = Fixture::new(&gif(4));
    let retired = Retirements::default();
    let registered = playback(fixture.0.clone(), &retired);
    let start = Instant::now();
    let first = registered.clocked(start, NonZeroU32::new(1));
    let second = registered.clocked(start + Duration::from_secs(1), None);
    assert_ne!(first.key, second.key);
    assert_ne!(first.key, registered.key);
    assert!(!Arc::ptr_eq(&first.shared, &second.shared));
    assert_eq!(first.plays, NonZeroU32::new(1));
    let mut first = first;
    assert_eq!(
        first.resume(start + Duration::from_millis(250)),
        Duration::from_millis(250)
    );
}

struct SlowFrames {
    reads: usize,
}

impl FrameSource for SlowFrames {
    fn read(&mut self) -> Result<SourceRead> {
        std::thread::sleep(Duration::from_millis(5));
        self.reads += 1;
        Ok(SourceRead::Frame(Frame::from_parts(
            RgbaImage::from_pixel(8, 4, Rgba([self.reads as u8, 0, 0, 255])),
            0,
            0,
            Delay::from_numer_denom_ms(1, 1),
        )))
    }

    fn restart(&mut self) -> Result<()> {
        Ok(())
    }

    fn max_frames(&self) -> usize {
        MAX_FRAMES
    }
}

#[test]
fn slow_sources_skip_frames_within_a_single_step() {
    let mut core = Core::open_kind(
        PathBuf::new(),
        8,
        SourceKind::Fake(Arc::new(|| Ok(Box::new(SlowFrames { reads: 0 })))),
        None,
    )
    .unwrap();
    let (_, start, end) = core
        .step(Duration::from_secs(60), || true)
        .unwrap()
        .unwrap();
    assert!(core.decoded > 1 && core.decoded <= DECODE_SLICE);
    assert!(start > Duration::ZERO);
    assert!(end > start);
}

#[test]
fn spooled_frames_round_trip_pixels_delays_and_restart() {
    let bytes = gif(7);
    let fixture = Fixture::new(&bytes);
    let spool = TempFile::new("spool");
    write_spool(&fixture.0, 8, &spool.0, || true).unwrap();
    let eager = decode(&bytes, 8).unwrap().0;
    let mut source = SpoolFrames::open(&spool.0, (8, 4)).unwrap();
    for index in 0..eager.frame_count() {
        let SourceRead::Frame(frame) = source.read().unwrap() else {
            panic!("spool ended after {index} frames");
        };
        assert_eq!(
            frame.buffer().as_raw().as_slice(),
            eager.as_bytes(index).unwrap()
        );
        assert_eq!(
            frame.delay().numer_denom_ms(),
            eager.delay(index).numer_denom_ms()
        );
    }
    assert!(matches!(source.read().unwrap(), SourceRead::End));
    source.restart().unwrap();
    let SourceRead::Frame(frame) = source.read().unwrap() else {
        panic!("spool did not restart");
    };
    assert_eq!(
        frame.buffer().as_raw().as_slice(),
        eager.as_bytes(0).unwrap()
    );
}

#[test]
fn cancelled_spooling_leaves_no_files_behind() {
    let fixture = Fixture::new(&gif(5));
    let spool = TempFile::new("spool");
    assert!(write_spool(&fixture.0, 8, &spool.0, || false).is_err());
    assert!(!spool.0.exists());
    assert!(!spool.0.with_extension("spool.tmp").exists());
}

#[test]
fn corrupt_spool_files_are_deleted_and_the_source_still_plays() {
    let bytes = gif(3);
    let fixture = Fixture::new(&bytes);
    let spool = TempFile::new("spool");
    std::fs::write(&spool.0, b"not a spool").unwrap();
    let mut core = Core::open_kind(
        fixture.0.clone(),
        8,
        SourceKind::Image,
        Some(spool.0.clone()),
    )
    .unwrap();
    assert!(!spool.0.exists());
    let (frame, _, _) = core.step(Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(frame.as_bytes(0), decode(&bytes, 8).unwrap().0.as_bytes(0));
}

#[test]
fn each_loop_measures_decode_time_from_zero() {
    let fixture = Fixture::new(&gif(2));
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    core.spent = Duration::from_secs(3600);
    core.step(Duration::from_secs(1), || true).unwrap();
    assert!(core.spent < Duration::from_secs(3600));
}

#[test]
fn wants_spool_waits_for_the_first_pass_to_fall_behind() {
    let fixture = Fixture::new(&gif(10));
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    let spool = TempFile::new("spool");
    core.spool = Some(spool.0.clone());
    core.count = 3;
    core.spent = Duration::from_millis(2);
    assert!(core.wants_spool().is_none());
    core.count = 4;
    core.position = Duration::from_millis(1);
    core.spent = Duration::from_millis(1);
    assert!(core.wants_spool().is_none());
    core.spent = Duration::from_millis(2);
    assert_eq!(core.wants_spool(), Some(spool.0.clone()));
    assert!(core.wants_spool().is_none());
}

#[test]
fn end_of_source_swaps_to_a_ready_spool() {
    let first = gif(3);
    let fixture = Fixture::new(&first);
    let second = gif_colored(3, [7, 8, 9, 255]);
    let other = Fixture::new(&second);
    let spool = TempFile::new("spool");
    write_spool(&other.0, 8, &spool.0, || true).unwrap();
    let expected = decode(&second, 8).unwrap().0;
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    core.spool = Some(spool.0.clone());
    for _ in 0..3 {
        let target = core.base + core.position;
        core.step(target, || true).unwrap().unwrap();
    }
    assert_eq!(core.count, 3);
    let target = core.base + core.position;
    let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
    assert!(core.spool.is_none());
    assert!(!core.terminal);
    assert_eq!(start, target);
    assert_eq!(end - start, Duration::from(expected.delay(0)));
    assert_eq!(frame.as_bytes(0), expected.as_bytes(0));
}

#[test]
fn a_frame_read_before_a_spool_swap_is_still_processed() {
    let source = gif(DECODE_SLICE - 1);
    let fixture = Fixture::new(&source);
    let other = Fixture::new(&gif_colored(3, [7, 8, 9, 255]));
    let spool = TempFile::new("spool");
    write_spool(&other.0, 8, &spool.0, || true).unwrap();
    let expected = decode(&source, 8).unwrap().0;
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    core.spool = Some(spool.0.clone());
    let (frame, _, _) = core
        .step(Duration::from_secs(10), || true)
        .unwrap()
        .unwrap();
    assert!(core.spool.is_none());
    assert_eq!(frame.as_bytes(0), expected.as_bytes(DECODE_SLICE - 2));
}

#[test]
fn first_frame_jobs_use_the_bounded_pool_and_cancel_before_opening_stale_sources() {
    let bytes = gif(3);
    let fixture = Fixture::new(&bytes[..bytes.len() - 10]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let image = runtime
        .block_on(first(
            fixture.0.clone(),
            8,
            Arc::new(Mutex::new(Instant::now())),
        ))
        .unwrap()
        .unwrap();
    assert_eq!(image.frame_count(), 1);
    assert_eq!(image.as_bytes(0), decode(&bytes, 8).unwrap().0.as_bytes(0));
    assert_eq!(pool().workers.len(), WORKERS);
    let missing = fixture.0.with_extension("missing");
    let cancelled = runtime
        .block_on(first(
            missing,
            8,
            Arc::new(Mutex::new(Instant::now() - STALE_AFTER - IDLE)),
        ))
        .unwrap();
    assert!(cancelled.is_none());
}

#[test]
fn many_frame_playback_holds_at_most_still_plus_three_frames_including_scene_pins() {
    let fixture = Fixture::new(&gif(500));
    let retired = Rc::default();
    let mut playback = playback(fixture.0.clone(), &retired);
    let lease = DisplayLease::new(&playback.shared, true);
    let mut sessions = HashMap::new();
    let slots = playback.shared.lock().unwrap().slots.clone();
    let mut released = std::collections::HashSet::new();
    let mut handed = std::collections::HashSet::new();
    for _ in 0..1000 {
        request(&playback, &mut sessions, playback.end);
        assert!(playback.shared.lock().unwrap().ready.is_some());
        let old_scene = playback.current.clone();
        adopt(&mut playback, &retired);
        handed.insert(playback.current.as_ref().unwrap().image.id);
        assert!(slots.load(Ordering::Acquire) <= PLAYBACK_FRAMES);
        if old_scene.is_some() {
            request(&playback, &mut sessions, playback.end);
            assert!(playback.shared.lock().unwrap().ready.is_some());
            assert_eq!(slots.load(Ordering::Acquire), PLAYBACK_FRAMES);
        }
        drop(old_scene);
        release_with(&retired, |image| assert!(released.insert(image.id)));
    }
    drop(lease);
    playback.shared.lock().unwrap().ready = None;
    playback.current = None;
    release_with(&retired, |image| assert!(released.insert(image.id)));
    assert_eq!(slots.load(Ordering::Acquire), 0);
    assert_eq!(handed, released);
}

#[test]
fn decoder_only_runs_on_demand_and_never_runs_ahead_of_its_ready_slot() {
    let fixture = Fixture::new(&gif(100));
    let retired = Rc::default();
    let playback = playback(fixture.0.clone(), &retired);
    let mut sessions = HashMap::new();
    request(&playback, &mut sessions, playback.end);
    assert!(sessions.is_empty());
    let lease = DisplayLease::new(&playback.shared, true);
    request(&playback, &mut sessions, playback.end);
    let count = sessions[&playback.key].core.decoded;
    assert_eq!(count, 2);
    for _ in 0..100 {
        request(&playback, &mut sessions, Duration::from_secs(10));
    }
    assert_eq!(sessions[&playback.key].core.decoded, count);
    playback.shared.lock().unwrap().ready = None;
    drop(lease);
    for _ in 0..100 {
        request(&playback, &mut sessions, Duration::from_secs(10));
    }
    assert_eq!(sessions[&playback.key].core.decoded, count);
}

#[test]
fn idle_discards_decoder_and_playback_frames_keeps_still_and_reopens_source() {
    let fixture = Fixture::new(&gif(100));
    let retired = Rc::default();
    let mut playback = playback(fixture.0.clone(), &retired);
    let still_id = playback.still.image.id;
    let slots = playback.shared.lock().unwrap().slots.clone();
    let lease = DisplayLease::new(&playback.shared, true);
    let mut sessions = HashMap::new();
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    request(&playback, &mut sessions, playback.end);
    assert_eq!(slots.load(Ordering::Acquire), 2);
    drop(lease);
    let absent = playback.shared.lock().unwrap().absent_since;
    assert!(!playback.expire(absent + IDLE - Duration::from_nanos(1)));
    assert!(playback.expire(absent + IDLE));
    sweep_sessions(&mut sessions, absent + IDLE);
    assert!(sessions.is_empty());
    assert!(playback.current.is_none());
    assert!(playback.shared.lock().unwrap().ready.is_none());
    assert_eq!(playback.still.image.id, still_id);
    assert!(playback.started.is_none());
    release_with(&retired, |_| {});
    assert_eq!(slots.load(Ordering::Acquire), 0);
    let _lease = DisplayLease::new(&playback.shared, true);
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    assert!(sessions.contains_key(&playback.key));
    assert_eq!(
        playback.current.as_ref().unwrap().image.as_bytes(0),
        decode(&gif(100), 8).unwrap().0.as_bytes(1)
    );
}

#[test]
fn truncated_gif_keeps_last_good_frame_and_stops_without_panicking() {
    let bytes = gif(3);
    let truncated = &bytes[..bytes.len() - 10];
    assert!(decode(truncated, 8).is_err());
    let fixture = Fixture::new(truncated);
    let retired = Rc::default();
    let mut playback = playback(fixture.0.clone(), &retired);
    let _lease = DisplayLease::new(&playback.shared, true);
    let mut sessions = HashMap::new();
    request(&playback, &mut sessions, Duration::ZERO);
    adopt(&mut playback, &retired);
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    release_with(&retired, |_| {});
    let last_good = playback.current.as_ref().unwrap().image.clone();
    request(&playback, &mut sessions, playback.end);
    assert!(playback.shared.lock().unwrap().terminal);
    assert!(playback.shared.lock().unwrap().ready.is_none());
    assert_eq!(playback.current.as_ref().unwrap().image.id, last_good.id);
    assert_eq!(
        last_good.as_bytes(0),
        decode(&bytes, 8).unwrap().0.as_bytes(1)
    );
    assert!(sessions.is_empty());
}

#[test]
fn stalled_playback_skips_full_cycles_and_bounds_each_decode_slice() {
    let fixture = Fixture::new(&gif(20));
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    let target = Duration::from_secs(3600);
    let mut latest_end = Duration::ZERO;
    for _ in 0..20 {
        let before = core.decoded;
        if let Some((_, _, end)) = core.step(target, || true).unwrap() {
            latest_end = end;
        }
        assert!(core.decoded - before <= DECODE_SLICE);
        if latest_end > target {
            break;
        }
    }
    assert!(latest_end > target);
    let before = core.decoded;
    assert!(core.step(target, || false).unwrap().is_none());
    assert_eq!(core.decoded, before);
}

#[test]
fn short_absences_pause_the_clock_and_stale_generations_cannot_publish() {
    let fixture = Fixture::new(&gif(100));
    let retired = Rc::default();
    let mut playback = playback(fixture.0.clone(), &retired);
    let base = playback.started.unwrap();
    playback.shared.lock().unwrap().paused_since = Some(base + Duration::from_millis(20));
    assert_eq!(
        playback.resume(base + Duration::from_millis(500)),
        Duration::from_millis(20)
    );
    let _lease = DisplayLease::new(&playback.shared, true);
    assert_eq!(
        playback.resume(base + Duration::from_millis(700)),
        Duration::from_millis(220)
    );
    let mut sessions = HashMap::new();
    let generation = playback.shared.lock().unwrap().generation;
    playback.shared.lock().unwrap().reset();
    advance(
        &mut sessions,
        playback.key,
        generation,
        playback.plays,
        playback.path.clone(),
        playback.size,
        playback.kind.clone(),
        None,
        &playback.shared,
    );
    assert!(sessions.is_empty());
    assert!(playback.shared.lock().unwrap().ready.is_none());
}

#[test]
fn inactive_scenes_keep_the_current_frame_without_decoding_or_advancing_the_clock() {
    let fixture = Fixture::new(&gif(100));
    let retired = Rc::default();
    let mut playback = playback(fixture.0.clone(), &retired);
    let mut sessions = HashMap::new();
    let active = DisplayLease::new(&playback.shared, true);
    request(&playback, &mut sessions, playback.end);
    adopt(&mut playback, &retired);
    let current = playback.current.as_ref().unwrap().image.id;
    let decoded = sessions[&playback.key].core.decoded;
    let inactive = DisplayLease::new(&playback.shared, false);
    drop(active);
    let paused = playback.shared.lock().unwrap().paused_since.unwrap();
    let elapsed = paused.saturating_duration_since(playback.started.unwrap());
    assert!(!playback.expire(paused + IDLE * 10));
    sweep_sessions(&mut sessions, paused + IDLE * 10);
    request(&playback, &mut sessions, playback.end);
    assert_eq!(sessions[&playback.key].core.decoded, decoded);
    assert_eq!(playback.current.as_ref().unwrap().image.id, current);
    assert!(playback.shared.lock().unwrap().ready.is_none());
    let returned = paused + Duration::from_secs(10);
    assert_eq!(playback.resume(returned), elapsed);
    let _active = DisplayLease::new(&playback.shared, true);
    drop(inactive);
    assert_eq!(playback.resume(returned + POLL), elapsed + POLL);
    assert_eq!(playback.current.as_ref().unwrap().image.id, current);
}

#[test]
fn source_dimensions_frame_count_and_file_size_remain_limited() {
    let fixture = Fixture::new(&gif(MAX_FRAMES + 1));
    let mut core = Core::open(fixture.0.clone(), 8).unwrap();
    for _ in 0..MAX_FRAMES + 1 {
        core.step(core.base + core.position, || true).unwrap();
    }
    assert!(core.terminal);
    assert_eq!(core.decoded, MAX_FRAMES);
    assert!(Core::open(fixture.0.clone(), 0).is_err());
    let mut bad_header = gif(1);
    bad_header[6..8].copy_from_slice(&(MAX_DIMENSION as u16 + 1).to_le_bytes());
    let bad = Fixture::new(&bad_header);
    assert!(Core::open(bad.0.clone(), 8).is_err());
    File::options()
        .write(true)
        .open(&bad.0)
        .unwrap()
        .set_len(MAX_SOURCE_BYTES + 1)
        .unwrap();
    assert!(Core::open(bad.0.clone(), 8).is_err());
}

#[test]
fn large_media_only_uses_streaming() {
    assert!(selected(&spec()));
    assert!(selected(&ImageSpec::Attachment {
        id: 1,
        url: "https://cdn.discordapp.com/test.gif".into(),
        width: 8,
        height: 4,
    }));
    for spec in [
        ImageSpec::Emoji {
            id: 1,
            animated: true,
        },
        ImageSpec::Avatar {
            user_id: 1,
            hash: "a_test".into(),
        },
        ImageSpec::Nameplate {
            asset: "test".into(),
            animated: true,
        },
    ] {
        assert!(!selected(&spec));
    }
    assert!(selected(&ImageSpec::Collectible { url: "test".into() }));
    let still = ImageSpec::Media {
        url: "https://media.discordapp.net/test.gif".into(),
        width: 8,
        height: 4,
        animated: false,
    };
    assert!(matches!(
        source_spec(&still),
        ImageSpec::Media { animated: true, .. }
    ));
    assert!(matches!(
        still,
        ImageSpec::Media {
            animated: false,
            ..
        }
    ));
}

#[test]
fn local_files_only_stream_gifs() {
    assert!(selected(&ImageSpec::LocalGif {
        path: PathBuf::from("staged.gif"),
    }));
    assert!(!selected(&ImageSpec::LocalVideo {
        path: PathBuf::from("staged.mp4"),
    }));
}

struct LeaseView {
    frame: Option<Rc<FrameImage>>,
}

struct StreamProbe {
    still: Arc<RenderImage>,
    animate: bool,
    picked: Rc<std::cell::Cell<Option<gpui::ImageId>>>,
}

impl Render for StreamProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let still = self.still.clone();
        let picked = self.picked.clone();
        let animate = self.animate;
        gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let image = paint(&still, animate, None, window, cx).unwrap();
                picked.set(Some(image.id));
                window
                    .paint_image(bounds, gpui::Corners::default(), image, 0, false)
                    .unwrap();
            },
        )
        .size(px(32.))
    }
}

#[gpui::test]
fn reduced_motion_inactivity_and_production_close_cleanup(cx: &mut TestAppContext) {
    cx.update(super::super::init);
    FORCE_ACTIVE.set(true);
    let fixture = Fixture::new(&gif(3));
    let still = still();
    let still_id = still.id;
    let playback = cx.update(|cx| {
        register(&spec(), 8, fixture.0.clone(), None, still.clone(), cx);
        cx.global::<StreamGlobal>().state.borrow().entries[&still_id].clone()
    });
    let slots = playback.borrow().shared.lock().unwrap().slots.clone();
    let frame_id = {
        let mut playback = playback.borrow_mut();
        let frame = Rc::new(FrameImage {
            image: self::still(),
            slot: Slot::reserve(&slots),
            retired: playback.still.retired.clone(),
        });
        let id = frame.image.id;
        playback.current = Some(frame);
        playback.started = Some(Instant::now());
        playback.shared.lock().unwrap().terminal = true;
        id
    };
    let picked = Rc::new(std::cell::Cell::new(None));
    let (view, cx) = cx.add_window_view(|_, _| StreamProbe {
        still,
        animate: true,
        picked: picked.clone(),
    });
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(picked.get(), Some(frame_id));
    assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
    cx.update(|window, cx| {
        cx.set_reduce_motion(true);
        window.refresh();
        window.draw(cx).clear();
    });
    assert_eq!(picked.get(), Some(still_id));
    assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
    assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
    FORCE_ACTIVE.set(false);
    cx.update(|window, cx| {
        cx.set_reduce_motion(false);
        window.refresh();
    });
    view.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(picked.get(), Some(frame_id));
    assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
    assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
    assert!(!playback.borrow_mut().expire(Instant::now() + IDLE * 2));
    assert_eq!(
        playback.borrow().current.as_ref().unwrap().image.id,
        frame_id
    );
    FORCE_ACTIVE.set(true);
    view.update(cx, |view, cx| {
        view.animate = false;
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(picked.get(), Some(still_id));
    assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
    assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
    FORCE_ACTIVE.set(false);
    cx.update(|window, _| window.remove_window());
    assert!(playback.borrow().current.is_none());
    assert!(playback.borrow().started.is_none());
    assert_eq!(slots.load(Ordering::Acquire), 0);
    assert!(cx.cx.read(|cx| {
        cx.global::<StreamGlobal>()
            .state
            .borrow()
            .retired
            .borrow()
            .is_empty()
    }));
}

impl Render for LeaseView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.clone();
        gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some(frame) = frame {
                    window
                        .paint_image(
                            bounds,
                            gpui::Corners::default(),
                            frame.image.clone(),
                            0,
                            false,
                        )
                        .unwrap();
                    pin_scene(frame.clone(), None, frame.image.clone(), window);
                }
            },
        )
        .size(px(32.))
    }
}

struct LeaseHost(Entity<LeaseView>);

impl Render for LeaseHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.0
                .clone()
                .cached(StyleRefinement::default().size(px(32.))),
        )
    }
}

#[gpui::test]
fn scene_reuse_replacement_and_window_close_release_each_image_exactly_once(
    cx: &mut TestAppContext,
) {
    let retired: Retirements = Rc::default();
    let slots = Arc::new(AtomicUsize::new(0));
    let first = Rc::new(FrameImage {
        image: still(),
        slot: Slot::reserve(&slots),
        retired: retired.clone(),
    });
    let first_id = first.image.id;
    let weak = Rc::downgrade(&first);
    let (host, cx) =
        cx.add_window_view(|_, cx| LeaseHost(cx.new(|_| LeaseView { frame: Some(first) })));
    cx.update(|window, cx| window.draw(cx).clear());
    let leaf = host.read_with(cx, |host, _| host.0.clone());
    leaf.update(cx, |view, _| view.frame = None);
    host.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(
        weak.upgrade().is_some(),
        "cached scene must retain its image"
    );
    assert!(retired.borrow().is_empty());
    assert_eq!(slots.load(Ordering::Acquire), 1);
    leaf.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(weak.upgrade().is_none());
    let mut released = Vec::new();
    cx.update(|_, cx| {
        release_with(&retired, |image| {
            released.push(image.id);
            cx.drop_image(image, None);
        })
    });
    assert_eq!(released, [first_id]);
    assert_eq!(slots.load(Ordering::Acquire), 0);

    let last = Rc::new(FrameImage {
        image: still(),
        slot: Slot::reserve(&slots),
        retired: retired.clone(),
    });
    let last_id = last.image.id;
    leaf.update(cx, |view, cx| {
        view.frame = Some(last);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear());
    leaf.update(cx, |view, _| view.frame = None);
    cx.update(|window, _| window.remove_window());
    cx.cx.update(|cx| {
        release_with(&retired, |image| {
            released.push(image.id);
            cx.drop_image(image, None);
        })
    });
    release_with(&retired, |image| released.push(image.id));
    assert_eq!(released, [first_id, last_id]);
    assert_eq!(slots.load(Ordering::Acquire), 0);
}
