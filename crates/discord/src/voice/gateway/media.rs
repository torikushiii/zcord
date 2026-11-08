use super::signaling::select_protocol_payload;
use super::*;

impl Connection {
    pub(super) async fn start_udp(
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

    pub(super) async fn audio_event(
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

    pub(super) async fn send_keepalive(&mut self) {
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

    pub(super) async fn tick(&mut self, events: &UnboundedSender<Event>) {
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

    pub(super) async fn on_datagram(&mut self, packet: &[u8], events: &UnboundedSender<Event>) {
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
