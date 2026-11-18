mod media;
mod signaling;

pub(super) use signaling::{
    AUDIO_PAYLOAD, NORMAL_CLOSE, RTX_PAYLOAD, VIDEO_PAYLOAD, close_socket, commit_welcome_body,
    flag_invalid_commit_welcome, heartbeat_payload, parse_binary_seq, ready_on_prepare,
    select_protocol_payload, send_binary, send_json, send_key_package, send_ready, ws_request,
};

use signaling::send_auth;

use super::{
    audio::{AudioFlags, AudioHandle, Incoming, Outgoing},
    crypto::{Cipher, Mode},
    dave::{
        Dave, EncryptOutcome, INVALID_MLS_LIMIT, REFUSED_DOWNGRADE, REPEATED_INVALID_MLS, SILENCE,
    },
    emit, rtcp,
    rtp::Header,
    udp,
};
use crate::{Event, Secret, VoiceEvent, model, properties};
use futures::{SinkExt, StreamExt, channel::mpsc::UnboundedSender};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::watch,
    time::{Instant, sleep_until, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        handshake::client::Request,
        http::HeaderValue,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_ATTEMPTS: usize = 3;
const HELLO_TIMEOUT: Duration = Duration::from_secs(20);
// Encryption can still fail after the session starts, so starting alone is not progress.
const STABLE_CONNECTION: Duration = Duration::from_secs(30);
const SENDER_REPORT: Duration = Duration::from_secs(5);
const REIDENTIFY_CLOSES: [u16; 4] = [4004, 4006, 4011, 4015];

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
pub(super) struct Config {
    pub guild_id: u64,
    pub channel_id: u64,
    pub user_id: u64,
    pub session_id: Secret,
    pub token: Secret,
    pub server: String,
    pub ws_url: String,
}

pub(super) enum Outcome {
    Stopped,
    Reconnect,
    Fatal { reason: String, leave: bool },
}

impl Outcome {
    fn fatal(reason: impl Into<String>) -> Self {
        Self::Fatal {
            reason: reason.into(),
            leave: true,
        }
    }

    fn fatal_without_leave(reason: impl Into<String>) -> Self {
        Self::Fatal {
            reason: reason.into(),
            leave: false,
        }
    }
}

struct Speaking {
    user_id: u64,
    last: Instant,
    active: bool,
}

struct RxStats {
    datagrams: u64,
    decrypted: u64,
    dave_failed: u64,
    unknown_ssrc: u64,
    forwarded: u64,
    rtcp_logged: [bool; 32],
    next_report: Instant,
}

impl RxStats {
    fn new() -> Self {
        Self {
            datagrams: 0,
            decrypted: 0,
            dave_failed: 0,
            unknown_ssrc: 0,
            forwarded: 0,
            rtcp_logged: [false; 32],
            next_report: Instant::now() + Duration::from_secs(5),
        }
    }
}

struct TxStats {
    received: u64,
    dave_encrypted: u64,
    dave_not_ready: u64,
    dave_failed: u64,
    sent: u64,
    send_errors: u64,
    speaking_ops: u64,
    next_report: Instant,
}

impl TxStats {
    fn new() -> Self {
        Self {
            received: 0,
            dave_encrypted: 0,
            dave_not_ready: 0,
            dave_failed: 0,
            sent: 0,
            send_errors: 0,
            speaking_ops: 0,
            next_report: Instant::now() + Duration::from_secs(5),
        }
    }
}

pub(super) struct Connection {
    config: Config,
    flags: AudioFlags,
    socket: Option<UdpSocket>,
    peer: Option<std::net::SocketAddr>,
    cipher: Option<Cipher>,
    dave: Option<Dave>,
    audio: Option<AudioHandle>,
    ssrc: u32,
    seq: Option<u64>,
    ssrc_users: HashMap<u32, u64>,
    user_ssrcs: HashMap<u64, u32>,
    speaking: HashMap<u32, Speaking>,
    recognized: HashSet<u64>,
    rtp_sequence: u16,
    rtp_timestamp: u32,
    rtp_epoch: Instant,
    new_talkspurt: bool,
    audio_packets: u32,
    audio_octets: u32,
    audio_sent: Option<Instant>,
    next_sender_report: Instant,
    ping_sequence: u32,
    next_ping: Instant,
    rtc_connection_id: String,
    speaking_ssrc: Option<u32>,
    handshake_done: bool,
    established: Option<Instant>,
    stable: Option<bool>,
    dave_epoch_reset: bool,
    invalid_mls: u8,
    last_close: Option<u16>,
    rx: RxStats,
    tx: TxStats,
}

impl Connection {
    pub fn new(config: Config, flags: AudioFlags) -> Self {
        let mut recognized = HashSet::new();
        recognized.insert(config.user_id);
        Self {
            config,
            flags,
            socket: None,
            peer: None,
            cipher: None,
            dave: None,
            audio: None,
            ssrc: 0,
            seq: None,
            ssrc_users: HashMap::new(),
            user_ssrcs: HashMap::new(),
            speaking: HashMap::new(),
            recognized,
            rtp_sequence: 0,
            rtp_timestamp: 0,
            rtp_epoch: Instant::now(),
            new_talkspurt: false,
            audio_packets: 0,
            audio_octets: 0,
            audio_sent: None,
            next_sender_report: Instant::now(),
            ping_sequence: 0,
            next_ping: Instant::now(),
            rtc_connection_id: properties::uuid_v4(),
            speaking_ssrc: None,
            handshake_done: false,
            established: None,
            stable: None,
            dave_epoch_reset: false,
            invalid_mls: 0,
            last_close: None,
            rx: RxStats::new(),
            tx: TxStats::new(),
        }
    }

    pub fn can_resume(&self) -> bool {
        self.handshake_done && self.seq.is_some()
    }

    pub fn established(&self) -> bool {
        self.stable == Some(true)
    }

    /// Must run before the socket closes, or a slow close counts as uptime.
    pub(super) fn sample_stability(&mut self) {
        if self.stable.is_none() {
            self.stable = Some(
                self.established
                    .is_some_and(|since| since.elapsed() >= STABLE_CONNECTION),
            );
        }
    }

    pub(super) fn mark_established(&mut self) {
        self.established.get_or_insert_with(Instant::now);
    }

    fn forget_session(&mut self) {
        self.handshake_done = false;
        self.seq = None;
    }

    pub fn last_close(&self) -> Option<u16> {
        self.last_close
    }

    pub fn guild_id(&self) -> u64 {
        self.config.guild_id
    }

    pub fn channel_id(&self) -> u64 {
        self.config.channel_id
    }

    pub async fn attempt(
        &mut self,
        events: &UnboundedSender<Event>,
        shutdown: &mut watch::Receiver<bool>,
        resume: bool,
    ) -> Outcome {
        self.last_close = None;
        self.established = None;
        self.stable = None;
        self.rx = RxStats::new();
        self.tx = TxStats::new();
        if !resume {
            self.ssrc_users.clear();
            self.user_ssrcs.clear();
            self.speaking.clear();
        }
        self.audio.take();
        let Some(request) = ws_request(&self.config.ws_url) else {
            return Outcome::fatal("Invalid voice endpoint");
        };
        let connection = tokio::select! {
            biased;
            _ = shutdown.changed() => return Outcome::Stopped,
            result = timeout(CONNECT_TIMEOUT, connect_async(request)) => result,
        };
        let Ok(Ok((mut socket, _))) = connection else {
            return Outcome::Reconnect;
        };
        let outcome = self.drive(&mut socket, events, shutdown, resume).await;
        self.sample_stability();
        close_socket(&mut socket, NORMAL_CLOSE).await;
        outcome
    }

    async fn drive(
        &mut self,
        socket: &mut Socket,
        events: &UnboundedSender<Event>,
        shutdown: &mut watch::Receiver<bool>,
        resume: bool,
    ) -> Outcome {
        let mut hello = false;
        let hello_deadline = Instant::now() + HELLO_TIMEOUT;
        let mut interval = Duration::from_secs(60);
        let mut next_heartbeat = Instant::now() + interval;
        let mut ack_at: Option<Instant> = None;
        let mut next_tick = Instant::now() + Duration::from_millis(100);
        let mut audio_out: Option<tokio::sync::mpsc::UnboundedReceiver<Outgoing>> = None;
        let mut udp_buffer = [0u8; 2048];

        if send_auth(socket, &self.config, self.seq, resume)
            .await
            .is_err()
        {
            return Outcome::Reconnect;
        }
        loop {
            let socket_ref = &mut *socket;
            let udp = self.socket.as_ref();
            let received = tokio::select! {
                biased;
                _ = shutdown.changed() => return Outcome::Stopped,
                _ = sleep_until(hello_deadline), if !hello => return Outcome::Reconnect,
                _ = sleep_until(next_heartbeat), if hello => {
                    if ack_at.is_some_and(|sent| sent.elapsed() >= interval) {
                        return Outcome::Reconnect;
                    }
                    if send_json(socket, heartbeat_payload(self.seq)).await.is_err() {
                        return Outcome::Reconnect;
                    }
                    ack_at.get_or_insert(Instant::now());
                    next_heartbeat = Instant::now() + interval;
                    continue;
                }
                _ = sleep_until(next_tick) => {
                    self.tick(events).await;
                    next_tick = Instant::now() + Duration::from_millis(100);
                    continue;
                }
                event = next_audio(&mut audio_out) => {
                    match event {
                        Some(event) => self.audio_event(event, socket, events).await,
                        None => audio_out = None,
                    }
                    continue;
                }
                result = udp_recv(udp, &mut udp_buffer) => result,
                message = socket_ref.next() => {
                    let Some(Ok(message)) = message else {
                        return Outcome::Reconnect;
                    };
                    match message {
                        Message::Text(text) => {
                            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                                log::warn!("Malformed voice gateway payload");
                                continue;
                            };
                            if let Some(seq) = value.get("seq").and_then(Value::as_u64) {
                                self.seq = Some(seq);
                            }
                            let op = value.get("op").and_then(Value::as_u64).unwrap_or(0) as u8;
                            if op == 6 {
                                if let Some(sent) = ack_at.take() {
                                    let ping = sent.elapsed().as_millis().min(u128::from(u32::MAX));
                                    emit(events, VoiceEvent::Ping(ping as u32));
                                }
                                continue;
                            }
                            if op == 8 {
                                hello = true;
                                interval = value
                                    .get("d")
                                    .and_then(|d| d.get("heartbeat_interval"))
                                    .and_then(Value::as_u64)
                                    .filter(|ms| *ms > 0 && *ms <= 300_000)
                                    .map(Duration::from_millis)
                                    .unwrap_or(Duration::from_secs(60));
                                next_heartbeat = Instant::now() + interval;
                                continue;
                            }
                            match self.on_json(op, &value, socket, events).await {
                                Ok(()) => {}
                                Err(outcome) => return outcome,
                            }
                            if self.audio.is_some() && audio_out.is_none()
                                && let Some(audio) = &mut self.audio
                            {
                                audio_out = audio.take_outgoing();
                            }
                        }
                        Message::Binary(bytes) => {
                            if let Some(seq) = parse_binary_seq(&bytes) {
                                self.seq = Some(seq);
                            }
                            if let Err(outcome) = self.on_binary(&bytes, socket, events).await {
                                return outcome;
                            }
                            if self.audio.is_some() && audio_out.is_none()
                                && let Some(audio) = &mut self.audio
                            {
                                audio_out = audio.take_outgoing();
                            }
                        }
                        Message::Close(frame) => {
                            let code = frame.map_or(1006, |frame| u16::from(frame.code));
                            self.last_close = Some(code);
                            log::info!("Voice gateway closed: {code}");
                            if REIDENTIFY_CLOSES.contains(&code) {
                                self.forget_session();
                            }
                            return close_outcome(code);
                        }
                        Message::Ping(_)
                            if socket.flush().await.is_err() => {
                                return Outcome::Reconnect;
                            }
                        _ => {}
                    }
                    continue;
                }
            };
            let Ok((size, _)) = received else {
                continue;
            };
            if size == 0 {
                continue;
            }
            self.on_datagram(&udp_buffer[..size], events).await;
            if audio_out.is_none()
                && self.audio.is_some()
                && let Some(audio) = &mut self.audio
            {
                audio_out = audio.take_outgoing();
            }
        }
    }
}

pub(super) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn next_audio(
    receiver: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Outgoing>>,
) -> Option<Outgoing> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => futures::future::pending().await,
    }
}

async fn udp_recv(
    socket: Option<&UdpSocket>,
    buffer: &mut [u8],
) -> std::io::Result<(usize, std::net::SocketAddr)> {
    match socket {
        Some(socket) => socket.recv_from(buffer).await,
        None => futures::future::pending().await,
    }
}

fn close_outcome(code: u16) -> Outcome {
    match code {
        4014 => Outcome::fatal_without_leave("Disconnected from voice"),
        4021 => Outcome::fatal_without_leave("Voice rate limited"),
        4022 => Outcome::fatal_without_leave("The voice call ended"),
        _ => Outcome::Reconnect,
    }
}

#[cfg(test)]
mod tests;
