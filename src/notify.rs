use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

mod sound;
#[cfg(windows)]
mod toast;

static ACTIVATION_SENDER: OnceLock<UnboundedSender<Activation>> = OnceLock::new();
static ACTIVATION_RECEIVER: Mutex<Option<UnboundedReceiver<Activation>>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    pub user_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
}

pub fn init() {
    #[cfg(windows)]
    toast::init();
}

pub fn show(title: &str, body: &str, icon: Option<&Path>, activation: Activation) {
    #[cfg(windows)]
    toast::show(title, body, icon, activation);
    #[cfg(not(windows))]
    let _ = (title, body, icon, activation);
}

pub fn toast_icon(cached: &Path) -> Option<PathBuf> {
    let icon = cached.with_extension("toast.png");
    if icon.is_file() {
        return Some(icon);
    }
    let bytes = std::fs::read(cached).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    image
        .save_with_format(&icon, image::ImageFormat::Png)
        .ok()?;
    Some(icon)
}

pub fn play_message_sound(volume: f32) {
    sound::play(volume);
}

pub fn activations() -> UnboundedReceiver<Activation> {
    activation_sender();
    let receiver = ACTIVATION_RECEIVER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    receiver.unwrap_or_else(|| {
        let (sender, receiver) = unbounded_channel();
        drop(sender);
        receiver
    })
}

pub(crate) fn activation_sender() -> &'static UnboundedSender<Activation> {
    ACTIVATION_SENDER.get_or_init(|| {
        let (sender, receiver) = unbounded_channel();
        *ACTIVATION_RECEIVER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(receiver);
        sender
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_icons_are_png_copies_of_the_cached_image() {
        let directory = std::env::temp_dir().join(format!("zcord-icon-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let cached = directory.join("avatar.webp");
        image::RgbaImage::new(4, 4)
            .save_with_format(&cached, image::ImageFormat::Bmp)
            .unwrap();
        let icon = toast_icon(&cached).unwrap();
        assert_eq!(image::open(&icon).unwrap().width(), 4);
        assert_eq!(toast_icon(&cached), Some(icon));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn activations_are_handed_out_once() {
        let activation = Activation {
            user_id: 5,
            channel_id: 7,
            message_id: 9,
        };
        activation_sender().send(activation).unwrap();
        let mut first = activations();
        assert_eq!(first.try_recv().unwrap(), activation);
        let mut second = activations();
        assert!(second.try_recv().is_err());
    }
}
