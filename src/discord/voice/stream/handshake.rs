use super::super::{
    crypto::Mode,
    gateway::{send_json, ws_request},
    udp,
};
use crate::discord::{Secret, properties};
use futures::StreamExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::watch,
    time::timeout,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const DISCOVERY_ATTEMPTS: usize = 3;
pub(crate) const HEARTBEAT_DEFAULT: Duration = Duration::from_secs(60);
pub(crate) const AUDIO_PAYLOAD: u8 = 120;
pub(crate) const VIDEO_PAYLOAD: u8 = 101;
pub(crate) const RTX_PAYLOAD: u8 = 102;

pub(crate) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) struct HandshakeConfig {
    pub user_id: u64,
    pub session_id: Secret,
    pub token: Secret,
    pub endpoint: String,
    pub rtc_server_id: u64,
    pub streams: Value,
    pub encode: bool,
    pub decode: bool,
}

pub(crate) struct Handshake {
    pub socket: Socket,
    pub udp: UdpSocket,
    pub peer: SocketAddr,
    pub ssrc: u32,
    pub video_ssrc: u32,
    pub rtx_ssrc: u32,
    pub heartbeat: Duration,
}

pub(crate) async fn handshake(
    config: &HandshakeConfig,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Handshake, String> {
    let url = super::super::task::ws_url(&config.endpoint)
        .ok_or_else(|| "Invalid Discord stream endpoint".to_owned())?;
    let request = ws_request(&url).ok_or_else(|| "Invalid Discord stream endpoint".to_owned())?;
    let connection = timeout(CONNECT_TIMEOUT, connect_async(request));
    let connection = tokio::select! {
        _ = cancel.changed() => return Err("Stream connection stopped".into()),
        result = connection => result,
    };
    let (mut socket, _) = connection
        .map_err(|_| "Discord stream connection timed out".to_owned())?
        .map_err(|_| "Discord stream TLS connection failed".to_owned())?;
    let mut heartbeat = HEARTBEAT_DEFAULT;
    loop {
        let message = tokio::select! {
            _ = cancel.changed() => return Err("Stream connection stopped".into()),
            message = socket.next() => message,
        };
        let Some(Ok(message)) = message else {
            return Err("Discord stream socket failed".into());
        };
        let Message::Text(text) = message else {
            continue;
        };
        let event: Value = serde_json::from_str(&text)
            .map_err(|_| "Discord sent invalid stream signaling".to_owned())?;
        match event.get("op").and_then(Value::as_u64) {
            Some(8) => {
                heartbeat = event
                    .get("d")
                    .and_then(|data| data.get("heartbeat_interval"))
                    .and_then(Value::as_u64)
                    .filter(|ms| *ms >= 100 && *ms <= 120_000)
                    .map_or(HEARTBEAT_DEFAULT, Duration::from_millis);
                let identify = json!({"op": 0, "d": {
                    "server_id": config.rtc_server_id.to_string(),
                    "user_id": config.user_id.to_string(),
                    "session_id": config.session_id.expose(),
                    "token": config.token.expose(),
                    "video": true,
                    "streams": config.streams.clone(),
                    "max_dave_protocol_version": davey::DAVE_PROTOCOL_VERSION,
                }});
                send_json(&mut socket, identify)
                    .await
                    .map_err(|_| "Discord stream identify failed".to_owned())?;
            }
            Some(2) => {
                let data = event.get("d").cloned().unwrap_or(Value::Null);
                let ssrc = data
                    .get("ssrc")
                    .and_then(Value::as_u64)
                    .filter(|ssrc| *ssrc != 0)
                    .and_then(|ssrc| u32::try_from(ssrc).ok())
                    .ok_or_else(|| "Discord sent an invalid stream SSRC".to_owned())?;
                let ip = data
                    .get("ip")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Discord sent an invalid stream address".to_owned())?;
                let port = data
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|port| u16::try_from(port).ok())
                    .filter(|port| *port != 0)
                    .ok_or_else(|| "Discord sent an invalid stream port".to_owned())?;
                let (video_ssrc, rtx_ssrc) = stream_ssrcs(&data).unwrap_or((0, 0));
                let modes: Vec<String> = data
                    .get("modes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|mode| mode.as_str().map(str::to_owned))
                    .collect();
                if !modes.iter().any(|mode| mode == Mode::Aes256Gcm.name()) {
                    return Err("Discord offered no supported stream encryption".into());
                }
                let udp = discover(ssrc, ip, port).await?;
                let peer = udp.0;
                let udp = udp.1;
                let select = json!({"op": 1, "d": {
                    "protocol": "udp",
                    "data": {
                        "address": peer.0,
                        "port": peer.1,
                        "mode": Mode::Aes256Gcm.name(),
                    },
                    "rtc_connection_id": properties::uuid_v4(),
                    "codecs": [
                        {"name": "opus", "type": "audio", "priority": 1000, "payload_type": 120},
                        {"name": "H264", "type": "video", "priority": 1000,
                         "payload_type": VIDEO_PAYLOAD, "rtx_payload_type": RTX_PAYLOAD,
                         "encode": config.encode, "decode": config.decode},
                    ],
                }});
                send_json(&mut socket, select)
                    .await
                    .map_err(|_| "Discord stream UDP selection failed".to_owned())?;
                return Ok(Handshake {
                    socket,
                    udp,
                    peer: peer.2,
                    ssrc,
                    video_ssrc,
                    rtx_ssrc,
                    heartbeat,
                });
            }
            _ => {}
        }
    }
}

fn stream_ssrcs(data: &Value) -> Option<(u32, u32)> {
    let ssrc = |value: Option<&Value>| {
        value
            .and_then(Value::as_u64)
            .and_then(|ssrc| u32::try_from(ssrc).ok())
            .filter(|ssrc| *ssrc != 0)
    };
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

async fn discover(
    ssrc: u32,
    ip: &str,
    port: u16,
) -> Result<((String, u16, SocketAddr), UdpSocket), String> {
    let udp = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|_| "Could not open the stream UDP socket".to_owned())?;
    let mut buffer = [0u8; 128];
    let mut discovered = None;
    for _ in 0..DISCOVERY_ATTEMPTS {
        if udp
            .send_to(&udp::discovery_request(ssrc), (ip, port))
            .await
            .is_err()
        {
            continue;
        }
        let Ok(Ok((size, _))) = timeout(DISCOVERY_TIMEOUT, udp.recv_from(&mut buffer)).await else {
            continue;
        };
        if let Some(found) = udp::parse_discovery_response(&buffer[..size]) {
            discovered = Some(found);
            break;
        }
    }
    let (address, external_port) =
        discovered.ok_or_else(|| "Could not discover the stream UDP endpoint".to_owned())?;
    let peer = tokio::net::lookup_host((ip, port))
        .await
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or_else(|| "Discord sent an invalid stream address".to_owned())?;
    Ok(((address, external_port, peer), udp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::SinkExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

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
            assert_eq!(identify["d"]["server_id"], "30");
            assert_eq!(identify["d"]["user_id"], "1");
            assert_eq!(identify["d"]["session_id"], "call-session");
            assert_eq!(identify["d"]["token"], "stream-token");
            assert_eq!(identify["d"]["video"], true);
            assert_eq!(identify["d"]["streams"], json!([]));
            assert_eq!(
                identify["d"]["max_dave_protocol_version"],
                davey::DAVE_PROTOCOL_VERSION
            );
            socket
                .send(Message::Text(
                    json!({"op": 2, "d": {
                        "ssrc": 4242,
                        "ip": "127.0.0.1",
                        "port": udp_address.port(),
                        "modes": ["aead_aes256_gcm_rtpsize"],
                        "streams": [{"ssrc": 700, "rtx_ssrc": 701}]
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
            let codecs = select["d"]["codecs"].as_array().unwrap();
            assert_eq!(codecs[1]["name"], "H264");
            assert_eq!(codecs[1]["payload_type"], 101);
            assert_eq!(codecs[1]["rtx_payload_type"], 102);
            assert_eq!(codecs[1]["encode"], false);
            assert_eq!(codecs[1]["decode"], true);
        });

        let config = HandshakeConfig {
            user_id: 1,
            session_id: Secret("call-session".into()),
            token: Secret("stream-token".into()),
            endpoint: format!("ws://{ws_address}"),
            rtc_server_id: 30,
            streams: json!([]),
            encode: false,
            decode: true,
        };
        let (_cancel, mut cancel) = watch::channel(false);
        let handshake = timeout(Duration::from_secs(10), handshake(&config, &mut cancel))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(handshake.ssrc, 4242);
        assert_eq!(handshake.video_ssrc, 700);
        assert_eq!(handshake.rtx_ssrc, 701);
        assert_eq!(handshake.heartbeat, Duration::from_secs(60));
        server.await.unwrap();
    }
}
