use super::*;

impl Connection {
    pub(super) async fn on_json(
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

    pub(super) async fn on_binary(
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

    pub(super) fn start_session(&mut self, data: &Value, events: &UnboundedSender<Event>) -> bool {
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

    pub(super) async fn send_speaking(&mut self, socket: &mut Socket, flags: u32) {
        if send_json(socket, speaking_payload(self.ssrc, flags))
            .await
            .is_ok()
        {
            self.tx.speaking_ops = self.tx.speaking_ops.saturating_add(1);
        }
    }
}

pub(in crate::voice) fn ws_request(url: &str) -> Option<Request> {
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

pub(in crate::voice) async fn send_json(socket: &mut Socket, value: Value) -> Result<(), ()> {
    timeout(SEND_TIMEOUT, socket.send(Message::Text(value.to_string())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

pub(in crate::voice) fn binary_frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 1);
    frame.push(op);
    frame.extend_from_slice(payload);
    frame
}

pub(in crate::voice) fn commit_welcome_body(commit: Vec<u8>, welcome: Option<Vec<u8>>) -> Vec<u8> {
    let mut body = commit;
    if let Some(welcome) = welcome {
        body.extend_from_slice(&welcome);
    }
    body
}

pub(in crate::voice) fn ready_payload(transition_id: u16) -> Value {
    json!({"op": 23, "d": {"transition_id": transition_id}})
}

pub(in crate::voice) fn ready_on_prepare(transition: u16, version: u16, epoch_reset: bool) -> bool {
    transition != 0 && (version == 0 || !epoch_reset)
}

pub(in crate::voice) async fn send_binary(socket: &mut Socket, op: u8, payload: &[u8]) {
    let _ = timeout(
        SEND_TIMEOUT,
        socket.send(Message::Binary(binary_frame(op, payload))),
    )
    .await;
}

pub(in crate::voice) async fn send_key_package(socket: &mut Socket, key_package: &[u8]) {
    log::info!("DAVE tx op=26 key_package len={}", key_package.len());
    send_binary(socket, 26, key_package).await;
}

pub(in crate::voice) async fn send_ready(socket: &mut Socket, transition_id: u16, version: u16) {
    log::info!("DAVE tx op=23 ready transition={transition_id} version={version}");
    let _ = send_json(socket, ready_payload(transition_id)).await;
}

pub(super) async fn send_auth(
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

pub(super) fn identify_payload(config: &Config) -> Value {
    json!({"op": 0, "d": {
        "server_id": if config.guild_id == 0 { config.channel_id } else { config.guild_id }.to_string(),
        "channel_id": config.channel_id.to_string(),
        "user_id": config.user_id.to_string(),
        "session_id": config.session_id.expose(),
        "token": config.token.expose(),
        "video": false,
        "streams": [],
        "max_dave_protocol_version": davey::DAVE_PROTOCOL_VERSION,
    }})
}

pub(super) fn resume_payload(config: &Config, seq: Option<u64>) -> Value {
    json!({"op": 7, "d": {
        "server_id": if config.guild_id == 0 { config.channel_id } else { config.guild_id }.to_string(),
        "channel_id": config.channel_id.to_string(),
        "session_id": config.session_id.expose(),
        "token": config.token.expose(),
        "seq_ack": seq,
    }})
}

pub(super) fn heartbeat_payload(seq: Option<u64>) -> Value {
    json!({"op": 3, "d": {"t": unix_ms(), "seq_ack": seq.unwrap_or(0)}})
}

pub(super) fn speaking_payload(ssrc: u32, flags: u32) -> Value {
    json!({"op": 5, "d": {"speaking": flags, "delay": 0, "ssrc": ssrc}})
}

pub(super) fn select_protocol_payload(address: &str, port: u16, mode: Mode) -> Value {
    json!({"op": 1, "d": {
        "protocol": "udp",
        "data": {"address": address, "port": port, "mode": mode.name()},
        "rtc_connection_id": properties::uuid_v4(),
        "codecs": [{"name": "opus", "type": "audio", "priority": 1000, "payload_type": 120}],
    }})
}

pub(in crate::voice) fn parse_binary_seq(bytes: &[u8]) -> Option<u64> {
    (bytes.len() >= 2).then(|| u64::from(u16::from_be_bytes([bytes[0], bytes[1]])))
}
