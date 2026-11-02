use super::{
    audio::{AudioFlags, AudioHandle, Incoming, Outgoing},
    crypto::{Cipher, Mode},
    dave::{Dave, EncryptOutcome, SILENCE},
    emit, rtcp,
    rtp::Header,
    udp,
};
use crate::discord::{Event, Secret, VoiceEvent, model, properties};
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
        Message, client::IntoClientRequest, handshake::client::Request, http::HeaderValue,
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_ATTEMPTS: usize = 3;
const KEEPALIVE: Duration = Duration::from_secs(5);

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
    last_send: Instant,
    handshake_done: bool,
    dave_epoch_reset: bool,
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
            last_send: Instant::now(),
            handshake_done: false,
            dave_epoch_reset: false,
            last_close: None,
            rx: RxStats::new(),
            tx: TxStats::new(),
        }
    }

    pub fn can_resume(&self) -> bool {
        self.handshake_done && self.seq.is_some()
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
        self.drive(&mut socket, events, shutdown, resume).await
    }

    async fn drive(
        &mut self,
        socket: &mut Socket,
        events: &UnboundedSender<Event>,
        shutdown: &mut watch::Receiver<bool>,
        resume: bool,
    ) -> Outcome {
        let mut hello = false;
        let mut interval = Duration::from_secs(60);
        let mut next_heartbeat = Instant::now() + interval;
        let mut ack_at: Option<Instant> = None;
        let mut next_tick = Instant::now() + Duration::from_millis(100);
        let mut audio_out: Option<tokio::sync::mpsc::UnboundedReceiver<Outgoing>> = None;
        let mut udp_buffer = [0u8; 2048];

        loop {
            let socket_ref = &mut *socket;
            let udp = self.socket.as_ref();
            let received = tokio::select! {
                biased;
                _ = shutdown.changed() => return Outcome::Stopped,
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
                                next_heartbeat =
                                    Instant::now() + interval.mul_f64(rand::random::<f64>().max(0.1));
                                let resumed = send_auth(socket, &self.config, self.seq, resume).await;
                                if resumed.is_err() {
                                    return Outcome::Reconnect;
                                }
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
                            self.on_binary(&bytes, socket, events).await;
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
                            return close_outcome(code);
                        }
                        Message::Ping(_) => {
                            if socket.flush().await.is_err() {
                                return Outcome::Reconnect;
                            }
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
            if audio_out.is_none() && self.audio.is_some() {
                if let Some(audio) = &mut self.audio {
                    audio_out = audio.take_outgoing();
                }
            }
        }
    }

    async fn on_json(
        &mut self,
        op: u8,
        value: &Value,
        socket: &mut Socket,
        events: &UnboundedSender<Event>,
    ) -> Result<(), Outcome> {
        let data = value.get("d").cloned().unwrap_or(Value::Null);
        match op {
            2 => {
                self.ssrc = data.get("ssrc").and_then(Value::as_u64).unwrap_or(0) as u32;
                let ip = data.get("ip").and_then(Value::as_str).unwrap_or_default();
                let port = data.get("port").and_then(Value::as_u64).unwrap_or(0) as u16;
                let modes: Vec<String> = data
                    .get("modes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|mode| mode.as_str().map(str::to_owned))
                    .collect();
                self.start_udp(ip, port, &modes, socket).await?;
            }
            4 => {
                if !self.start_session(&data, events) {
                    return Err(Outcome::fatal(
                        "Discord sent an invalid voice session description",
                    ));
                }
                if let Some(dave) = &mut self.dave
                    && let Some(key_package) = dave.key_package()
                {
                    send_key_package(socket, &key_package).await;
                }
                self.send_keepalive().await;
            }
            5 => {
                let user_id = data
                    .get("user_id")
                    .and_then(model::id_value)
                    .unwrap_or_default();
                let ssrc = data.get("ssrc").and_then(Value::as_u64).unwrap_or(0) as u32;
                if user_id != 0 {
                    self.recognized.insert(user_id);
                    if ssrc != 0 {
                        self.ssrc_users.insert(ssrc, user_id);
                        self.user_ssrcs.insert(user_id, ssrc);
                    }
                    if let Some(dave) = &mut self.dave {
                        dave.add_user(user_id);
                    }
                }
            }
            6 => {}
            9 => {
                if data.as_bool() == Some(false) || self.cipher.is_none() || self.socket.is_none() {
                    self.handshake_done = false;
                    return Err(Outcome::Reconnect);
                }
                self.handshake_done = true;
                self.rtp_epoch = Instant::now();
                if self.audio.is_none() {
                    self.audio = Some(AudioHandle::start(self.flags.clone()));
                }
                self.send_keepalive().await;
            }
            11 => {
                if let Some(ids) = data.get("user_ids").and_then(Value::as_array) {
                    for id in ids.iter().filter_map(model::id_value) {
                        self.recognized.insert(id);
                        if let Some(dave) = &mut self.dave {
                            dave.add_user(id);
                        }
                    }
                }
            }
            13 => {
                if let Some(user_id) = data.get("user_id").and_then(model::id_value) {
                    self.recognized.remove(&user_id);
                    if let Some(ssrc) = self.user_ssrcs.remove(&user_id) {
                        self.ssrc_users.remove(&ssrc);
                        if let Some(state) = self.speaking.remove(&ssrc)
                            && state.active
                        {
                            emit(
                                events,
                                VoiceEvent::Speaking {
                                    user_id,
                                    speaking: false,
                                },
                            );
                        }
                    }
                    if let Some(dave) = &mut self.dave {
                        dave.remove_user(user_id);
                    }
                }
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
                let mut executed = false;
                let mut ready_sent = false;
                if let Some(dave) = &mut self.dave {
                    dave.prepare_transition(version, transition);
                    if transition == 0 {
                        if let Some(code) = dave.execute_transition(0) {
                            emit(events, VoiceEvent::PrivacyCode(code));
                        }
                        self.dave_epoch_reset = false;
                        executed = true;
                    } else if ready_on_prepare(transition, version, self.dave_epoch_reset) {
                        ready_sent = true;
                        send_ready(socket, transition, version).await;
                    }
                }
                log::info!(
                    "DAVE rx op=21 prepare transition={transition} version={version} -> executed={executed} ready_sent={ready_sent}"
                );
            }
            22 => {
                let transition = data
                    .get("transition_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                let (version, ready, code) = match &mut self.dave {
                    Some(dave) => {
                        let code = dave.execute_transition(transition);
                        if code.is_some() {
                            self.dave_epoch_reset = false;
                        }
                        (dave.version(), dave.is_ready(), code)
                    }
                    None => (0, false, None),
                };
                log::info!(
                    "DAVE rx op=22 execute transition={transition} -> version={version} ready={ready} privacy_code={}",
                    code.as_ref().is_some_and(|code| code.is_some())
                );
                if let Some(code) = code {
                    emit(events, VoiceEvent::PrivacyCode(code));
                }
            }
            24 => {
                let version = data
                    .get("protocol_version")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16;
                let epoch = data.get("epoch").and_then(Value::as_u64).unwrap_or(0);
                self.dave_epoch_reset = epoch == 1;
                if version > 0 && self.dave.is_none() {
                    self.dave = Dave::new(version, self.config.user_id, self.config.channel_id);
                    if let Some(dave) = &mut self.dave {
                        dave.mark_upgrade();
                        for user_id in &self.recognized {
                            dave.add_user(*user_id);
                        }
                    }
                }
                let mut key_package_len = 0;
                if let Some(dave) = &mut self.dave
                    && let Some(key_package) = dave.prepare_epoch(version, epoch)
                {
                    key_package_len = key_package.len();
                    send_key_package(socket, &key_package).await;
                }
                log::info!(
                    "DAVE rx op=24 epoch={epoch} version={version} -> key_package_len={key_package_len}"
                );
            }
            3 => {
                let _ = send_json(
                    socket,
                    json!({"op": 3, "d": {"t": unix_ms(), "seq_ack": self.seq}}),
                )
                .await;
            }
            _ => log::debug!("Ignoring voice op {op}"),
        }
        Ok(())
    }

    async fn on_binary(
        &mut self,
        bytes: &[u8],
        socket: &mut Socket,
        events: &UnboundedSender<Event>,
    ) {
        if bytes.len() < 3 {
            return;
        }
        let op = bytes[2];
        let payload = &bytes[3..];
        match op {
            25 => {
                let ok = self
                    .dave
                    .as_mut()
                    .and_then(|dave| dave.set_external_sender(payload))
                    .is_some();
                log::info!(
                    "DAVE rx op=25 external_sender len={} -> ok={ok}",
                    payload.len()
                );
            }
            27 => {
                if payload.is_empty() {
                    log::info!("DAVE rx op=27 proposals len=0 -> ok=false");
                    return;
                }
                let operation = payload[0];
                let proposals_len = payload.len() - 1;
                let mut commit_len = 0;
                let mut welcome_len = 0;
                let mut sent = false;
                if let Some(dave) = &mut self.dave
                    && let Some((commit, welcome)) = dave.proposals(operation, &payload[1..])
                {
                    commit_len = commit.len();
                    welcome_len = welcome.as_ref().map_or(0, Vec::len);
                    let body = commit_welcome_body(commit, welcome);
                    log::info!(
                        "DAVE tx op=28 commit_welcome commit_len={commit_len} welcome_len={welcome_len} total_len={}",
                        body.len()
                    );
                    send_binary(socket, 28, &body).await;
                    sent = true;
                }
                log::info!(
                    "DAVE rx op=27 proposals operation={operation} len={proposals_len} -> commit_len={commit_len} welcome_len={welcome_len} sent={sent}"
                );
            }
            29 | 30 => {
                if payload.len() < 2 {
                    return;
                }
                let transition = u16::from_be_bytes([payload[0], payload[1]]);
                let body = &payload[2..];
                let result = if op == 29 {
                    self.dave
                        .as_mut()
                        .is_some_and(|dave| dave.commit(transition, body))
                } else {
                    self.dave
                        .as_mut()
                        .is_some_and(|dave| dave.welcome(transition, body))
                };
                let label = if op == 29 { "commit" } else { "welcome" };
                if !result {
                    let (version, ready) = self
                        .dave
                        .as_ref()
                        .map_or((0, false), |dave| (dave.version(), dave.is_ready()));
                    log::info!(
                        "DAVE rx op={op} {label} transition={transition} len={} -> ok=false version={version} ready={ready}",
                        body.len()
                    );
                    if transition != 0 {
                        log::info!("DAVE tx op=31 invalid_commit_welcome transition={transition}");
                        let _ = send_json(
                            socket,
                            json!({"op": 31, "d": {"transition_id": transition}}),
                        )
                        .await;
                    }
                    self.dave_epoch_reset = true;
                    if let Some(dave) = &mut self.dave {
                        let version = dave.version();
                        if let Some(key_package) = dave.prepare_epoch(version, 1) {
                            send_key_package(socket, &key_package).await;
                        }
                    }
                    return;
                }
                if transition == 0 {
                    let mut privacy_code = false;
                    if let Some(dave) = &mut self.dave
                        && let Some(code) = dave.execute_transition(0)
                    {
                        privacy_code = code.is_some();
                        emit(events, VoiceEvent::PrivacyCode(code));
                    }
                    self.dave_epoch_reset = false;
                    let (version, ready) = self
                        .dave
                        .as_ref()
                        .map_or((0, false), |dave| (dave.version(), dave.is_ready()));
                    log::info!(
                        "DAVE rx op={op} {label} transition=0 len={} -> ok=true version={version} ready={ready} privacy_code={privacy_code}",
                        body.len()
                    );
                } else {
                    let version = self
                        .dave
                        .as_ref()
                        .and_then(|dave| dave.transition_version(transition))
                        .or_else(|| self.dave.as_ref().map(Dave::version))
                        .unwrap_or(0);
                    let ready = self.dave.as_ref().is_some_and(Dave::is_ready);
                    log::info!(
                        "DAVE rx op={op} {label} transition={transition} len={} -> ok=true version={version} ready={ready}",
                        body.len()
                    );
                    send_ready(socket, transition, version).await;
                }
            }
            _ => log::debug!("Ignoring voice binary op {op}"),
        }
    }

    async fn start_udp(
        &mut self,
        ip: &str,
        port: u16,
        modes: &[String],
        socket: &mut Socket,
    ) -> Result<(), Outcome> {
        let Some(mode) = Mode::preferred(modes) else {
            return Err(Outcome::fatal(
                "Discord offered no supported voice encryption mode",
            ));
        };
        let Ok(udp) = UdpSocket::bind("0.0.0.0:0").await else {
            return Err(Outcome::fatal("Could not open the voice UDP socket"));
        };
        let mut buffer = [0u8; 128];
        let mut discovered = None;
        for _ in 0..DISCOVERY_ATTEMPTS {
            if udp
                .send_to(&udp::discovery_request(self.ssrc), (ip, port))
                .await
                .is_err()
            {
                continue;
            }
            let Ok(Ok((size, _))) = timeout(DISCOVERY_TIMEOUT, udp.recv_from(&mut buffer)).await
            else {
                continue;
            };
            if let Some(found) = udp::parse_discovery_response(&buffer[..size]) {
                discovered = Some(found);
                break;
            }
        }
        let Some((external_address, external_port)) = discovered else {
            return Err(Outcome::fatal("Could not discover the voice UDP endpoint"));
        };
        if let Ok(peer) = tokio::net::lookup_host((ip, port)).await {
            self.peer = peer.into_iter().next();
        }
        self.socket = Some(udp);
        send_json(
            socket,
            select_protocol_payload(&external_address, external_port, mode),
        )
        .await
        .map_err(|_| Outcome::Reconnect)?;
        Ok(())
    }

    fn start_session(&mut self, data: &Value, events: &UnboundedSender<Event>) -> bool {
        let Some(mode) = data
            .get("mode")
            .and_then(Value::as_str)
            .and_then(Mode::from_name)
        else {
            return false;
        };
        let Some(key) = data.get("secret_key").and_then(Value::as_array) else {
            return false;
        };
        let key: Vec<u8> = key
            .iter()
            .filter_map(|byte| byte.as_u64())
            .map(|b| b as u8)
            .collect();
        let Some(cipher) = Cipher::new(mode, &key) else {
            return false;
        };
        self.cipher = Some(cipher);
        // Discord sends op 5 for users who are already speaking during the handshake,
        // before op 4. Clearing here would lose those SSRC mappings.
        let version = data
            .get("dave_protocol_version")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u16;
        self.dave = Dave::new(version, self.config.user_id, self.config.channel_id);
        if let Some(dave) = &mut self.dave {
            for user_id in &self.recognized {
                dave.add_user(*user_id);
            }
        }
        log::info!("Voice connected: mode={} dave={}", mode.name(), version);
        if self.dave.is_none() {
            emit(events, VoiceEvent::PrivacyCode(None));
        }
        self.rtp_epoch = Instant::now();
        self.rtp_sequence = 0;
        self.rtp_timestamp = 0;
        self.new_talkspurt = false;
        self.handshake_done = true;
        self.dave_epoch_reset = version > 0;
        if self.audio.is_none() {
            self.audio = Some(AudioHandle::start(self.flags.clone()));
        }
        emit(events, VoiceEvent::Server(self.config.server.clone()));
        emit(
            events,
            VoiceEvent::Connected {
                guild_id: self.config.guild_id,
                channel_id: self.config.channel_id,
            },
        );
        true
    }

    async fn audio_event(
        &mut self,
        event: Outgoing,
        socket: &mut Socket,
        events: &UnboundedSender<Event>,
    ) {
        match event {
            Outgoing::Speaking(true) => {
                self.new_talkspurt = true;
                self.send_speaking(socket, 1).await;
                emit(
                    events,
                    VoiceEvent::Speaking {
                        user_id: self.config.user_id,
                        speaking: true,
                    },
                );
            }
            Outgoing::Speaking(false) => {
                self.send_speaking(socket, 0).await;
                emit(
                    events,
                    VoiceEvent::Speaking {
                        user_id: self.config.user_id,
                        speaking: false,
                    },
                );
            }
            Outgoing::Frame(packet) => {
                self.tx.received = self.tx.received.saturating_add(1);
                self.send_audio(&packet).await;
            }
        }
    }

    async fn send_speaking(&mut self, socket: &mut Socket, flags: u32) {
        if send_json(socket, speaking_payload(self.ssrc, flags))
            .await
            .is_ok()
        {
            self.tx.speaking_ops = self.tx.speaking_ops.saturating_add(1);
        }
    }

    async fn send_audio(&mut self, opus: &[u8]) {
        let (Some(socket_udp), Some(cipher)) = (&self.socket, &mut self.cipher) else {
            return;
        };
        let mut payload = opus.to_vec();
        if let Some(dave) = &mut self.dave {
            match dave.encrypt(opus) {
                EncryptOutcome::Plaintext(data) => payload = data,
                EncryptOutcome::Encrypted(data) => {
                    payload = data;
                    self.tx.dave_encrypted = self.tx.dave_encrypted.saturating_add(1);
                }
                EncryptOutcome::NotReady => {
                    self.tx.dave_not_ready = self.tx.dave_not_ready.saturating_add(1);
                    return;
                }
                EncryptOutcome::Failed => {
                    self.tx.dave_failed = self.tx.dave_failed.saturating_add(1);
                    return;
                }
            }
        }
        let now = Instant::now();
        let timestamp = if self.new_talkspurt {
            now.duration_since(self.rtp_epoch)
                .as_micros()
                .saturating_mul(48)
                .checked_div(1000)
                .unwrap_or(0) as u32
        } else {
            self.rtp_timestamp.wrapping_add(960)
        };
        let header = Header::audio(self.ssrc, self.rtp_sequence, timestamp, self.new_talkspurt);
        let aad = header.encode();
        let Some(encrypted) = cipher.encrypt(&aad, &payload) else {
            return;
        };
        let mut packet = aad.to_vec();
        packet.extend_from_slice(&encrypted);
        let sent = if let Some(peer) = self.peer {
            match socket_udp.send_to(&packet, peer).await {
                Ok(_) => {
                    self.tx.sent = self.tx.sent.saturating_add(1);
                    true
                }
                Err(_) => {
                    self.tx.send_errors = self.tx.send_errors.saturating_add(1);
                    false
                }
            }
        } else {
            self.tx.send_errors = self.tx.send_errors.saturating_add(1);
            false
        };
        if sent {
            self.rtp_timestamp = timestamp;
            self.rtp_sequence = self.rtp_sequence.wrapping_add(1);
            self.new_talkspurt = false;
        }
        self.last_send = now;
    }

    async fn send_keepalive(&mut self) {
        let timestamp = self.rtp_timestamp.wrapping_add(960);
        let mut payload = SILENCE.to_vec();
        if let Some(dave) = &mut self.dave {
            match dave.encrypt(&SILENCE) {
                EncryptOutcome::Plaintext(data) => payload = data,
                EncryptOutcome::Encrypted(data) => {
                    payload = data;
                    self.tx.dave_encrypted = self.tx.dave_encrypted.saturating_add(1);
                }
                EncryptOutcome::NotReady => {
                    self.tx.dave_not_ready = self.tx.dave_not_ready.saturating_add(1);
                    self.last_send = Instant::now();
                    return;
                }
                EncryptOutcome::Failed => {
                    self.tx.dave_failed = self.tx.dave_failed.saturating_add(1);
                    self.last_send = Instant::now();
                    return;
                }
            }
        }
        let (Some(udp), Some(peer), Some(cipher)) = (&self.socket, self.peer, &mut self.cipher)
        else {
            return;
        };
        let header = Header::audio(self.ssrc, self.rtp_sequence, timestamp, false);
        let aad = header.encode();
        if let Some(encrypted) = cipher.encrypt(&aad, &payload) {
            let mut packet = aad.to_vec();
            packet.extend_from_slice(&encrypted);
            match udp.send_to(&packet, peer).await {
                Ok(_) => {
                    self.tx.sent = self.tx.sent.saturating_add(1);
                    self.rtp_timestamp = timestamp;
                    self.rtp_sequence = self.rtp_sequence.wrapping_add(1);
                }
                Err(_) => {
                    self.tx.send_errors = self.tx.send_errors.saturating_add(1);
                }
            }
        }
        self.last_send = Instant::now();
    }

    async fn tick(&mut self, events: &UnboundedSender<Event>) {
        if let Some(audio) = &mut self.audio {
            audio.maintain();
        }
        let now = Instant::now();
        if self.handshake_done && now >= self.rx.next_report {
            log::debug!(
                "voice rx: datagrams={} decrypted={} dave_failed={} unknown_ssrc={} forwarded={}",
                self.rx.datagrams,
                self.rx.decrypted,
                self.rx.dave_failed,
                self.rx.unknown_ssrc,
                self.rx.forwarded
            );
            self.rx.datagrams = 0;
            self.rx.decrypted = 0;
            self.rx.dave_failed = 0;
            self.rx.unknown_ssrc = 0;
            self.rx.forwarded = 0;
            self.rx.next_report = now + Duration::from_secs(5);
        }
        if self.handshake_done && now >= self.tx.next_report {
            let (mic_frames, gate_open, encoded) = self
                .audio
                .as_ref()
                .map_or((0, 0, 0), AudioHandle::take_input_stats);
            log::debug!(
                "voice tx: mic_frames={mic_frames} gate_open={gate_open} encoded={encoded} received={} dave_encrypted={} dave_not_ready={} dave_failed={} sent={} send_errors={} speaking_ops={}",
                self.tx.received,
                self.tx.dave_encrypted,
                self.tx.dave_not_ready,
                self.tx.dave_failed,
                self.tx.sent,
                self.tx.send_errors,
                self.tx.speaking_ops
            );
            self.tx.received = 0;
            self.tx.dave_encrypted = 0;
            self.tx.dave_not_ready = 0;
            self.tx.dave_failed = 0;
            self.tx.sent = 0;
            self.tx.send_errors = 0;
            self.tx.speaking_ops = 0;
            self.tx.next_report = now + Duration::from_secs(5);
        }
        for state in self.speaking.values_mut() {
            if state.active && now.duration_since(state.last) >= Duration::from_millis(250) {
                state.active = false;
                emit(
                    events,
                    VoiceEvent::Speaking {
                        user_id: state.user_id,
                        speaking: false,
                    },
                );
            }
        }
        if self.last_send.elapsed() >= KEEPALIVE {
            self.send_keepalive().await;
        }
    }

    async fn on_datagram(&mut self, packet: &[u8], events: &UnboundedSender<Event>) {
        self.rx.datagrams = self.rx.datagrams.saturating_add(1);
        if let Some(packet_type) = rtcp::packet_type(packet) {
            self.on_rtcp(packet_type, packet, events);
            return;
        }
        let Some(info) = Header::parse(packet) else {
            return;
        };
        if info.header.payload_type != 120 || info.header.ssrc == self.ssrc {
            return;
        }
        let Some(cipher) = &self.cipher else {
            return;
        };
        let Some(plaintext) = cipher.decrypt(&packet[..info.aad_len], &packet[info.aad_len..])
        else {
            return;
        };
        self.rx.decrypted = self.rx.decrypted.saturating_add(1);
        let extension_len = info.payload_offset - info.aad_len;
        if plaintext.len() <= extension_len {
            return;
        }
        let mut payload = plaintext[extension_len..].to_vec();
        let mapped = self.ssrc_users.get(&info.header.ssrc).copied();
        if self.dave.is_some() && mapped.is_none() {
            self.rx.unknown_ssrc = self.rx.unknown_ssrc.saturating_add(1);
        }
        if let Some(dave) = &mut self.dave {
            let Some(user_id) = mapped else {
                return;
            };
            let Some(decrypted) = dave.decrypt(user_id, davey::MediaType::AUDIO, &payload) else {
                self.rx.dave_failed = self.rx.dave_failed.saturating_add(1);
                return;
            };
            payload = decrypted;
        }
        let silent = payload == SILENCE;
        if let Some(user_id) = mapped {
            let state = self
                .speaking
                .entry(info.header.ssrc)
                .or_insert_with(|| Speaking {
                    user_id,
                    last: Instant::now(),
                    active: false,
                });
            state.last = Instant::now();
            if !silent && !state.active {
                state.active = true;
                emit(
                    events,
                    VoiceEvent::Speaking {
                        user_id,
                        speaking: true,
                    },
                );
            }
        }
        if let Some(audio) = &self.audio
            && audio
                .incoming
                .send(Incoming {
                    ssrc: info.header.ssrc,
                    sequence: info.header.sequence,
                    opus: payload,
                })
                .is_ok()
        {
            self.rx.forwarded = self.rx.forwarded.saturating_add(1);
        }
    }

    fn on_rtcp(&mut self, packet_type: u8, packet: &[u8], events: &UnboundedSender<Event>) {
        // Discord keeps the RTCP header and sender SSRC clear and encrypts the rest.
        // https://docs.discord.food/topics/voice-connections#sending-and-receiving-media
        let plaintext = if packet.len() >= 8 {
            self.cipher
                .as_ref()
                .and_then(|cipher| cipher.decrypt(&packet[..8], &packet[8..]))
        } else {
            None
        };
        let index = usize::from(packet_type - 192);
        if !self.rx.rtcp_logged[index] {
            self.rx.rtcp_logged[index] = true;
            log::info!(
                "voice rtcp: type={packet_type} len={} decrypted={}",
                packet.len(),
                plaintext.is_some()
            );
        }
        let Some(plaintext) = plaintext else {
            return;
        };
        let mut decrypted = packet[..8].to_vec();
        decrypted.extend_from_slice(&plaintext);
        let Some(block) = rtcp::report_block_for(&decrypted, self.ssrc) else {
            return;
        };
        log::debug!(
            "voice rtcp loss: ssrc={} fraction={} cumulative={}",
            block.ssrc,
            block.fraction_lost,
            block.cumulative_lost
        );
        emit(events, VoiceEvent::PacketLoss(block.loss_percent()));
    }
}

pub(super) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn ws_request(url: &str) -> Option<Request> {
    let mut request = url.into_client_request().ok()?;
    for (name, value) in [
        ("user-agent", properties::user_agent()),
        ("origin", "https://discord.com"),
        ("accept-language", properties::ACCEPT_LANGUAGE),
    ] {
        request
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    Some(request)
}

pub(super) async fn send_json(socket: &mut Socket, value: Value) -> Result<(), ()> {
    timeout(SEND_TIMEOUT, socket.send(Message::Text(value.to_string())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

pub(super) fn binary_frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 1);
    frame.push(op);
    frame.extend_from_slice(payload);
    frame
}

pub(super) fn commit_welcome_body(commit: Vec<u8>, welcome: Option<Vec<u8>>) -> Vec<u8> {
    let mut body = commit;
    if let Some(welcome) = welcome {
        body.extend_from_slice(&welcome);
    }
    body
}

pub(super) fn ready_payload(transition_id: u16) -> Value {
    json!({"op": 23, "d": {"transition_id": transition_id}})
}

pub(super) fn ready_on_prepare(transition: u16, version: u16, epoch_reset: bool) -> bool {
    transition != 0 && (version == 0 || !epoch_reset)
}

pub(super) async fn send_binary(socket: &mut Socket, op: u8, payload: &[u8]) {
    let _ = timeout(
        SEND_TIMEOUT,
        socket.send(Message::Binary(binary_frame(op, payload))),
    )
    .await;
}

pub(super) async fn send_key_package(socket: &mut Socket, key_package: &[u8]) {
    log::info!("DAVE tx op=26 key_package len={}", key_package.len());
    send_binary(socket, 26, key_package).await;
}

pub(super) async fn send_ready(socket: &mut Socket, transition_id: u16, version: u16) {
    log::info!("DAVE tx op=23 ready transition={transition_id} version={version}");
    let _ = send_json(socket, ready_payload(transition_id)).await;
}

async fn send_auth(
    socket: &mut Socket,
    config: &Config,
    seq: Option<u64>,
    resume: bool,
) -> Result<(), ()> {
    if resume && seq.is_some() {
        return send_json(socket, resume_payload(config, seq)).await;
    }
    send_json(socket, identify_payload(config)).await
}

fn identify_payload(config: &Config) -> Value {
    json!({"op": 0, "d": {
        "server_id": config.guild_id.to_string(),
        "channel_id": config.channel_id.to_string(),
        "user_id": config.user_id.to_string(),
        "session_id": config.session_id.expose(),
        "token": config.token.expose(),
        "video": false,
        "streams": [],
        "max_dave_protocol_version": davey::DAVE_PROTOCOL_VERSION,
    }})
}

fn resume_payload(config: &Config, seq: Option<u64>) -> Value {
    json!({"op": 7, "d": {
        "server_id": config.guild_id.to_string(),
        "channel_id": config.channel_id.to_string(),
        "session_id": config.session_id.expose(),
        "token": config.token.expose(),
        "seq_ack": seq,
    }})
}

fn heartbeat_payload(seq: Option<u64>) -> Value {
    json!({"op": 3, "d": {"t": unix_ms(), "seq_ack": seq.unwrap_or(0)}})
}

fn speaking_payload(ssrc: u32, flags: u32) -> Value {
    json!({"op": 5, "d": {"speaking": flags, "delay": 0, "ssrc": ssrc}})
}

fn select_protocol_payload(address: &str, port: u16, mode: Mode) -> Value {
    json!({"op": 1, "d": {
        "protocol": "udp",
        "data": {"address": address, "port": port, "mode": mode.name()},
        "rtc_connection_id": properties::uuid_v4(),
        "codecs": [{"name": "opus", "type": "audio", "priority": 1000, "payload_type": 120}],
    }})
}

pub(super) fn parse_binary_seq(bytes: &[u8]) -> Option<u64> {
    (bytes.len() >= 2).then(|| u64::from(u16::from_be_bytes([bytes[0], bytes[1]])))
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
        4006 | 4009 => Outcome::fatal("The voice session expired"),
        4016 => Outcome::fatal("Discord rejected the voice encryption mode"),
        4017 => Outcome::fatal("This channel requires end-to-end encryption"),
        _ => Outcome::Reconnect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::protocol::CloseFrame};

    fn config() -> Config {
        Config {
            guild_id: 10,
            channel_id: 20,
            user_id: 30,
            session_id: Secret("fixture-session".into()),
            token: Secret("fixture-token".into()),
            server: "fixture.invalid".into(),
            ws_url: "wss://fixture.invalid/?v=9".into(),
        }
    }

    #[test]
    fn early_ssrc_mapping_survives_the_session_description() {
        let mut connection = Connection::new(config(), AudioFlags::new());
        connection.ssrc_users.insert(4242, 30);
        connection.user_ssrcs.insert(30, 4242);
        let (events, _receiver) = futures::channel::mpsc::unbounded();
        let session = json!({
            "mode": "aead_aes256_gcm_rtpsize",
            "secret_key": vec![0u8; 32],
            "dave_protocol_version": 1,
        });
        assert!(connection.start_session(&session, &events));
        assert_eq!(connection.ssrc_users.get(&4242), Some(&30));
        assert_eq!(connection.user_ssrcs.get(&30), Some(&4242));
    }

    #[test]
    fn dave_framing_helpers_match_gateway() {
        assert!(!ready_on_prepare(0, 0, true));
        assert!(ready_on_prepare(1, 0, true));
        assert!(ready_on_prepare(1, 1, false));
        assert!(!ready_on_prepare(1, 1, true));

        let ready = ready_payload(7);
        assert_eq!(ready["op"], 23);
        assert_eq!(ready["d"]["transition_id"], 7);
        assert!(ready["d"].get("protocol_version").is_none());

        assert_eq!(binary_frame(26, &[1, 2]), vec![26, 1, 2]);
        assert_eq!(binary_frame(28, &[3, 4]), vec![28, 3, 4]);

        assert_eq!(
            commit_welcome_body(vec![1, 2], Some(vec![3, 4])),
            vec![1, 2, 3, 4]
        );
        assert_eq!(commit_welcome_body(vec![1, 2], None), vec![1, 2]);
    }

    #[test]
    fn close_codes_choose_whether_to_leave() {
        for code in [4014, 4021, 4022] {
            assert!(matches!(
                close_outcome(code),
                Outcome::Fatal { leave: false, .. }
            ));
        }
        for code in [4006, 4009, 4016, 4017] {
            assert!(matches!(
                close_outcome(code),
                Outcome::Fatal { leave: true, .. }
            ));
        }
        assert!(matches!(close_outcome(4015), Outcome::Reconnect));
    }

    #[test]
    fn gateway_payload_shapes() {
        let identify = identify_payload(&config());
        assert_eq!(identify["op"], 0);
        assert_eq!(identify["d"]["server_id"], "10");
        assert_eq!(identify["d"]["channel_id"], "20");
        assert_eq!(identify["d"]["user_id"], "30");
        assert_eq!(identify["d"]["session_id"], "fixture-session");
        assert_eq!(identify["d"]["video"], false);
        assert_eq!(identify["d"]["streams"], json!([]));
        assert_eq!(
            identify["d"]["max_dave_protocol_version"],
            davey::DAVE_PROTOCOL_VERSION
        );

        let heartbeat = heartbeat_payload(Some(7));
        assert_eq!(heartbeat["op"], 3);
        assert!(heartbeat["d"]["t"].as_u64().unwrap() > 0);
        assert_eq!(heartbeat["d"]["seq_ack"], 7);

        let speaking = speaking_payload(99, 1);
        assert_eq!(speaking["op"], 5);
        assert_eq!(speaking["d"]["speaking"], 1);
        assert_eq!(speaking["d"]["ssrc"], 99);
        assert_eq!(speaking["d"]["delay"], 0);

        let select = select_protocol_payload("198.51.100.7", 50000, Mode::Aes256Gcm);
        assert_eq!(select["op"], 1);
        assert_eq!(select["d"]["protocol"], "udp");
        assert_eq!(select["d"]["data"]["address"], "198.51.100.7");
        assert_eq!(select["d"]["data"]["port"], 50000);
        assert_eq!(select["d"]["data"]["mode"], "aead_aes256_gcm_rtpsize");
        assert_eq!(select["d"]["codecs"][0]["payload_type"], 120);
        assert_eq!(select["d"]["codecs"][0]["priority"], 1000);
        assert_eq!(select["d"]["rtc_connection_id"].as_str().unwrap().len(), 36);
    }

    #[tokio::test]
    async fn local_handshake_reaches_select_protocol() {
        let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_address = ws_listener.local_addr().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_address = udp.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (tcp, _) = ws_listener.accept().await.unwrap();
            let mut socket = accept_async(tcp).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"op": 8, "d": {"v": 9, "heartbeat_interval": 60000}}).to_string(),
                ))
                .await
                .unwrap();
            let identify: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(identify["op"], 0);
            assert_eq!(identify["d"]["channel_id"], "20");
            assert_eq!(identify["d"]["max_dave_protocol_version"], 1);
            socket
                .send(Message::Text(
                    json!({"op": 2, "d": {
                        "ssrc": 4242,
                        "ip": "127.0.0.1",
                        "port": udp_address.port(),
                        "modes": ["aead_aes256_gcm_rtpsize", "aead_xchacha20_poly1305_rtpsize"],
                        "experiments": [],
                        "streams": [],
                    }})
                    .to_string(),
                ))
                .await
                .unwrap();

            let mut buffer = [0u8; 128];
            let (size, peer) = udp.recv_from(&mut buffer).await.unwrap();
            assert_eq!(size, 74);
            assert_eq!(&buffer[..2], &[0, 1]);
            assert_eq!(&buffer[4..8], &4242u32.to_be_bytes());
            udp.send_to(&udp::discovery_response("198.51.100.2", 50000), peer)
                .await
                .unwrap();

            let select: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(select["op"], 1);
            assert_eq!(select["d"]["data"]["address"], "198.51.100.2");
            assert_eq!(select["d"]["data"]["port"], 50000);
            assert_eq!(select["d"]["data"]["mode"], "aead_aes256_gcm_rtpsize");
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: 4014.into(),
                    reason: "fixture".into(),
                })))
                .await
                .unwrap();
        });

        let (mut socket, _) = connect_async(format!("ws://{ws_address}")).await.unwrap();
        let (events, _receiver) = futures::channel::mpsc::unbounded();
        let (_shutdown, mut shutdown) = watch::channel(false);
        let mut connection = Connection::new(
            Config {
                ws_url: format!("ws://{ws_address}"),
                ..config()
            },
            AudioFlags::new(),
        );
        let outcome = timeout(
            Duration::from_secs(10),
            connection.drive(&mut socket, &events, &mut shutdown, false),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Outcome::Fatal { .. }));
        // Media goes to the voice server's UDP port, not the external port discovery reported.
        assert_eq!(connection.peer, Some(udp_address));
        server.await.unwrap();
    }
}
