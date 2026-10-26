use super::*;

impl DiscordApp {
    pub(crate) fn set_video_hover(&mut self, key: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(key)
        } else if self.hovered_video == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_video != next {
            self.hovered_video = next;
            cx.notify();
        }
    }

    pub(super) fn stop_player(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        if let Some(fullscreen) = self.video_fullscreen.take()
            && fullscreen.entered_window_fullscreen
            && let Some(window) = window.as_deref_mut()
            && window.is_fullscreen()
        {
            window.toggle_fullscreen();
        }
        self.hovered_video = None;
        self.video_drag = None;
        self.video_dragged_at = None;
        self.video_volume_hover = [false; 2];
        if let Some(player) = self.player.take() {
            for image in player.take_frames() {
                cx.drop_image(image, window.as_deref_mut());
            }
        }
    }

    pub(crate) fn start_video(
        &mut self,
        key: u64,
        url: String,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_player(Some(window), cx);
        let Some(mut player) = Player::start(
            key,
            url.clone(),
            theme::image_px(width),
            theme::image_px(height),
        ) else {
            cx.open_url(&url);
            return;
        };
        if let Some(audio) = crate::settings::get(cx).video_audio {
            player.restore_audio(audio.volume, audio.muted);
        }
        player.attach(cx);
        self.player = Some(player);
        self.hovered_video = Some(key);
        cx.notify();
    }

    pub(crate) fn enter_video_fullscreen(
        &mut self,
        mut fullscreen: Fullscreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        fullscreen.entered_window_fullscreen = !window.is_fullscreen();
        if fullscreen.entered_window_fullscreen {
            window.toggle_fullscreen();
        }
        self.video_fullscreen = Some(fullscreen);
        cx.notify();
    }

    pub(crate) fn exit_video_fullscreen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(fullscreen) = self.video_fullscreen.take() else {
            return false;
        };
        if fullscreen.entered_window_fullscreen && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.video_volume_hover = [false; 2];
        cx.notify();
        true
    }

    pub(crate) fn video_click(&mut self, key: u64, cx: &mut Context<Self>) {
        if self
            .video_dragged_at
            .take()
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.toggle_video(key, cx);
    }

    pub(crate) fn toggle_video(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_ref().filter(|player| player.key == key) {
            player.toggle();
        }
        cx.notify();
    }

    pub(crate) fn toggle_video_mute(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_ref().filter(|player| player.key == key) {
            player.toggle_muted();
            remember_video_audio(player, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_video_volume_hover(
        &mut self,
        index: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if self.video_volume_hover[index] != hovered {
            self.video_volume_hover[index] = hovered;
            cx.notify();
        }
    }

    pub(crate) fn video_volume_open(&self) -> bool {
        self.video_volume_hover.contains(&true)
            || matches!(self.video_drag, Some((_, VideoDrag::Volume)))
    }

    pub(crate) fn begin_video_drag(
        &mut self,
        key: u64,
        drag: VideoDrag,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.video_drag = Some((key, drag));
        self.apply_video_drag(key, drag, position, cx);
    }

    pub(crate) fn drag_video(&mut self, key: u64, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some((drag_key, drag)) = self.video_drag
            && drag_key == key
        {
            self.video_dragged_at = Some(Instant::now());
            self.apply_video_drag(key, drag, position, cx);
        }
    }

    pub(crate) fn end_video_drag(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.video_drag.is_some_and(|(drag_key, _)| drag_key == key) {
            self.video_drag = None;
            cx.notify();
        }
    }

    fn apply_video_drag(
        &mut self,
        key: u64,
        drag: VideoDrag,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let bounds = match drag {
            VideoDrag::Seek => &self.video_seek_bounds,
            VideoDrag::Volume => &self.video_volume_bounds,
        };
        let Some(bounds) = bounds
            .borrow()
            .as_ref()
            .filter(|(bound_key, _)| *bound_key == key)
            .map(|(_, bounds)| *bounds)
        else {
            return;
        };
        let Some(player) = self.player.as_ref().filter(|player| player.key == key) else {
            return;
        };
        match drag {
            VideoDrag::Seek => {
                let width = bounds.size.width.as_f32();
                if width > 0. {
                    let fraction = ((position.x - bounds.origin.x).as_f32() / width).clamp(0., 1.);
                    player.seek(fraction as f64 * player.view().duration);
                }
            }
            VideoDrag::Volume => {
                let height = bounds.size.height.as_f32();
                if height > 0. {
                    let fraction =
                        1. - ((position.y - bounds.origin.y).as_f32() / height).clamp(0., 1.);
                    player.set_volume(fraction as f64);
                    remember_video_audio(player, cx);
                }
            }
        }
        cx.notify();
    }
}
