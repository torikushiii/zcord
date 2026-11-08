use super::super::{
    audio::AudioFlags,
    crypto::{Cipher, Mode},
    dave::Dave,
    gateway::{
        self, commit_welcome_body, parse_binary_seq, ready_on_prepare, send_binary, send_json,
        send_key_package, send_ready,
    },
    rtcp,
    rtp::Header,
};
use super::{
    FrameSlot, decode, h264,
    h264::{Assembler, Packet, Reorder},
    handshake,
    handshake::{
        AUDIO_PAYLOAD, HEARTBEAT_DEFAULT, HandshakeConfig, RTX_PAYLOAD, Socket, VIDEO_PAYLOAD,
    },
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

pub(crate) struct ViewerConfig {
    pub user_id: u64,
    pub streamer_id: u64,
    pub session_id: Secret,
    pub token: Secret,
    pub endpoint: String,
    pub rtc_server_id: u64,
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
            streams: json!([]),
            encode: false,
            decode: true,
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
    sink_wants: bool,
    next_sink_wants: Instant,
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
            sink_wants: false,
            next_sink_wants: Instant::now(),
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
            let heartbeat = json!({"op": 3, "d": {
                "t": gateway::unix_ms(), "seq_ack": self.seq_ack,
            }});
            send_json(socket, heartbeat)
                .await
                .map_err(|_| "Discord stream heartbeat failed".to_owned())?;
            self.next_heartbeat = now + self.heartbeat;
        }
        if self.dave.as_ref().is_some_and(Dave::is_ready)
            && (!self.sink_wants || now >= self.next_sink_wants)
        {
            send_json(socket, json!({"op": 15, "d": {"any": 100}}))
                .await
                .map_err(|_| "Discord stream sink update failed".to_owned())?;
            self.sink_wants = true;
            self.next_sink_wants = now + Duration::from_secs(5);
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
