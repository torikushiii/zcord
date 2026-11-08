use super::*;

impl VoiceManager {
    pub fn watch(&self, user_id: u64) -> Option<Value> {
        let mut shared = self.lock();
        if user_id == 0 || shared.user_id == Some(user_id) {
            return None;
        }
        let (guild_id, channel_id) = shared.confirmed?;
        shared.session_id.as_ref()?;
        let key = stream::stream_key((guild_id != 0).then_some(guild_id), channel_id, user_id);
        self.end_stream(&mut shared, Some("Watching another stream".into()));
        shared.stream = Some(StreamWatch {
            user_id,
            key: key.clone(),
            server_id: server_id(guild_id, channel_id),
            rtc_server_id: None,
            rtc_channel_id: None,
            token: None,
            endpoint: None,
            shutdown: None,
            ended: Arc::new(AtomicBool::new(false)),
            started: false,
        });
        *shared.frames.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Some(json!({"op": 20, "d": {"stream_key": key}}))
    }

    pub fn stop_watching(&self) -> Option<Value> {
        let mut shared = self.lock();
        let key = shared.stream.as_ref()?.key.clone();
        self.end_stream(&mut shared, None);
        Some(json!({"op": 19, "d": {"stream_key": key}}))
    }

    pub fn stream_created(&self, key: &str, rtc_server_id: u64, rtc_channel_id: Option<u64>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            let next = (rtc_server_id, rtc_channel_id.unwrap_or(rtc_server_id));
            if shared.share.as_ref().is_some_and(|share| {
                share
                    .rtc_server_id
                    .is_some_and(|server| (server, share.rtc_channel_id.unwrap_or(server)) != next)
            }) {
                self.end_share(
                    &mut shared,
                    Some("Discord changed the stream connection identity".into()),
                );
                return;
            }
            if let Some(share) = shared.share.as_mut() {
                share.rtc_server_id = Some(rtc_server_id);
                share.rtc_channel_id = rtc_channel_id;
            }
            self.spawn_share(&mut shared);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        let next = (rtc_server_id, rtc_channel_id.unwrap_or(rtc_server_id));
        if shared.stream.as_ref().is_some_and(|stream| {
            stream
                .rtc_server_id
                .is_some_and(|server| (server, stream.rtc_channel_id.unwrap_or(server)) != next)
        }) {
            self.end_stream(
                &mut shared,
                Some("Discord changed the stream connection identity".into()),
            );
            return;
        }
        if let Some(stream) = shared.stream.as_mut() {
            stream.rtc_server_id = Some(rtc_server_id);
            stream.rtc_channel_id = rtc_channel_id;
        }
        self.spawn_stream(&mut shared);
    }

    pub fn stream_server(&self, key: &str, token: &str, endpoint: Option<&str>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            if token.len() > 2048 || endpoint.is_some_and(|endpoint| endpoint.len() > 512) {
                self.end_share(
                    &mut shared,
                    Some("Discord sent oversized stream server data".into()),
                );
                return;
            }
            if let Some(share) = shared.share.as_mut() {
                share.token = Some(Secret(token.to_owned()));
                share.endpoint = endpoint.map(str::to_owned);
            }
            self.spawn_share(&mut shared);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        if token.len() > 2048 || endpoint.is_some_and(|endpoint| endpoint.len() > 512) {
            self.end_stream(
                &mut shared,
                Some("Discord sent oversized stream server data".into()),
            );
            return;
        }
        if let Some(stream) = shared.stream.as_mut() {
            stream.token = Some(Secret(token.to_owned()));
            stream.endpoint = endpoint.map(str::to_owned);
        }
        self.spawn_stream(&mut shared);
    }

    pub fn stream_deleted(&self, key: &str, reason: Option<String>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            self.end_share(&mut shared, reason);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        self.end_stream(&mut shared, reason);
    }

    pub fn screen_sources(&self) -> Vec<ScreenSource> {
        stream::share::sources()
    }

    pub fn screen_thumbnail(&self, source: &ScreenSource) -> Option<stream::StreamFrame> {
        stream::share::thumbnail(source)
    }

    pub fn screen_icon(&self, source: &ScreenSource) -> Option<stream::StreamFrame> {
        stream::share::icon(source)
    }

    pub fn go_live(&self) -> Option<Value> {
        self.begin_share(None, StreamQuality::STANDARD, false)
    }

    pub fn start_share(
        &self,
        source: ScreenSource,
        quality: StreamQuality,
        audio: bool,
    ) -> Option<Value> {
        self.begin_share(Some(source), quality, audio)
    }

    pub fn stop_sharing(&self) -> Option<Value> {
        let mut shared = self.lock();
        let key = shared.share.as_ref()?.key.clone();
        self.end_share_with(&mut shared, None, false);
        Some(json!({"op": 19, "d": {"stream_key": key}}))
    }

    fn begin_share(
        &self,
        source: Option<ScreenSource>,
        quality: StreamQuality,
        audio: bool,
    ) -> Option<Value> {
        let mut shared = self.lock();
        let (guild_id, channel_id) = shared.confirmed?;
        let user_id = shared.user_id?;
        shared.session_id.as_ref()?;
        let key = stream::stream_key((guild_id != 0).then_some(guild_id), channel_id, user_id);
        self.end_share(&mut shared, Some("Sharing replaced".into()));
        let local = shared.local_preview.clone();
        shared.share = Some(StreamShare {
            key,
            server_id: server_id(guild_id, channel_id),
            live: source.map(|source| stream::share::LiveShare::new(source, quality, audio, local)),
            rtc_server_id: None,
            rtc_channel_id: None,
            token: None,
            endpoint: None,
            shutdown: None,
            ended: Arc::new(AtomicBool::new(false)),
            started: false,
            keyframe: Arc::new(AtomicBool::new(true)),
            bitrate: Arc::new(AtomicU32::new(quality.initial_bitrate())),
            stop: Arc::new(AtomicBool::new(false)),
            reannounce: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(Mutex::new(None)),
        });
        Some(json!({"op": 18, "d": {
            "type": if guild_id == 0 { "call" } else { "guild" },
            "guild_id": if guild_id == 0 { Value::Null } else { json!(guild_id.to_string()) },
            "channel_id": channel_id.to_string(),
            "preferred_region": null,
        }}))
    }

    pub(super) fn end_share(&self, shared: &mut Shared, reason: Option<String>) {
        self.end_share_with(shared, reason, true);
    }

    fn end_share_with(&self, shared: &mut Shared, reason: Option<String>, notify: bool) {
        let Some(mut share) = shared.share.take() else {
            return;
        };
        share.stop.store(true, Ordering::Release);
        if let Some(shutdown) = share.shutdown.take() {
            let _ = shutdown.send(true);
        }
        *shared
            .local_preview
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        if notify
            && let (Some(sender), Some(generation)) = (
                self.commands.clone(),
                self.gateway_generation
                    .as_ref()
                    .map(|generation| generation.load(Ordering::Acquire)),
            )
            && generation % 2 == 1
        {
            let payload = json!({"op": 19, "d": {"stream_key": share.key}});
            let _ = sender.send((generation, commands::voice_state(payload)));
        }
        if !share.ended.swap(true, Ordering::AcqRel) {
            self.emit_share(ShareEvent::Ended { reason });
        }
    }

    fn spawn_share(&self, shared: &mut Shared) {
        let Some(share) = shared.share.as_ref() else {
            return;
        };
        if share.started {
            return;
        }
        let (Some(live), Some(rtc_server_id), Some(token)) =
            (share.live.clone(), share.rtc_server_id, share.token.clone())
        else {
            return;
        };
        let Some(endpoint) = share
            .endpoint
            .clone()
            .or_else(|| shared.servers.get(&share.server_id)?.endpoint.clone())
        else {
            return;
        };
        let (Some(user_id), Some(session_id)) = (shared.user_id, shared.session_id.clone()) else {
            return;
        };
        let (Some(runtime), Some(events)) = (&self.runtime, &self.events) else {
            return;
        };
        let key = share.key.clone();
        let ended = share.ended.clone();
        let keyframe = share.keyframe.clone();
        let bitrate = share.bitrate.clone();
        let stop = share.stop.clone();
        let reannounce = share.reannounce.clone();
        let failure = share.failure.clone();
        let (frame_sender, frame_receiver) = std::sync::mpsc::sync_channel(2);
        let previews: stream::share::PreviewSink = {
            let (events, stream_key) = (events.clone(), key.clone());
            Box::new(move |thumbnail| {
                let stream_key = stream_key.clone();
                emit(
                    &events,
                    VoiceEvent::Share(ShareEvent::Preview {
                        stream_key,
                        thumbnail,
                    }),
                );
            })
        };
        if let Err(reason) = stream::share::spawn(
            live.clone(),
            bitrate.clone(),
            keyframe.clone(),
            stop.clone(),
            frame_sender,
            failure.clone(),
            previews,
        ) {
            self.end_share(shared, Some(reason));
            return;
        }
        let (audio_sender, audio_receiver) = std::sync::mpsc::sync_channel(8);
        let audio_frames = match stream::share::spawn_audio(audio_sender, stop.clone(), live) {
            Ok(()) => Some(audio_receiver),
            Err(reason) => {
                log::warn!("Streaming without sound: {reason}");
                None
            }
        };
        let (shutdown, cancel) = watch::channel(false);
        let Some(share) = shared.share.as_mut() else {
            return;
        };
        share.shutdown = Some(shutdown);
        share.started = true;
        self.emit_share(ShareEvent::Starting);
        let config = SenderConfig {
            user_id,
            session_id,
            token,
            endpoint,
            rtc_server_id,
            key,
            keyframe,
            bitrate,
            reannounce,
            failure,
            events: events.clone(),
            ended: ended.clone(),
        };
        let manager = self.clone();
        runtime.spawn(async move {
            let result = stream::sender::run(config, frame_receiver, audio_frames, cancel).await;
            stop.store(true, Ordering::Release);
            let mut shared = manager.lock();
            manager.end_share_with(&mut shared, result.err(), true);
        });
    }

    fn emit_share(&self, event: ShareEvent) {
        if let Some(events) = &self.events {
            emit(events, VoiceEvent::Share(event));
        }
    }

    pub fn change_share(&self, source: Option<ScreenSource>, quality: Option<StreamQuality>) {
        if let Some(live) = self
            .lock()
            .share
            .as_ref()
            .and_then(|share| share.live.as_ref())
        {
            live.change(source, quality);
        }
    }

    pub fn set_share_audio(&self, audio: bool) {
        if let Some(live) = self
            .lock()
            .share
            .as_ref()
            .and_then(|share| share.live.as_ref())
        {
            live.audio.store(audio, Ordering::Release);
        }
    }

    pub fn stream_updated(&self, key: &str, viewers: Vec<u64>) {
        let shared = self.lock();
        let Some(share) = shared.share.as_ref().filter(|share| share.key == key) else {
            return;
        };
        if !viewers.is_empty() {
            share.reannounce.store(true, Ordering::Release);
        }
        self.emit_share(ShareEvent::Viewers(viewers));
    }

    pub fn stream_frames(&self) -> FrameSlot {
        self.lock().frames.clone()
    }

    pub fn share_frames(&self) -> FrameSlot {
        self.lock().local_preview.frames.clone()
    }

    pub fn set_share_preview(&self, bounds: Option<(u32, u32)>) {
        let shared = self.lock();
        *shared
            .local_preview
            .bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = bounds;
        if bounds.is_none() {
            *shared
                .local_preview
                .frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    pub(super) fn end_stream(&self, shared: &mut Shared, reason: Option<String>) {
        shared.flags.clear_stream_audio();
        let Some(mut stream) = shared.stream.take() else {
            return;
        };
        if let Some(shutdown) = stream.shutdown.take() {
            let _ = shutdown.send(true);
        }
        *shared.frames.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if !stream.ended.swap(true, Ordering::AcqRel) {
            self.emit_stream(StreamEvent::Ended {
                user_id: stream.user_id,
                reason,
            });
        }
    }

    fn spawn_stream(&self, shared: &mut Shared) {
        let Some(stream) = shared.stream.as_ref() else {
            return;
        };
        if stream.started {
            return;
        }
        let (Some(rtc_server_id), Some(token)) = (stream.rtc_server_id, stream.token.clone())
        else {
            return;
        };
        let Some(endpoint) = stream
            .endpoint
            .clone()
            .or_else(|| shared.servers.get(&stream.server_id)?.endpoint.clone())
        else {
            return;
        };
        let (Some(user_id), Some(session_id)) = (shared.user_id, shared.session_id.clone()) else {
            return;
        };
        let (Some(runtime), Some(events)) = (&self.runtime, &self.events) else {
            return;
        };
        let frames = shared.frames.clone();
        let flags = shared.flags.clone();
        let Some(stream) = shared.stream.as_mut() else {
            return;
        };
        let streamer_id = stream.user_id;
        let ended = stream.ended.clone();
        let watch_ended = ended.clone();
        let (shutdown, cancel) = watch::channel(false);
        stream.shutdown = Some(shutdown);
        stream.started = true;
        self.emit_stream(StreamEvent::Connecting {
            user_id: streamer_id,
        });
        let audio_generation = flags.begin_stream_audio();
        let config = ViewerConfig {
            user_id,
            streamer_id,
            session_id,
            token,
            endpoint,
            rtc_server_id,
            frames,
            events: events.clone(),
            ended,
            flags,
            audio_generation,
        };
        let manager = self.clone();
        runtime.spawn(async move {
            let result = stream::viewer::run(config, cancel).await;
            manager.stream_viewer_ended(&watch_ended, result);
        });
    }

    fn stream_viewer_ended(&self, ended: &Arc<AtomicBool>, result: Result<(), String>) {
        let mut shared = self.lock();
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| !Arc::ptr_eq(&stream.ended, ended))
        {
            return;
        }
        self.end_stream(&mut shared, result.err());
    }

    fn emit_stream(&self, event: StreamEvent) {
        if let Some(events) = &self.events {
            emit(events, VoiceEvent::Stream(event));
        }
    }
}
