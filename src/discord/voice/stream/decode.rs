use super::{DecodeError, Decoder, FrameSlot, StreamFrame, nv12_to_bgra};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
};

pub(super) struct DecodeControl {
    pub frames: FrameSlot,
    pub lost: Arc<Mutex<bool>>,
    pub fatal: Arc<Mutex<Option<String>>>,
    pub live: Arc<AtomicBool>,
}

pub(super) fn spawn(
    decoder: impl FnOnce() -> Result<Box<dyn Decoder>, String> + Send + 'static,
    control: DecodeControl,
    on_live: impl Fn(u32, u32) + Send + 'static,
) -> Result<SyncSender<Vec<u8>>, String> {
    let (send, receive) = sync_channel(4);
    thread::Builder::new()
        .name("stream-decode".into())
        .spawn(move || match decoder() {
            Ok(decoder) => run(receive, decoder, control, on_live),
            Err(reason) => {
                *control
                    .fatal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
            }
        })
        .map_err(|_| "Could not start the stream decoder thread".to_owned())?;
    Ok(send)
}

fn run(
    receive: Receiver<Vec<u8>>,
    mut decoder: Box<dyn Decoder>,
    control: DecodeControl,
    on_live: impl Fn(u32, u32),
) {
    while let Ok(access_unit) = receive.recv() {
        match decoder.decode(&access_unit) {
            Ok(pictures) => {
                for picture in pictures {
                    if picture.width > 1920 || picture.height > 1080 {
                        *control
                            .fatal
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some("Stream resolution exceeds 1920x1080".into());
                        return;
                    }
                    let bgra = nv12_to_bgra(
                        &picture.y,
                        &picture.uv,
                        picture.stride as usize,
                        picture.width as usize,
                        picture.height as usize,
                    );
                    *control
                        .frames
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(StreamFrame {
                        width: picture.width,
                        height: picture.height,
                        bgra,
                    });
                    if !control.live.swap(true, Ordering::AcqRel) {
                        on_live(picture.width, picture.height);
                    }
                }
            }
            Err(DecodeError::TooLarge) => {
                *control
                    .fatal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some("Stream resolution exceeds 1920x1080".into());
                return;
            }
            Err(DecodeError::Failed) => {
                *control
                    .lost
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Failing;

    impl Decoder for Failing {
        fn decode(&mut self, _: &[u8]) -> Result<Vec<super::super::Picture>, DecodeError> {
            Err(DecodeError::Failed)
        }
    }

    #[test]
    fn a_decode_failure_marks_the_stream_for_a_keyframe() {
        let control = DecodeControl {
            frames: Arc::new(Mutex::new(None)),
            lost: Arc::new(Mutex::new(false)),
            fatal: Arc::new(Mutex::new(None)),
            live: Arc::new(AtomicBool::new(false)),
        };
        let lost = control.lost.clone();
        let send = spawn(|| Ok(Box::new(Failing)), control, |_, _| {}).unwrap();
        send.send(vec![0, 0, 0, 1, 0x65]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !*lost.lock().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(*lost.lock().unwrap());
    }
}
