use super::super::{
    crypto::{Cipher, Mode},
    dave::{Dave, EncryptOutcome},
    gateway::{
        self, commit_welcome_body, parse_binary_seq, ready_on_prepare, send_binary, send_json,
        send_key_package, send_ready,
    },
    rtcp,
    rtp::Header,
};
use super::{
    handshake::{self, HEARTBEAT_DEFAULT, HandshakeConfig, Socket},
    history::History,
    max_bitrate,
    pacer::Pacer,
    packetize::{self, Packet},
    share::{AudioFrame, EncodedFrame},
};
use crate::{Event, Secret, ShareEvent, VoiceEvent, model};
use futures::{StreamExt, channel::mpsc::UnboundedSender};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::UdpSocket,
    sync::watch,
    time::{MissedTickBehavior, interval, sleep_until},
};
use tokio_tungstenite::tungstenite::Message;

const MIN_BITRATE: u32 = 500_000;
const KEYFRAME_INTERVAL: Duration = Duration::from_secs(3);
const SENDER_REPORT: Duration = Duration::from_secs(1);
const MAX_PACKET: usize = 2048;
const STATS_INTERVAL: Duration = Duration::from_secs(5);
const AUDIO_GAP: Duration = Duration::from_millis(100);
const SOUNDSHARE: u32 = 2;

pub(crate) struct SenderConfig {
    pub user_id: u64,
    pub session_id: Secret,
    pub token: Secret,
    pub endpoint: String,
    pub rtc_server_id: u64,
    pub key: String,
    pub keyframe: Arc<AtomicBool>,
    pub bitrate: Arc<AtomicU32>,
    pub reannounce: Arc<AtomicBool>,
    pub failure: Arc<Mutex<Option<String>>>,
    pub events: UnboundedSender<Event>,
    pub ended: Arc<AtomicBool>,
}

pub(crate) async fn run(
    config: SenderConfig,
    frames: Receiver<EncodedFrame>,
    audio: Option<Receiver<AudioFrame>>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let handshake = handshake::handshake(
        &HandshakeConfig {
            user_id: config.user_id,
            session_id: config.session_id.clone(),
            token: config.token.clone(),
            endpoint: config.endpoint.clone(),
            rtc_server_id: config.rtc_server_id,
            streams: json!([{"type": "video", "rid": "100", "quality": 100}]),
            encode: true,
            decode: false,
        },
        &mut cancel,
    )
    .await?;
    let (frame_sender, mut frame_receiver) = tokio::sync::mpsc::channel(2);
    std::thread::spawn(move || {
        while let Ok(frame) = frames.recv() {
            if frame_sender.blocking_send(frame).is_err() {
                break;
            }
        }
    });
    let mut audio = audio.map(|frames| {
        let (audio_sender, audio_receiver) = tokio::sync::mpsc::channel(8);
        std::thread::spawn(move || {
            while let Ok(frame) = frames.recv() {
                if audio_sender.blocking_send(frame).is_err() {
                    break;
                }
            }
        });
        audio_receiver
    });
    if handshake.video_ssrc == 0 {
        return Err("Discord did not assign a stream SSRC".into());
    }
    let mut sender = Sender::new(config);
    sender.sound = audio.is_some();
    sender.audio_ssrc = handshake.ssrc;
    sender.video_ssrc = handshake.video_ssrc;
    sender.rtx_ssrc = handshake.rtx_ssrc;
    sender.heartbeat = handshake.heartbeat;
    sender.next_heartbeat = Instant::now() + handshake.heartbeat;
    let handshake::Handshake {
        mut socket,
        udp,
        peer,
        ..
    } = handshake;
    let mut tick = interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut buffer = [0u8; MAX_PACKET];
    loop {
        tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(()),
            _ = tick.tick() => sender.tick(&mut socket, &udp, peer).await?,
            _ = sleep_until(tokio::time::Instant::from_std(sender.pacer.deadline())), if !sender.pacer.is_empty() || sender.history.has_pending() => {
                sender.flush(&udp, peer, Instant::now()).await?;
            }
            frame = frame_receiver.recv(), if sender.pacer.is_empty() => {
                let Some(frame) = frame else {
                    return Err("The screen capture stopped".into());
                };
                sender.frame(&mut socket, frame, Instant::now()).await?;
            }
            sound = next_audio(&mut audio) => match sound {
                Some(sound) => sender.audio(&udp, peer, sound, Instant::now()).await?,
                None => audio = None,
            },
            received = udp.recv_from(&mut buffer) => {
                let Ok((size, _)) = received else { continue };
                sender.feedback(&udp, peer, &buffer[..size], Instant::now()).await?;
            }
            message = socket.next() => {
                let Some(Ok(message)) = message else {
                    return Err("Discord stream socket failed".into());
                };
                sender.message(&mut socket, message).await?;
            }
        }
    }
}

struct Sender {
    config: SenderConfig,
    cipher: Option<Cipher>,
    dave: Option<Dave>,
    audio_ssrc: u32,
    video_ssrc: u32,
    rtx_ssrc: u32,
    rtp_sequence: u16,
    transport_sequence: u16,
    rtx_sequence: u16,
    pacer: Pacer,
    history: History,
    announced: bool,
    frame_size: (u32, u32),
    frame_fps: u32,
    next_keyframe: Instant,
    next_report: Instant,
    next_heartbeat: Instant,
    heartbeat: Duration,
    seq_ack: Option<u64>,
    rtp_timestamp: u32,
    packets: u32,
    octets: u32,
    stats: Stats,
    audio_sequence: u16,
    audio_timestamp: u32,
    audio_sent: Option<Instant>,
    audio_sample: u64,
    video_sent: Option<Instant>,
    audio_epoch: Instant,
    sound: bool,
    audio_packets: u32,
    audio_octets: u32,
}

struct Stats {
    since: Instant,
    frames: u32,
    keyframes: u32,
    bytes: u64,
    remb: Option<u32>,
    audio_frames: u32,
    received: u32,
    dropped_insecure: u32,
    dropped_for_keyframe: u32,
}

impl Stats {
    fn new(now: Instant) -> Self {
        Self {
            since: now,
            frames: 0,
            keyframes: 0,
            bytes: 0,
            remb: None,
            audio_frames: 0,
            received: 0,
            dropped_insecure: 0,
            dropped_for_keyframe: 0,
        }
    }
}

impl Sender {
    fn new(config: SenderConfig) -> Self {
        let now = Instant::now();
        Self {
            config,
            cipher: None,
            dave: None,
            audio_ssrc: 0,
            video_ssrc: 0,
            rtx_ssrc: 0,
            rtp_sequence: rand::random(),
            transport_sequence: rand::random(),
            rtx_sequence: rand::random(),
            pacer: Pacer::default(),
            history: History::default(),
            announced: false,
            frame_size: (0, 0),
            frame_fps: 0,
            next_keyframe: now,
            next_report: now + SENDER_REPORT,
            next_heartbeat: now + HEARTBEAT_DEFAULT,
            heartbeat: HEARTBEAT_DEFAULT,
            seq_ack: None,
            rtp_timestamp: 0,
            packets: 0,
            octets: 0,
            stats: Stats::new(now),
            audio_sequence: rand::random(),
            audio_timestamp: 0,
            audio_sent: None,
            audio_sample: 0,
            video_sent: None,
            audio_epoch: now,
            sound: false,
            audio_packets: 0,
            audio_octets: 0,
        }
    }

    async fn message(&mut self, socket: &mut Socket, message: Message) -> Result<(), String> {
        match message {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(&text)
                    .map_err(|_| "Discord sent invalid stream signaling".to_owned())?;
                let op = value.get("op").and_then(Value::as_u64).unwrap_or(0) as u8;
                if let Some(seq) = value.get("seq").and_then(Value::as_u64) {
                    self.seq_ack = Some(seq);
                }
                let data = value.get("d").cloned().unwrap_or(Value::Null);
                self.signal(socket, op, data).await
            }
            Message::Binary(bytes) => self.binary(socket, &bytes).await,
            Message::Close(_) => Err("Discord stream connection closed".into()),
            _ => Ok(()),
        }
    }

    async fn signal(&mut self, socket: &mut Socket, op: u8, data: Value) -> Result<(), String> {
        match op {
            2 => Err("Unexpected stream transport replacement".into()),
            4 => self.session(socket, &data).await,
            5 | 6 | 12 => Ok(()),
            8 => {
                if let Some(interval) = data
                    .get("heartbeat_interval")
                    .and_then(Value::as_u64)
                    .filter(|ms| *ms >= 100 && *ms <= 120_000)
                {
                    self.heartbeat = Duration::from_millis(interval);
                    self.next_heartbeat = Instant::now() + self.heartbeat;
                }
                Ok(())
            }
            11 => {
                if let Some(ids) = data.get("user_ids").and_then(Value::as_array)
                    && let Some(dave) = &mut self.dave
                {
                    for id in ids.iter().filter_map(model::id_value) {
                        dave.add_user(id);
                    }
                }
                self.config.reannounce.store(true, Ordering::Release);
                Ok(())
            }
            13 => {
                if let Some(user_id) = data.get("user_id").and_then(model::id_value)
                    && let Some(dave) = &mut self.dave
                {
                    dave.remove_user(user_id);
                }
                Ok(())
            }
            21 => {
                let version = data
                    .get("protocol_version")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                let transition = data
                    .get("transition_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                if let Some(dave) = &mut self.dave {
                    dave.prepare_transition(version, transition);
                    if transition == 0 {
                        dave.execute_transition(0);
                    } else if ready_on_prepare(transition, version, false) {
                        send_ready(socket, transition, version).await;
                    }
                }
                Ok(())
            }
            22 => {
                let transition = data
                    .get("transition_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                if let Some(dave) = &mut self.dave {
                    dave.execute_transition(transition);
                }
                Ok(())
            }
            24 => {
                let version = data
                    .get("protocol_version")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                let epoch = data.get("epoch").and_then(Value::as_u64).unwrap_or(0);
                if version > 0 && self.dave.is_none() {
                    let group = self.rtc_group()?;
                    let mut dave =
                        Dave::new(version, self.config.user_id, group).ok_or_else(|| {
                            "Discord requested unsupported stream encryption".to_owned()
                        })?;
                    dave.add_user(self.config.user_id);
                    self.dave = Some(dave);
                }
                if let Some(dave) = &mut self.dave
                    && let Some(key_package) = dave.prepare_epoch(version, epoch)
                {
                    send_key_package(socket, &key_package).await;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn binary(&mut self, socket: &mut Socket, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < 3 {
            return Ok(());
        }
        if let Some(seq) = parse_binary_seq(bytes) {
            self.seq_ack = Some(seq);
        }
        let op = bytes[2];
        let payload = &bytes[3..];
        match op {
            25 => {
                if let Some(dave) = &mut self.dave {
                    dave.set_external_sender(payload);
                }
            }
            27 => {
                if payload.is_empty() {
                    return Ok(());
                }
                if let Some(dave) = &mut self.dave
                    && let Some((commit, welcome)) = dave.proposals(payload[0], &payload[1..])
                {
                    send_binary(socket, 28, &commit_welcome_body(commit, welcome)).await;
                }
            }
            29 | 30 => {
                if payload.len() < 2 {
                    return Ok(());
                }
                let transition = u16::from_be_bytes([payload[0], payload[1]]);
                let body = &payload[2..];
                let result = if let Some(dave) = &mut self.dave {
                    if op == 29 {
                        dave.commit(transition, body)
                    } else {
                        dave.welcome(transition, body)
                    }
                } else {
                    false
                };
                if !result {
                    if transition != 0 {
                        let _ = send_json(
                            socket,
                            json!({"op": 31, "d": {"transition_id": transition}}),
                        )
                        .await;
                    }
                    return Ok(());
                }
                if transition == 0 {
                    if let Some(dave) = &mut self.dave {
                        dave.execute_transition(0);
                    }
                } else {
                    let version = self
                        .dave
                        .as_ref()
                        .and_then(|dave| dave.transition_version(transition))
                        .or_else(|| self.dave.as_ref().map(Dave::version))
                        .unwrap_or(0);
                    send_ready(socket, transition, version).await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn rtc_group(&self) -> Result<u64, String> {
        self.config
            .rtc_server_id
            .checked_sub(1)
            .filter(|id| *id != 0)
            .ok_or_else(|| "Unsupported Discord stream media-session identifier".to_owned())
    }

    async fn session(&mut self, socket: &mut Socket, data: &Value) -> Result<(), String> {
        if data.get("mode").and_then(Value::as_str) != Some(Mode::Aes256Gcm.name()) {
            return Err("Discord did not negotiate the required stream transport".into());
        }
        if data.get("dave_protocol_version").and_then(Value::as_u64) != Some(1) {
            return Err("Discord did not negotiate DAVE stream media".into());
        }
        if !data
            .get("video_codec")
            .and_then(Value::as_str)
            .is_some_and(|codec| codec.eq_ignore_ascii_case("H264"))
        {
            return Err("Discord did not negotiate H264 stream video".into());
        }
        let values = data
            .get("secret_key")
            .and_then(Value::as_array)
            .ok_or_else(|| "Discord sent an invalid stream transport key".to_owned())?;
        if values.len() != 32 {
            return Err("Discord sent an invalid stream transport key".into());
        }
        let mut key = [0u8; 32];
        for (output, value) in key.iter_mut().zip(values) {
            *output = value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| "Discord sent an invalid stream transport key".to_owned())?;
        }
        self.cipher = Some(
            Cipher::new(Mode::Aes256Gcm, &key)
                .ok_or_else(|| "Discord sent an invalid stream transport key".to_owned())?,
        );
        let group = self.rtc_group()?;
        let mut dave = Dave::new(1, self.config.user_id, group)
            .ok_or_else(|| "Discord requested unsupported stream encryption".to_owned())?;
        dave.add_user(self.config.user_id);
        let key_package = dave
            .key_package()
            .ok_or_else(|| "Could not create the stream encryption key".to_owned())?;
        send_key_package(socket, &key_package).await;
        self.dave = Some(dave);
        self.next_keyframe = Instant::now();
        self.config.keyframe.store(true, Ordering::Release);
        Ok(())
    }

    async fn frame(
        &mut self,
        socket: &mut Socket,
        frame: EncodedFrame,
        now: Instant,
    ) -> Result<(), String> {
        self.stats.received += 1;
        if !self.secure() {
            self.stats.dropped_insecure += 1;
            self.config.keyframe.store(true, Ordering::Release);
            return Ok(());
        }
        if self.config.keyframe.swap(false, Ordering::AcqRel) && !frame.keyframe {
            self.stats.dropped_for_keyframe += 1;
            self.config.keyframe.store(true, Ordering::Release);
            return Ok(());
        }
        let Some(dave) = &mut self.dave else {
            self.config.keyframe.store(true, Ordering::Release);
            return Ok(());
        };
        let Some(encrypted) = dave.encrypt_video(&frame.data) else {
            self.config.keyframe.store(true, Ordering::Release);
            return Ok(());
        };
        self.rtp_timestamp = frame.timestamp;
        self.video_sent = Some(now);
        let packets = packetize::packetize(
            &encrypted,
            &mut self.rtp_sequence,
            &mut self.transport_sequence,
            frame.timestamp,
            self.video_ssrc,
        )
        .map_err(str::to_owned)?;
        let size = ((frame.width, frame.height), frame.fps);
        if self.announced && size != (self.frame_size, self.frame_fps) {
            self.config.reannounce.store(true, Ordering::Release);
        }
        (self.frame_size, self.frame_fps) = size;
        if !self.announced {
            self.announce(socket).await?;
        }
        if frame.keyframe {
            self.next_keyframe = now + KEYFRAME_INTERVAL;
        }
        self.pacer.queue(packets, now);
        self.stats.frames += 1;
        self.stats.keyframes += u32::from(frame.keyframe);
        self.stats.bytes += frame.data.len() as u64;
        Ok(())
    }

    async fn announce(&mut self, socket: &mut Socket) -> Result<(), String> {
        let (width, height) = self.frame_size;
        let streams = json!([{
            "type": "video", "rid": "100", "ssrc": self.video_ssrc, "active": true,
            "quality": 100, "rtx_ssrc": self.rtx_ssrc, "max_bitrate": max_bitrate(height),
            "max_framerate": self.frame_fps, "max_resolution": {"type": "fixed", "width": width, "height": height},
        }]);
        send_json(
            socket,
            json!({"op": 12, "d": {
                "audio_ssrc": self.audio_ssrc,
                "video_ssrc": self.video_ssrc,
                "rtx_ssrc": self.rtx_ssrc,
                "streams": streams,
            }}),
        )
        .await
        .map_err(|_| "Discord stream announcement failed".to_owned())?;
        if self.sound {
            let _ = send_json(
                socket,
                json!({"op": 5, "d": {"speaking": SOUNDSHARE, "delay": 0, "ssrc": self.audio_ssrc}}),
            )
            .await;
        }
        let _ = send_json(
            socket,
            json!({"op": 22, "d": {"stream_key": self.config.key, "paused": false}}),
        )
        .await;
        self.announced = true;
        self.config.reannounce.store(false, Ordering::Release);
        self.next_report = Instant::now() + SENDER_REPORT;
        self.next_keyframe = Instant::now() + KEYFRAME_INTERVAL;
        if !self.config.ended.load(Ordering::Acquire) {
            let _ = self
                .config
                .events
                .unbounded_send(Event::Voice(VoiceEvent::Share(ShareEvent::Live)));
        }
        Ok(())
    }

    async fn audio(
        &mut self,
        udp: &UdpSocket,
        peer: SocketAddr,
        sound: AudioFrame,
        now: Instant,
    ) -> Result<(), String> {
        if !self.announced || !self.secure() || self.audio_ssrc == 0 {
            return Ok(());
        }
        let Some(dave) = &mut self.dave else {
            return Ok(());
        };
        let payload = match dave.encrypt(&sound.opus) {
            EncryptOutcome::Plaintext(payload) | EncryptOutcome::Encrypted(payload) => payload,
            EncryptOutcome::NotReady | EncryptOutcome::Failed => return Ok(()),
        };
        let talkspurt = self
            .audio_sent
            .is_none_or(|sent| now.duration_since(sent) > AUDIO_GAP);
        let timestamp = if talkspurt {
            (now.duration_since(self.audio_epoch).as_micros() * 48 / 1000) as u32
        } else {
            let elapsed = sound.sample.wrapping_sub(self.audio_sample);
            self.audio_timestamp.wrapping_add(elapsed as u32)
        };
        self.audio_sample = sound.sample;
        let aad =
            Header::audio(self.audio_ssrc, self.audio_sequence, timestamp, talkspurt).encode();
        let Some(cipher) = &mut self.cipher else {
            return Ok(());
        };
        let Some(sealed) = cipher.encrypt(&aad, &payload) else {
            return Ok(());
        };
        let mut wire = aad.to_vec();
        wire.extend_from_slice(&sealed);
        udp.send_to(&wire, peer)
            .await
            .map_err(|_| "Stream UDP send failed".to_owned())?;
        self.audio_packets = self.audio_packets.wrapping_add(1);
        self.audio_octets = self
            .audio_octets
            .wrapping_add(u32::try_from(payload.len()).unwrap_or(u32::MAX));
        self.audio_sequence = self.audio_sequence.wrapping_add(1);
        self.audio_timestamp = timestamp;
        self.audio_sent = Some(now);
        self.stats.audio_frames += 1;
        Ok(())
    }

    fn secure(&self) -> bool {
        self.cipher.is_some() && self.dave.as_ref().is_some_and(Dave::can_encrypt)
    }

    async fn flush(
        &mut self,
        udp: &UdpSocket,
        peer: SocketAddr,
        now: Instant,
    ) -> Result<(), String> {
        if self.history.has_pending()
            && let Some(packet) = self.history.repair(
                self.rtx_ssrc,
                &mut self.rtx_sequence,
                &mut self.transport_sequence,
                now,
            )
        {
            self.send_packet(udp, peer, packet).await?;
        }
        for packet in self.pacer.next_batch(now) {
            self.send_packet(udp, peer, packet).await?;
        }
        Ok(())
    }

    async fn send_packet(
        &mut self,
        udp: &UdpSocket,
        peer: SocketAddr,
        packet: Packet,
    ) -> Result<(), String> {
        let Some(cipher) = &mut self.cipher else {
            return Ok(());
        };
        let aad = packet.aad();
        let Some(sealed) = cipher.encrypt(&aad, &packet.plaintext()) else {
            return Ok(());
        };
        let mut wire = aad.to_vec();
        wire.extend_from_slice(&sealed);
        udp.send_to(&wire, peer)
            .await
            .map_err(|_| "Stream UDP send failed".to_owned())?;
        self.packets = self.packets.saturating_add(1);
        self.octets = self
            .octets
            .saturating_add(u32::try_from(packet.payload.len()).unwrap_or(u32::MAX));
        if self.rtx_ssrc != 0 {
            self.history.remember(packet);
        }
        Ok(())
    }

    async fn tick(
        &mut self,
        socket: &mut Socket,
        udp: &UdpSocket,
        peer: SocketAddr,
    ) -> Result<(), String> {
        let now = Instant::now();
        if let Some(reason) = self
            .config
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(reason);
        }
        if self.pacer.stale(now) {
            self.pacer.clear();
            self.history.clear();
            self.config.keyframe.store(true, Ordering::Release);
        }
        if self.announced && !self.secure() {
            self.announced = false;
            self.pacer.clear();
            self.history.clear();
            self.config.keyframe.store(true, Ordering::Release);
        }
        if self.announced {
            if now >= self.next_keyframe {
                self.config.keyframe.store(true, Ordering::Release);
                self.next_keyframe = now + KEYFRAME_INTERVAL;
            }
            if now >= self.next_report {
                self.send_report(udp, peer).await;
                self.next_report = now + SENDER_REPORT;
            }
            self.log_stats(now);
            if self.announced && self.config.reannounce.load(Ordering::Acquire) && self.secure() {
                self.announce(socket).await?;
            }
        }
        if now >= self.next_heartbeat {
            send_json(
                socket,
                json!({"op": 3, "d": {"t": gateway::unix_ms(), "seq_ack": self.seq_ack}}),
            )
            .await
            .map_err(|_| "Discord stream heartbeat failed".to_owned())?;
            self.next_heartbeat = now + self.heartbeat;
        }
        Ok(())
    }

    async fn send_report(&mut self, udp: &UdpSocket, peer: SocketAddr) {
        if self.video_ssrc == 0 {
            return;
        }
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now = Instant::now();
        let stats = rtcp::SenderStats {
            ntp_seconds: elapsed.as_secs() as u32,
            ntp_fraction: (((elapsed.subsec_nanos() as u64) << 32) / 1_000_000_000) as u32,
            rtp_timestamp: rtp_at(self.rtp_timestamp, self.video_sent, now, 90),
            packet_count: self.packets,
            octet_count: self.octets,
        };
        let clear = rtcp::sender_report(self.video_ssrc, &stats);
        self.send_rtcp(udp, peer, clear).await;
        if self.audio_packets > 0 {
            let stats = rtcp::SenderStats {
                rtp_timestamp: rtp_at(self.audio_timestamp, self.audio_sent, now, 48),
                packet_count: self.audio_packets,
                octet_count: self.audio_octets,
                ..stats
            };
            let clear = rtcp::sender_report(self.audio_ssrc, &stats);
            self.send_rtcp(udp, peer, clear).await;
        }
    }

    async fn feedback(
        &mut self,
        _udp: &UdpSocket,
        peer: SocketAddr,
        packet: &[u8],
        now: Instant,
    ) -> Result<(), String> {
        if packet.len() == 8 && packet.starts_with(&[0x13, 0x37, 0xca, 0xfe]) {
            return Ok(());
        }
        if rtcp::packet_type(packet).is_none() {
            return Ok(());
        }
        let Some(clear) = self.rtcp_plaintext(packet) else {
            return Ok(());
        };
        let Some(feedback) = rtcp::feedback(&clear, self.video_ssrc) else {
            return Ok(());
        };
        if feedback.keyframe || self.history.request(&feedback.nacks, now) {
            self.config.keyframe.store(true, Ordering::Release);
        }
        if let Some(bitrate) = feedback.bitrate {
            self.apply_remb(bitrate);
        }
        let _ = peer;
        Ok(())
    }

    fn log_stats(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.stats.since);
        if elapsed < STATS_INTERVAL {
            return;
        }
        let seconds = elapsed.as_secs_f64();
        let (width, height) = self.frame_size;
        log::debug!(
            "stream {width}x{height}: {} audio frames, {} frames in, {} dropped before encryption, {} dropped waiting for a keyframe; sent {:.1} fps, {} keyframes, sent {} kbps, encoder target {} kbps, cap {} kbps, last REMB {}",
            self.stats.audio_frames,
            self.stats.received,
            self.stats.dropped_insecure,
            self.stats.dropped_for_keyframe,
            f64::from(self.stats.frames) / seconds,
            self.stats.keyframes,
            (self.stats.bytes as f64 * 8. / seconds / 1000.) as u64,
            self.config.bitrate.load(Ordering::Acquire) / 1000,
            max_bitrate(height) / 1000,
            self.stats
                .remb
                .map_or("none".to_owned(), |remb| format!("{} kbps", remb / 1000)),
        );
        self.stats = Stats::new(now);
    }

    fn apply_remb(&mut self, bitrate: u32) {
        self.stats.remb = Some(bitrate);
        let target = bitrate.min(max_bitrate(self.frame_size.1)).max(MIN_BITRATE);
        let current = self.config.bitrate.load(Ordering::Acquire);
        let difference = target.abs_diff(current);
        if difference > current / 10 {
            self.config.bitrate.store(target, Ordering::Release);
        }
    }

    fn rtcp_plaintext(&self, packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() < 8 {
            return None;
        }
        let cipher = self.cipher.as_ref()?;
        let plaintext = cipher.decrypt(&packet[..8], &packet[8..])?;
        let mut clear = packet[..8].to_vec();
        clear.extend_from_slice(&plaintext);
        Some(clear)
    }

    async fn send_rtcp(&mut self, udp: &UdpSocket, peer: SocketAddr, clear: Vec<u8>) {
        if clear.len() < 8 {
            return;
        }
        let Some(cipher) = &mut self.cipher else {
            return;
        };
        let Some(sealed) = cipher.encrypt(&clear[..8], &clear[8..]) else {
            return;
        };
        let mut packet = clear[..8].to_vec();
        packet.extend_from_slice(&sealed);
        let _ = udp.send_to(&packet, peer).await;
    }
}

fn rtp_at(timestamp: u32, taken: Option<Instant>, now: Instant, khz: u128) -> u32 {
    let Some(taken) = taken else {
        return timestamp;
    };
    let ticks = now.saturating_duration_since(taken).as_micros() * khz / 1000;
    timestamp.wrapping_add(ticks as u32)
}

async fn next_audio(
    audio: &mut Option<tokio::sync::mpsc::Receiver<AudioFrame>>,
) -> Option<AudioFrame> {
    match audio {
        Some(frames) => frames.recv().await,
        None => std::future::pending().await,
    }
}
