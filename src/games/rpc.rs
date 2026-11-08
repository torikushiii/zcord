use serde_json::{Map, Value, json};
use std::{collections::HashSet, io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, watch},
    task::JoinSet,
};

const MAX_FRAME: usize = 16 * 1024;
const MAX_CLIENTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TEXT: usize = 128;
const MAX_ASSET: usize = 1024;
const MAX_BUTTONS: usize = 2;
const MAX_TIMESTAMP_MS: u64 = 4_102_444_800_000;
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

struct Frame {
    opcode: u32,
    payload: Vec<u8>,
}

fn frame_header(opcode: u32, length: usize) -> io::Result<[u8; 8]> {
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RPC frame exceeds 16 KiB",
        ));
    }
    let mut header = [0; 8];
    header[..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..].copy_from_slice(&(length as u32).to_le_bytes());
    Ok(header)
}

fn parse_header(header: [u8; 8]) -> io::Result<(u32, usize)> {
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RPC frame exceeds 16 KiB",
        ));
    }
    Ok((opcode, length))
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Frame> {
    let mut header = [0; 8];
    stream.read_exact(&mut header).await?;
    let (opcode, length) = parse_header(header)?;
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    Ok(Frame { opcode, payload })
}

async fn read_frame_after_start<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Frame> {
    let mut header = [0; 8];
    stream.read_exact(&mut header[..1]).await?;
    let read = async {
        stream.read_exact(&mut header[1..]).await?;
        let (opcode, length) = parse_header(header)?;
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await?;
        Ok(Frame { opcode, payload })
    };
    tokio::time::timeout(TIMEOUT, read)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "RPC read timed out"))?
}

async fn write_frame<S: AsyncWrite + Unpin>(
    stream: &mut S,
    opcode: u32,
    payload: &[u8],
) -> io::Result<()> {
    let header = frame_header(opcode, payload.len())?;
    let write = async {
        stream.write_all(&header).await?;
        stream.write_all(payload).await?;
        stream.flush().await
    };
    tokio::time::timeout(TIMEOUT, write)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "RPC write timed out"))?
}

async fn send_json<S: AsyncWrite + Unpin>(stream: &mut S, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_frame(stream, OP_FRAME, &payload).await
}

fn ready() -> Value {
    json!({
        "cmd": "DISPATCH",
        "evt": "READY",
        "nonce": null,
        "data": {
            "v": 1,
            "config": {
                "cdn_host": "cdn.discordapp.com",
                "api_endpoint": "//discord.com/api",
                "environment": "production"
            },
            "user": {
                "id": "0",
                "username": "zcord",
                "discriminator": "0",
                "avatar": null
            }
        }
    })
}

fn parse_handshake(frame: &Frame) -> Option<u64> {
    if frame.opcode != OP_HANDSHAKE {
        return None;
    }
    let handshake: Value = serde_json::from_slice(&frame.payload).ok()?;
    if handshake.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    match handshake.get("client_id")? {
        Value::String(client) => client.parse().ok(),
        Value::Number(client) => client.as_u64(),
        _ => None,
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn truncate_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn activity_type(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .filter(|kind| matches!(kind, 0 | 2 | 3 | 5))
        .unwrap_or(0)
}

fn activity_timestamp(value: u64) -> Option<u64> {
    if value == 0 {
        return None;
    }
    let value = if value < 1_000_000_000_000 {
        value.checked_mul(1000)?
    } else {
        value
    };
    (value <= MAX_TIMESTAMP_MS).then_some(value)
}

fn build_activity(input: &Value, application_id: u64) -> Option<Value> {
    let input = input.as_object()?;
    let mut activity = Map::new();
    activity.insert(
        "application_id".into(),
        Value::String(application_id.to_string()),
    );
    activity.insert("type".into(), Value::from(activity_type(input.get("type"))));
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .map(|name| truncate_chars(name, MAX_TEXT))
        .unwrap_or_default();
    activity.insert("name".into(), Value::String(name));
    for key in ["details", "state"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            activity.insert(key.into(), Value::String(truncate_chars(value, MAX_TEXT)));
        }
    }
    if let Some(timestamps) = input.get("timestamps").and_then(Value::as_object) {
        let mut converted = Map::new();
        for key in ["start", "end"] {
            if let Some(value) = timestamps
                .get(key)
                .and_then(Value::as_u64)
                .and_then(activity_timestamp)
            {
                converted.insert(key.into(), Value::from(value));
            }
        }
        if let (Some(start), Some(end)) = (
            converted.get("start").and_then(Value::as_u64),
            converted.get("end").and_then(Value::as_u64),
        ) && end <= start
        {
            converted.remove("end");
        }
        if !converted.is_empty() {
            activity.insert("timestamps".into(), Value::Object(converted));
        }
    }
    if let Some(assets) = input.get("assets").and_then(Value::as_object) {
        let mut converted = Map::new();
        for key in ["large_image", "large_text", "small_image", "small_text"] {
            if let Some(value) = assets.get(key).and_then(Value::as_str) {
                let value = if key.ends_with("_image") {
                    truncate_bytes(value, MAX_ASSET)
                } else {
                    truncate_chars(value, MAX_TEXT)
                };
                converted.insert(key.into(), Value::String(value));
            }
        }
        if !converted.is_empty() {
            activity.insert("assets".into(), Value::Object(converted));
        }
    }
    if let Some(party) = input.get("party").filter(|value| value.is_object()) {
        activity.insert("party".into(), party.clone());
    }
    if let Some(buttons) = input.get("buttons").and_then(Value::as_array) {
        let mut labels = Vec::new();
        let mut urls = Vec::new();
        for button in buttons {
            if labels.len() == MAX_BUTTONS {
                break;
            }
            let label = button
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = button
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if label.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
                continue;
            }
            labels.push(Value::String(truncate_chars(label, MAX_TEXT)));
            urls.push(Value::String(url.to_owned()));
        }
        if !labels.is_empty() {
            activity.insert("buttons".into(), Value::Array(labels));
            let mut metadata = Map::new();
            metadata.insert("button_urls".into(), Value::Array(urls));
            activity.insert("metadata".into(), Value::Object(metadata));
        }
    }
    Some(Value::Object(activity))
}

enum Update {
    Keep,
    Set(Value),
    Clear,
}

fn reply(command: &str, nonce: &Value, data: Value) -> Value {
    json!({"cmd": command, "nonce": nonce, "evt": Value::Null, "data": data})
}

fn error_reply(command: &str, nonce: &Value) -> Value {
    json!({
        "cmd": command,
        "nonce": nonce,
        "evt": "ERROR",
        "data": {"code": 4000, "message": "Unsupported"}
    })
}

fn handle_command(packet: &Value, application_id: u64) -> (Update, Value) {
    let command = packet
        .get("cmd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let nonce = packet.get("nonce").cloned().unwrap_or(Value::Null);
    if command != "SET_ACTIVITY" {
        return (Update::Keep, error_reply(command, &nonce));
    }
    match packet.get("args").and_then(|args| args.get("activity")) {
        None | Some(Value::Null) => (Update::Clear, reply(command, &nonce, Value::Null)),
        Some(activity) => match build_activity(activity, application_id) {
            Some(activity) => (
                Update::Set(activity.clone()),
                reply(command, &nonce, activity),
            ),
            None => (Update::Keep, error_reply(command, &nonce)),
        },
    }
}

trait Listener: Send {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn accept(&mut self) -> impl std::future::Future<Output = io::Result<Self::Stream>> + Send;
}

async fn serve<L: Listener>(mut listener: L, output: watch::Sender<Option<Value>>) {
    let (updates, mut incoming) = mpsc::channel(MAX_CLIENTS);
    let mut clients = JoinSet::new();
    let mut connected = HashSet::new();
    let mut active: Vec<(u64, Value)> = Vec::new();
    let mut next = 0_u64;
    loop {
        tokio::select! {
            accepted = listener.accept(), if clients.len() < MAX_CLIENTS => {
                match accepted {
                    Ok(stream) => {
                        next += 1;
                        let id = next;
                        connected.insert(id);
                        let updates = updates.clone();
                        clients.spawn(async move {
                            connection(stream, id, updates).await;
                            id
                        });
                    }
                    Err(error) => {
                        log::warn!("Discord IPC accept failed: {error}");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
            Some((id, update)) = incoming.recv() => {
                if !connected.contains(&id) {
                    continue;
                }
                active.retain(|(other, _)| *other != id);
                if let Some(activity) = update {
                    active.push((id, activity));
                }
            }
            Some(done) = clients.join_next(), if !clients.is_empty() => {
                match done {
                    Ok(id) => {
                        connected.remove(&id);
                        active.retain(|(other, _)| *other != id);
                    }
                    Err(error) => log::warn!("Discord IPC client failed: {error}"),
                }
            }
        }
        output.send_if_modified(|shown| {
            let next = active.last().map(|(_, activity)| activity);
            if shown.as_ref() == next {
                false
            } else {
                *shown = next.cloned();
                true
            }
        });
    }
}

async fn connection<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    connection_id: u64,
    updates: mpsc::Sender<(u64, Option<Value>)>,
) {
    let handshake = match tokio::time::timeout(TIMEOUT, read_frame(&mut stream)).await {
        Ok(Ok(frame)) => frame,
        _ => return,
    };
    let Some(application_id) = parse_handshake(&handshake) else {
        return;
    };
    if send_json(&mut stream, &ready()).await.is_err() {
        return;
    }
    loop {
        let frame = match read_frame_after_start(&mut stream).await {
            Ok(frame) => frame,
            Err(_) => return,
        };
        match frame.opcode {
            OP_CLOSE => return,
            OP_PING => {
                if write_frame(&mut stream, OP_PONG, &frame.payload)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            OP_PONG => {}
            OP_FRAME => {
                let Ok(packet) = serde_json::from_slice::<Value>(&frame.payload) else {
                    return;
                };
                let (update, response) = handle_command(&packet, application_id);
                if send_json(&mut stream, &response).await.is_err() {
                    return;
                }
                match update {
                    Update::Keep => {}
                    Update::Set(activity) => {
                        if updates.send((connection_id, Some(activity))).await.is_err() {
                            return;
                        }
                    }
                    Update::Clear => {
                        if updates.send((connection_id, None)).await.is_err() {
                            return;
                        }
                    }
                }
            }
            _ => return,
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::Listener;
    use std::{
        fs, io,
        os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        time::Duration,
    };
    use tokio::net::{UnixListener, UnixStream};

    pub(super) struct Socket {
        listener: UnixListener,
        path: PathBuf,
        device: u64,
        inode: u64,
    }

    impl Listener for Socket {
        type Stream = UnixStream;

        async fn accept(&mut self) -> io::Result<UnixStream> {
            self.listener.accept().await.map(|(stream, _)| stream)
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            if let Ok(metadata) = fs::symlink_metadata(&self.path)
                && metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    pub(super) async fn bind() -> io::Result<Socket> {
        let directory = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .into_iter()
            .filter_map(std::env::var_os)
            .find(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let mut last = None;
        for slot in 0..10 {
            match bind_slot(&directory, slot).await {
                Ok(socket) => return Ok(socket),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("no Discord IPC socket available")))
    }

    async fn bind_slot(directory: &Path, slot: u32) -> io::Result<Socket> {
        let path = directory.join(format!("discord-ipc-{slot}"));
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "slot is not a socket",
                ));
            }
            match tokio::time::timeout(Duration::from_millis(100), UnixStream::connect(&path)).await
            {
                Ok(Ok(stream)) => {
                    drop(stream);
                    return Err(io::Error::new(io::ErrorKind::AddrInUse, "socket is live"));
                }
                Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    if !same_file(&metadata, &path) {
                        return Err(io::Error::new(io::ErrorKind::AddrInUse, "socket changed"));
                    }
                    fs::remove_file(&path)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "socket state is unknown",
                    ));
                }
            }
        }
        let listener = UnixListener::bind(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let socket = Socket {
            listener,
            path: path.clone(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(socket)
    }

    fn same_file(metadata: &fs::Metadata, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|now| {
            now.file_type().is_socket()
                && now.dev() == metadata.dev()
                && now.ino() == metadata.ino()
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            sync::atomic::{AtomicU64, Ordering},
            time::Duration,
        };
        use tokio::sync::watch;

        static NEXT: AtomicU64 = AtomicU64::new(0);

        async fn read(stream: &mut UnixStream) -> super::super::Frame {
            tokio::time::timeout(Duration::from_secs(5), super::super::read_frame(stream))
                .await
                .expect("frame arrives")
                .expect("frame reads")
        }

        async fn write(stream: &mut UnixStream, opcode: u32, payload: &[u8]) {
            super::super::write_frame(stream, opcode, payload)
                .await
                .expect("frame writes");
        }

        #[tokio::test]
        async fn handshakes_publish_activities_and_clean_up() {
            let directory = std::env::temp_dir().join(format!(
                "zcord-rpc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).expect("temporary directory");
            let path = directory.join("discord-ipc-0");
            let listener = bind_slot(&directory, 0).await.expect("socket binds");
            assert_eq!(
                fs::metadata(&path)
                    .expect("socket exists")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            let (output, mut published) = watch::channel(None);
            let server = tokio::spawn(super::super::serve(listener, output));
            let mut client = UnixStream::connect(&path).await.expect("client connects");

            write(
                &mut client,
                super::super::OP_HANDSHAKE,
                br#"{"v":1,"client_id":"42"}"#,
            )
            .await;
            let ready: serde_json::Value =
                serde_json::from_slice(&read(&mut client).await.payload).expect("ready JSON");
            assert_eq!(ready["evt"], "READY");
            assert_eq!(ready["data"]["user"]["username"], "zcord");

            let activity = serde_json::json!({
                "cmd": "SET_ACTIVITY",
                "nonce": "1",
                "args": {"activity": {"name": "Game", "details": "Test"}}
            });
            write(
                &mut client,
                super::super::OP_FRAME,
                &serde_json::to_vec(&activity).expect("activity JSON"),
            )
            .await;
            let reply: serde_json::Value =
                serde_json::from_slice(&read(&mut client).await.payload).expect("reply JSON");
            assert_eq!(reply["cmd"], "SET_ACTIVITY");
            assert_eq!(reply["nonce"], "1");
            assert_eq!(reply["data"]["application_id"], "42");

            let shown = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(activity) = published.borrow().clone() {
                        break activity;
                    }
                    published.changed().await.expect("activity channel");
                }
            })
            .await
            .expect("activity is published");
            assert_eq!(shown["name"], "Game");

            write(&mut client, super::super::OP_PING, b"ping").await;
            let pong = read(&mut client).await;
            assert_eq!(pong.opcode, super::super::OP_PONG);
            assert_eq!(pong.payload, b"ping");

            write(
                &mut client,
                super::super::OP_FRAME,
                br#"{"cmd":"SUBSCRIBE","nonce":"2"}"#,
            )
            .await;
            let error: serde_json::Value =
                serde_json::from_slice(&read(&mut client).await.payload).expect("error JSON");
            assert_eq!(error["evt"], "ERROR");
            assert!(published.borrow().is_some());

            drop(client);
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if published.borrow().is_none() {
                        break;
                    }
                    published.changed().await.expect("activity channel");
                }
            })
            .await
            .expect("disconnect clears the activity");

            server.abort();
            let _ = server.await;
            assert!(!path.exists());
            let _ = fs::remove_dir_all(&directory);
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::Listener;
    use std::{
        ffi::c_void,
        io, mem,
        pin::Pin,
        ptr,
        task::{Context, Poll},
    };
    use tokio::{
        io::{AsyncRead, AsyncWrite, ReadBuf},
        net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    };

    const PIPE_PREFIX: &str = r"\\.\pipe\discord-ipc-";
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER: u32 = 1;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }

    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    #[repr(C)]
    struct TokenUser {
        user: SidAndAttributes,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: u32,
            information: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, string: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor: *const u16,
            revision: u32,
            security_descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    struct Dacl {
        descriptor: usize,
    }

    impl Dacl {
        fn attributes(&self) -> SecurityAttributes {
            SecurityAttributes {
                length: std::mem::size_of::<SecurityAttributes>() as u32,
                descriptor: self.descriptor as *mut c_void,
                inherit: 0,
            }
        }

        fn current_user() -> io::Result<Self> {
            unsafe {
                let mut token = ptr::null_mut();
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut storage = [0_usize; 64];
                let mut needed = 0;
                let found = GetTokenInformation(
                    token,
                    TOKEN_USER,
                    storage.as_mut_ptr().cast(),
                    std::mem::size_of_val(&storage) as u32,
                    &mut needed,
                );
                let _ = CloseHandle(token);
                if found == 0 {
                    return Err(io::Error::last_os_error());
                }
                let user = &*storage.as_ptr().cast::<TokenUser>();
                let mut string = ptr::null_mut();
                if ConvertSidToStringSidW(user.user.sid, &mut string) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut length = 0;
                while *string.add(length) != 0 {
                    length += 1;
                }
                let sid = String::from_utf16_lossy(std::slice::from_raw_parts(string, length));
                let _ = LocalFree(string.cast());
                let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
                    .encode_utf16()
                    .chain([0])
                    .collect();
                let mut descriptor = ptr::null_mut();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self {
                    descriptor: descriptor as usize,
                })
            }
        }
    }

    impl Drop for Dacl {
        fn drop(&mut self) {
            if self.descriptor != 0 {
                unsafe {
                    LocalFree(self.descriptor as *mut c_void);
                }
            }
        }
    }

    pub(super) struct PipeStream(NamedPipeServer);

    impl AsyncRead for PipeStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_read(context, buffer)
        }
    }

    impl AsyncWrite for PipeStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(context, buffer)
        }

        fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(context)
        }
    }

    impl Drop for PipeStream {
        fn drop(&mut self) {
            let _ = self.0.disconnect();
        }
    }

    pub(super) struct PipeSocket {
        server: NamedPipeServer,
        name: String,
        dacl: Dacl,
    }

    impl PipeSocket {
        pub(super) fn bind() -> io::Result<Self> {
            let dacl = Dacl::current_user()?;
            let mut last = None;
            for slot in 0..10 {
                let name = format!("{PIPE_PREFIX}{slot}");
                match create(&name, &dacl, true) {
                    Ok(server) => {
                        return Ok(Self { server, name, dacl });
                    }
                    Err(error) => last = Some(error),
                }
            }
            Err(last.unwrap_or_else(|| io::Error::other("no Discord IPC pipe available")))
        }
    }

    impl Listener for PipeSocket {
        type Stream = PipeStream;

        async fn accept(&mut self) -> io::Result<PipeStream> {
            self.server.connect().await?;
            let next = create(&self.name, &self.dacl, false)?;
            Ok(PipeStream(mem::replace(&mut self.server, next)))
        }
    }

    fn create(name: &str, dacl: &Dacl, first: bool) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(super::MAX_CLIENTS + 1)
            .in_buffer_size(16 * 1024)
            .out_buffer_size(16 * 1024);
        let mut attributes = dacl.attributes();
        unsafe {
            options.create_with_security_attributes_raw(
                name,
                &mut attributes as *mut SecurityAttributes as *mut c_void,
            )
        }
    }
}

pub(crate) async fn run(output: watch::Sender<Option<Value>>) {
    #[cfg(unix)]
    match unix::bind().await {
        Ok(listener) => serve(listener, output).await,
        Err(error) => log::warn!("Discord IPC is unavailable: {error}"),
    }
    #[cfg(windows)]
    match windows::PipeSocket::bind() {
        Ok(listener) => serve(listener, output).await,
        Err(error) => log::warn!("Discord IPC is unavailable: {error}"),
    }
}

#[cfg(test)]
mod tests;
