use super::super::{
    audio::AudioFlags,
    crypto::Cipher,
    dave::{
        Dave, INVALID_MLS_LIMIT, REFUSED_DOWNGRADE, REFUSED_DOWNGRADE_REASON, REPEATED_INVALID_MLS,
        REPEATED_INVALID_MLS_REASON,
    },
    gateway::{
        AUDIO_PAYLOAD, NORMAL_CLOSE, RTX_PAYLOAD, VIDEO_PAYLOAD, close_socket, commit_welcome_body,
        flag_invalid_commit_welcome, heartbeat_payload, parse_binary_seq, ready_on_prepare,
        send_binary, send_json, send_key_package, send_ready,
    },
    rtcp,
    rtp::Header,
    udp as discord_udp,
};
use super::{
    FrameSlot, decode, h264,
    h264::{Assembler, Packet, Reorder},
    handshake,
    handshake::{HEARTBEAT_DEFAULT, HandshakeConfig, Socket},
};
use crate::{Event, Secret, StreamEvent, VoiceEvent, model};
use futures::{StreamExt, channel::mpsc::UnboundedSender};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    net::UdpSocket,
    sync::watch,
    time::{MissedTickBehavior, interval},
};
use tokio_tungstenite::tungstenite::Message;

const RECEIVER_REPORT: Duration = Duration::from_secs(1);
const PLI_INTERVAL: Duration = Duration::from_millis(500);
const MAX_PACKET: usize = 2048;
const DEFAULT_PIXELS: u64 = 1920 * 1080;

pub(crate) struct ViewerConfig {
    pub user_id: u64,
    pub streamer_id: u64,
    pub session_id: Secret,
    pub token: Secret,
    pub endpoint: String,
    pub rtc_server_id: u64,
    pub rtc_channel_id: Option<u64>,
    pub frames: FrameSlot,
    pub events: UnboundedSender<Event>,
    pub ended: Arc<AtomicBool>,
    pub flags: AudioFlags,
    pub audio_generation: u64,
}

pub(crate) async fn run(
    config: ViewerConfig,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let handshake = handshake::handshake(
        &HandshakeConfig {
            user_id: config.user_id,
            session_id: config.session_id.clone(),
            token: config.token.clone(),
            endpoint: config.endpoint.clone(),
            rtc_server_id: config.rtc_server_id,
            rtc_channel_id: super::rtc_channel_id(config.rtc_server_id, config.rtc_channel_id)?,
            streams: super::screen_streams(),
        },
        &mut cancel,
    )
    .await?;
    let mut viewer = Viewer::new(config);
    viewer.udp_ssrc = handshake.ssrc;
    viewer.heartbeat = handshake.heartbeat;
    viewer.next_heartbeat = Instant::now() + handshake.heartbeat;
    let handshake::Handshake {
        mut socket,
        udp,
        peer,
        ..
    } = handshake;
    let mut tick = interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut buffer = [0u8; MAX_PACKET];
    let result: Result<(), String> = async {
        loop {
            tokio::select! {
                biased;
                _ = cancel.changed() => return Ok(()),
                _ = tick.tick() => viewer.tick(&mut socket, &udp, peer).await?,
                received = udp.recv_from(&mut buffer) => {
                    let Ok((size, _)) = received else { continue };
                    viewer.datagram(&udp, peer, &buffer[..size], Instant::now()).await?;
                }
                message = socket.next() => {
                    let Some(Ok(message)) = message else {
                        return Err("Discord stream socket failed".into());
                    };
                    viewer.message(&mut socket, message).await?;
                }
            }
        }
    }
    .await;
    close_socket(&mut socket, viewer.close_code).await;
    result
}

fn ssrc(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|ssrc| u32::try_from(ssrc).ok())
        .filter(|ssrc| *ssrc != 0)
}

fn stream_audio_ssrc(data: &Value) -> Option<u32> {
    ssrc(data.get("audio_ssrc"))
}

fn speaking_audio_ssrc(data: &Value, streamer_id: u64) -> Option<u32> {
    if data.get("user_id").and_then(model::id_value) != Some(streamer_id) {
        return None;
    }
    ssrc(data.get("ssrc"))
}

fn stream_ssrcs(data: &Value) -> Option<(u32, u32)> {
    let streams = data.get("streams").and_then(Value::as_array);
    let video = ssrc(data.get("video_ssrc")).or_else(|| ssrc(streams?.first()?.get("ssrc")))?;
    let rtx = ssrc(data.get("rtx_ssrc")).or_else(|| {
        streams?
            .iter()
            .find(|stream| ssrc(stream.get("ssrc")) == Some(video))
            .and_then(|stream| ssrc(stream.get("rtx_ssrc")))
    });
    Some((video, rtx.unwrap_or(0)))
}

fn stream_pixels(data: &Value, video_ssrc: u32) -> u64 {
    data.get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|stream| ssrc(stream.get("ssrc")) == Some(video_ssrc))
        .and_then(|stream| stream.get("max_resolution"))
        .map(|resolution| {
            let side = |key| resolution.get(key).and_then(Value::as_u64).unwrap_or(0);
            side("width").saturating_mul(side("height"))
        })
        .filter(|pixels| *pixels > 0)
        .unwrap_or(DEFAULT_PIXELS)
}

fn sink_wants(video_ssrc: u32, pixels: u64) -> Value {
    let ssrc = video_ssrc.to_string();
    let mut pixel_counts = serde_json::Map::new();
    pixel_counts.insert(ssrc.clone(), pixels.into());
    let mut wants = serde_json::Map::new();
    wants.insert(ssrc, 100.into());
    wants.insert("pixelCounts".into(), pixel_counts.into());
    json!({"op": 15, "d": wants})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Media {
    Audio,
    Video,
    Rtx,
}

fn media_route(
    payload_type: u8,
    ssrc: u32,
    audio_ssrc: u32,
    video_ssrc: u32,
    rtx_ssrc: u32,
) -> Option<Media> {
    match payload_type {
        AUDIO_PAYLOAD if audio_ssrc != 0 && ssrc == audio_ssrc => Some(Media::Audio),
        VIDEO_PAYLOAD if video_ssrc == 0 || ssrc == video_ssrc => Some(Media::Video),
        RTX_PAYLOAD if rtx_ssrc == 0 || ssrc == rtx_ssrc => Some(Media::Rtx),
        _ => None,
    }
}

struct Viewer {
    config: ViewerConfig,
    cipher: Option<Cipher>,
    dave: Option<Dave>,
    udp_ssrc: u32,
    audio_ssrc: u32,
    video_ssrc: u32,
    rtx_ssrc: u32,
    reorder: Reorder,
    assembler: Assembler,
    awaiting_keyframe: bool,
    need_keyframe: bool,
    next_pli: Instant,
    next_report: Instant,
    heartbeat: Duration,
    next_heartbeat: Instant,
    wants_pending: bool,
    video_pixels: u64,
    ping_sequence: u32,
    next_ping: Instant,
    invalid_mls: u8,
    close_code: u16,
    highest_sequence: u32,
    seq_ack: Option<u64>,
    lsr: u32,
    lsr_at: Option<Instant>,
    decoder: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    lost: Arc<Mutex<bool>>,
    fatal: Arc<Mutex<Option<String>>>,
    live: Arc<AtomicBool>,
}

impl Viewer {
    fn new(config: ViewerConfig) -> Self {
        Self {
            config,
            cipher: None,
            dave: None,
            udp_ssrc: 0,
            audio_ssrc: 0,
            video_ssrc: 0,
            rtx_ssrc: 0,
            reorder: Reorder::new(),
            assembler: Assembler::new(),
            awaiting_keyframe: true,
            need_keyframe: true,
            next_pli: Instant::now(),
            next_report: Instant::now() + RECEIVER_REPORT,
            heartbeat: HEARTBEAT_DEFAULT,
            next_heartbeat: Instant::now() + HEARTBEAT_DEFAULT,
            wants_pending: false,
            video_pixels: DEFAULT_PIXELS,
            ping_sequence: 0,
            next_ping: Instant::now(),
            invalid_mls: 0,
            close_code: NORMAL_CLOSE,
            highest_sequence: 0,
            seq_ack: None,
            lsr: 0,
            lsr_at: None,
            decoder: None,
            lost: Arc::new(Mutex::new(false)),
            fatal: Arc::new(Mutex::new(None)),
            live: Arc::new(AtomicBool::new(false)),
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
            3 => send_json(socket, heartbeat_payload(self.seq_ack))
                .await
                .map_err(|_| "Discord stream heartbeat failed".to_owned()),
            4 => self.session(socket, &data).await,
            5 => {
                if let Some(audio) = speaking_audio_ssrc(&data, self.config.streamer_id) {
                    self.audio_ssrc = audio;
                }
                Ok(())
            }
            6 => Ok(()),
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
                Ok(())
            }
            12 => {
                let Some(user_id) = data.get("user_id").and_then(model::id_value) else {
                    return Ok(());
                };
                if user_id != self.config.streamer_id {
                    return Ok(());
                }
                if let Some(audio) = stream_audio_ssrc(&data) {
                    self.audio_ssrc = audio;
                }
                if let Some((video, rtx)) = stream_ssrcs(&data) {
                    self.video_ssrc = video;
                    self.rtx_ssrc = rtx;
                    self.video_pixels = stream_pixels(&data, video);
                    self.wants_pending = true;
                }
                Ok(())
            }
            13 => {
                let Some(user_id) = data.get("user_id").and_then(model::id_value) else {
                    return Ok(());
                };
                if user_id == self.config.streamer_id {
                    return Err("The streamer left the call".into());
                }
                if let Some(dave) = &mut self.dave {
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
                    if !dave.prepare_transition(version, transition) {
                        return self.refuse_downgrade();
                    }
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
                if version == 0 {
                    return self.refuse_downgrade();
                }
                if self.dave.is_none() {
                    let group = self.rtc_group()?;
                    let mut dave =
                        Dave::new(version, self.config.user_id, group).ok_or_else(|| {
                            "Discord requested unsupported stream encryption".to_owned()
                        })?;
                    dave.add_user(self.config.streamer_id);
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
                    self.invalid_mls = self.invalid_mls.saturating_add(1);
                    if self.invalid_mls >= INVALID_MLS_LIMIT {
                        self.close_code = REPEATED_INVALID_MLS;
                        return Err(REPEATED_INVALID_MLS_REASON.into());
                    }
                    flag_invalid_commit_welcome(socket, self.dave.as_mut(), transition).await;
                    return Ok(());
                }
                self.invalid_mls = 0;
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
        super::rtc_channel_id(self.config.rtc_server_id, self.config.rtc_channel_id)
    }

    fn refuse_downgrade(&mut self) -> Result<(), String> {
        self.close_code = REFUSED_DOWNGRADE;
        Err(REFUSED_DOWNGRADE_REASON.into())
    }

    async fn session(&mut self, socket: &mut Socket, data: &Value) -> Result<(), String> {
        let mode = super::transport_mode(data)?;
        let version = super::dave_version(data);
        if version == 0 {
            return self.refuse_downgrade();
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
            Cipher::new(mode, &key)
                .ok_or_else(|| "Discord sent an invalid stream transport key".to_owned())?,
        );
        let group = self.rtc_group()?;
        let mut dave = Dave::new(version, self.config.user_id, group)
            .ok_or_else(|| "Discord requested unsupported stream encryption".to_owned())?;
        dave.add_user(self.config.streamer_id);
        let key_package = dave
            .key_package()
            .ok_or_else(|| "Could not create the stream encryption key".to_owned())?;
        send_key_package(socket, &key_package).await;
        self.dave = Some(dave);
        self.decoder = Some(self.start_decoder()?);
        Ok(())
    }

    fn start_decoder(&self) -> Result<std::sync::mpsc::SyncSender<Vec<u8>>, String> {
        let frames = self.config.frames.clone();
        let lost = self.lost.clone();
        let fatal = self.fatal.clone();
        let live = self.live.clone();
        let events = self.config.events.clone();
        let ended = self.config.ended.clone();
        let streamer_id = self.config.streamer_id;
        decode::spawn(
            super::decoder,
            decode::DecodeControl {
                frames,
                lost,
                fatal,
                live,
            },
            move |width, height| {
                if !ended.load(Ordering::Acquire) {
                    let _ = events.unbounded_send(Event::Voice(VoiceEvent::Stream(
                        StreamEvent::Live {
                            user_id: streamer_id,
                            width,
                            height,
                        },
                    )));
                }
            },
        )
    }

    async fn tick(
        &mut self,
        socket: &mut Socket,
        udp: &UdpSocket,
        peer: SocketAddr,
    ) -> Result<(), String> {
        let now = Instant::now();
        if let Some(reason) = self
            .fatal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(reason);
        }
        if std::mem::take(
            &mut *self
                .lost
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) {
            self.request_keyframe();
        }
        if self.need_keyframe && self.video_ssrc != 0 && now >= self.next_pli {
            self.send_feedback(udp, peer, rtcp::pli(self.udp_ssrc, self.video_ssrc))
                .await;
            self.need_keyframe = false;
            self.next_pli = now + PLI_INTERVAL;
        }
        let missing = self.reorder.missing(now);
        if !missing.is_empty() && self.video_ssrc != 0 {
            self.send_feedback(
                udp,
                peer,
                rtcp::generic_nack(self.udp_ssrc, self.video_ssrc, &missing),
            )
            .await;
        }
        let packets = self.reorder.expire(now);
        if self.reorder.take_skipped() {
            self.assembler.mark_lost();
            self.request_keyframe();
        }
        for packet in packets {
            self.packet(&packet)?;
        }
        if now >= self.next_report {
            self.send_report(udp, peer, now).await;
            self.next_report = now + RECEIVER_REPORT;
        }
        if now >= self.next_heartbeat {
            send_json(socket, heartbeat_payload(self.seq_ack))
                .await
                .map_err(|_| "Discord stream heartbeat failed".to_owned())?;
            self.next_heartbeat = now + self.heartbeat;
        }
        if now >= self.next_ping {
            let _ = udp
                .send_to(&discord_udp::ping(self.ping_sequence), peer)
                .await;
            self.ping_sequence = self.ping_sequence.wrapping_add(1);
            self.next_ping = now + discord_udp::PING_INTERVAL;
        }
        if self.wants_pending && self.cipher.is_some() && self.video_ssrc != 0 {
            send_json(socket, sink_wants(self.video_ssrc, self.video_pixels))
                .await
                .map_err(|_| "Discord stream sink update failed".to_owned())?;
            self.wants_pending = false;
        }
        Ok(())
    }

    async fn datagram(
        &mut self,
        udp: &UdpSocket,
        peer: SocketAddr,
        packet: &[u8],
        now: Instant,
    ) -> Result<(), String> {
        if discord_udp::is_ping(packet) {
            return Ok(());
        }
        if rtcp::packet_type(packet).is_some() {
            if let Some(plaintext) = self.rtcp_plaintext(packet)
                && let Some(lsr) = rtcp::sender_report_lsr(&plaintext, self.video_ssrc)
            {
                self.lsr = lsr;
                self.lsr_at = Some(now);
            }
            return Ok(());
        }
        let Some(info) = Header::parse(packet) else {
            return Ok(());
        };
        let Some(media) = media_route(
            info.header.payload_type,
            info.header.ssrc,
            self.audio_ssrc,
            self.video_ssrc,
            self.rtx_ssrc,
        ) else {
            return Ok(());
        };
        let Some(plaintext) = self
            .cipher
            .as_ref()
            .and_then(|cipher| cipher.decrypt(&packet[..info.aad_len], &packet[info.aad_len..]))
        else {
            return Ok(());
        };
        let extension = info.payload_offset - info.aad_len;
        if plaintext.len() <= extension {
            return Ok(());
        }
        let mut payload = &plaintext[extension..];
        if media == Media::Audio {
            self.audio(payload, info.header.sequence);
            return Ok(());
        }
        let mut sequence = info.header.sequence;
        if media == Media::Rtx {
            let (original, rest) = h264::depacketize_rtx(payload);
            if original == 0 || rest.is_empty() {
                return Ok(());
            }
            sequence = original;
            payload = rest;
        }
        if self.highest_sequence == 0 {
            self.highest_sequence = u32::from(sequence);
        } else {
            let difference = sequence.wrapping_sub(self.highest_sequence as u16);
            if difference < 32768 {
                self.highest_sequence = self.highest_sequence.wrapping_add(u32::from(difference));
            }
        }
        let released = self.reorder.push(
            Packet::new(
                sequence,
                info.header.timestamp,
                info.header.marker,
                payload.to_vec(),
            ),
            now,
        );
        for packet in released {
            self.packet(&packet)?;
        }
        let missing = self.reorder.missing(now);
        if !missing.is_empty() && self.video_ssrc != 0 {
            self.send_feedback(
                udp,
                peer,
                rtcp::generic_nack(self.udp_ssrc, self.video_ssrc, &missing),
            )
            .await;
        }
        Ok(())
    }

    fn audio(&mut self, payload: &[u8], sequence: u16) {
        let streamer_id = self.config.streamer_id;
        let Some(dave) = &mut self.dave else {
            return;
        };
        if !dave.can_encrypt() {
            return;
        }
        let Some(opus) = dave.decrypt(streamer_id, davey::MediaType::AUDIO, payload) else {
            return;
        };
        self.config
            .flags
            .push_stream_audio(self.config.audio_generation, sequence, opus);
    }

    fn packet(&mut self, packet: &Packet) -> Result<(), String> {
        let frame = self.assembler.push(packet);
        if self.assembler.take_lost() {
            self.request_keyframe();
        }
        let Some(frame) = frame else {
            return Ok(());
        };
        let Some(dave) = &mut self.dave else {
            self.request_keyframe();
            return Ok(());
        };
        if !dave.can_encrypt() {
            self.request_keyframe();
            return Ok(());
        }
        let Some(frame) = dave.decrypt(self.config.streamer_id, davey::MediaType::VIDEO, &frame)
        else {
            self.request_keyframe();
            return Ok(());
        };
        let keyframe = h264::is_keyframe(&frame);
        if keyframe {
            self.awaiting_keyframe = false;
            if !h264::has_parameter_sets(&frame) {
                self.request_keyframe();
            }
        } else if self.awaiting_keyframe {
            return Ok(());
        }
        let Some(decoder) = &self.decoder else {
            self.request_keyframe();
            return Ok(());
        };
        match decoder.try_send(frame) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => self.request_keyframe(),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                return Err("Stream decoder stopped".into());
            }
        }
        Ok(())
    }

    fn request_keyframe(&mut self) {
        self.awaiting_keyframe = true;
        self.need_keyframe = true;
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

    async fn send_feedback(&mut self, udp: &UdpSocket, peer: SocketAddr, clear: Vec<u8>) {
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

    async fn send_report(&mut self, udp: &UdpSocket, peer: SocketAddr, now: Instant) {
        if self.video_ssrc == 0 {
            return;
        }
        let dlsr = self.lsr_at.map_or(0, |at| {
            let elapsed = now.saturating_duration_since(at);
            (elapsed.as_micros().saturating_mul(65_536) / 1_000_000).min(u128::from(u32::MAX))
                as u32
        });
        let report = rtcp::receiver_report(
            self.udp_ssrc,
            &rtcp::ReceiverReport {
                media_ssrc: self.video_ssrc,
                fraction_lost: 0,
                cumulative_lost: 0,
                highest_sequence: self.highest_sequence,
                jitter: 0,
                lsr: self.lsr,
                dlsr,
            },
        );
        self.send_feedback(udp, peer, report).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_12_ssrcs_fall_back_to_streams() {
        assert_eq!(
            stream_ssrcs(&json!({"video_ssrc": 0, "streams": [
                {"ssrc": 700, "rtx_ssrc": 701}, {"ssrc": 710, "rtx_ssrc": 711}
            ]})),
            Some((700, 701))
        );
        assert_eq!(
            stream_ssrcs(&json!({"video_ssrc": 710, "streams": [
                {"ssrc": 700, "rtx_ssrc": 701}, {"ssrc": 710, "rtx_ssrc": 711}
            ]})),
            Some((710, 711))
        );
        assert_eq!(
            stream_ssrcs(&json!({"video_ssrc": 710, "rtx_ssrc": 712})),
            Some((710, 712))
        );
        assert_eq!(stream_ssrcs(&json!({"video_ssrc": 0, "streams": []})), None);
    }

    #[test]
    fn sink_wants_request_the_streamed_ssrc_with_its_pixel_count() {
        let op_12 = json!({"video_ssrc": 700, "streams": [
            {"ssrc": 700, "max_resolution": {"type": "fixed", "width": 1280, "height": 720}},
        ]});
        assert_eq!(stream_pixels(&op_12, 700), 1280 * 720);
        assert_eq!(stream_pixels(&op_12, 710), DEFAULT_PIXELS);
        assert_eq!(
            stream_pixels(&json!({"streams": [{"ssrc": 700}]}), 700),
            DEFAULT_PIXELS
        );
        let wants = sink_wants(700, 921_600);
        assert_eq!(wants["op"], 15);
        assert_eq!(
            wants["d"],
            json!({"700": 100, "pixelCounts": {"700": 921_600}})
        );
    }

    #[test]
    fn op_12_audio_ssrc_is_read_independently() {
        assert_eq!(stream_audio_ssrc(&json!({"audio_ssrc": 42})), Some(42));
        assert_eq!(stream_audio_ssrc(&json!({"audio_ssrc": 0})), None);
        assert_eq!(stream_audio_ssrc(&json!({})), None);
    }

    #[test]
    fn op_5_streamer_ssrc_sets_audio() {
        assert_eq!(
            speaking_audio_ssrc(&json!({"user_id": "2", "ssrc": 42}), 2),
            Some(42)
        );
        assert_eq!(
            speaking_audio_ssrc(&json!({"user_id": "3", "ssrc": 42}), 2),
            None
        );
        assert_eq!(
            speaking_audio_ssrc(&json!({"user_id": "2", "ssrc": 0}), 2),
            None
        );
        assert_eq!(speaking_audio_ssrc(&json!({"ssrc": 42}), 2), None);
    }

    #[test]
    fn audio_and_video_route_by_payload_and_ssrc() {
        assert_eq!(
            media_route(AUDIO_PAYLOAD, 42, 42, 700, 701),
            Some(Media::Audio)
        );
        assert_eq!(media_route(AUDIO_PAYLOAD, 700, 42, 700, 701), None);
        assert_eq!(
            media_route(VIDEO_PAYLOAD, 700, 42, 700, 701),
            Some(Media::Video)
        );
        assert_eq!(media_route(VIDEO_PAYLOAD, 42, 42, 700, 701), None);
        assert_eq!(
            media_route(RTX_PAYLOAD, 701, 42, 700, 701),
            Some(Media::Rtx)
        );
        assert_eq!(media_route(RTX_PAYLOAD, 42, 42, 700, 701), None);
        assert_eq!(media_route(AUDIO_PAYLOAD, 42, 0, 700, 701), None);
    }

    #[tokio::test]
    async fn non_windows_decoders_end_with_a_clear_reason() {
        if cfg!(not(windows)) {
            let error = match super::super::decoder() {
                Ok(_) => panic!("unexpected decoder on non-Windows"),
                Err(error) => error,
            };
            assert_eq!(error, "Watching streams needs Windows");
        }
    }
}
