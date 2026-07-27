use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{Receiver, Sender, TryRecvError},
    time::Duration,
};

use gpui::{
    Bounds, Context, ExternalPaths, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Render, Window, canvas, div, prelude::*, px, relative,
    svg,
};
use pulse_engine::{
    PcmFormat, PlaybackCommand, PlaybackController, PlaybackEvent, PlaybackState, device,
};

use crate::theme;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const SUPPORTED_EXTENSIONS: &[&str] = &["flac", "m4a", "aif", "aiff", "wav"];

pub struct PlaybackRow {
    controller: Option<PlaybackController>,
    command_tx: Option<Sender<PlaybackCommand>>,
    event_rx: Option<Receiver<PlaybackEvent>>,
    playback_state: PlaybackState,
    source_path: Option<PathBuf>,
    title: String,
    secondary: String,
    format: Option<PcmFormat>,
    device_name: Option<String>,
    position_ms: u64,
    duration_ms: Option<u64>,
    error: Option<String>,
    track_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scrubbing: bool,
    scrub_fraction: Option<f32>,
}

impl PlaybackRow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EVENT_POLL_INTERVAL).await;
                if this.update(cx, |this, cx| this.drain_events(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self::initial()
    }

    fn initial() -> Self {
        Self {
            controller: None,
            command_tx: None,
            event_rx: None,
            playback_state: PlaybackState::Idle,
            source_path: None,
            title: "No track loaded".to_string(),
            secondary: "Drop FLAC, ALAC, AIFF, or WAV".to_string(),
            format: None,
            device_name: None,
            position_ms: 0,
            duration_ms: None,
            error: None,
            track_bounds: Rc::new(Cell::new(None)),
            scrubbing: false,
            scrub_fraction: None,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn has_track(&self) -> bool {
        self.source_path.is_some()
    }

    pub(crate) fn handle_drop(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        if paths.paths().len() != 1 {
            self.error = Some("Drop one audio file at a time.".to_string());
            cx.notify();
            return;
        }

        let path = &paths.paths()[0];
        if !path.is_file() {
            self.error = Some("Drop a file, not a folder.".to_string());
            cx.notify();
            return;
        }
        if !is_supported_audio(path) {
            self.error = Some(
                "Unsupported file. Pulse accepts FLAC, ALAC (.m4a), AIFF, and WAV.".to_string(),
            );
            cx.notify();
            return;
        }

        self.error = None;
        self.play_file(path.clone(), cx);
    }

    fn play_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.controller.is_none() {
            let output_device = match device::default_output_device() {
                Ok(device) => device,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            };
            let controller = PlaybackController::spawn(output_device.id);
            self.event_rx = Some(controller.subscribe());
            self.command_tx = Some(controller.command_sender());
            self.device_name = Some(output_device.name);
            self.controller = Some(controller);
        }

        self.send_command(PlaybackCommand::PlayFile { path }, cx);
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self.toggle_command() {
            self.send_command(command, cx);
        }
    }

    fn toggle_command(&self) -> Option<PlaybackCommand> {
        match self.playback_state {
            PlaybackState::Playing => Some(PlaybackCommand::Pause),
            PlaybackState::Paused => Some(PlaybackCommand::Resume),
            PlaybackState::Ended | PlaybackState::Error => self
                .source_path
                .clone()
                .map(|path| PlaybackCommand::PlayFile { path }),
            PlaybackState::Idle | PlaybackState::Loading | PlaybackState::Stopping => None,
        }
    }

    fn send_command(&mut self, command: PlaybackCommand, cx: &mut Context<Self>) {
        let Some(command_tx) = &self.command_tx else {
            return;
        };
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
            cx.notify();
        }
    }

    fn drain_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        loop {
            let event = match self.event_rx.as_ref().map(Receiver::try_recv) {
                Some(Ok(event)) => event,
                Some(Err(TryRecvError::Empty)) | None => break,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.event_rx = None;
                    self.error = Some("Playback engine disconnected.".to_string());
                    changed = true;
                    break;
                }
            };
            self.handle_event(event);
            changed = true;
        }

        if changed {
            cx.notify();
        }
    }

    fn handle_event(&mut self, event: PlaybackEvent) {
        match event {
            PlaybackEvent::StateChanged(state) => {
                self.playback_state = state;
            }
            PlaybackEvent::NowPlaying { source, format } => {
                self.title = track_title(&source.path);
                self.secondary = track_secondary(&source.path);
                self.source_path = Some(source.path);
                self.duration_ms = source.duration_ms;
                self.format = Some(format);
                self.position_ms = 0;
                self.error = None;
            }
            PlaybackEvent::Position {
                position_ms,
                duration_ms,
            } => {
                self.position_ms = position_ms;
                self.duration_ms = duration_ms;
            }
            PlaybackEvent::Ended => {
                self.playback_state = PlaybackState::Ended;
                if let Some(duration_ms) = self.duration_ms {
                    self.position_ms = duration_ms;
                }
            }
            PlaybackEvent::CommandRejected { command, state } => {
                self.error = Some(format!(
                    "{command} is unavailable while playback is {}.",
                    playback_state_label(state)
                ));
            }
            PlaybackEvent::Error { message } => {
                self.error = Some(message);
            }
        }
    }

    fn begin_scrub(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Paused
        ) || self.duration_ms.is_none_or(|duration_ms| duration_ms == 0)
        {
            return;
        }
        let Some(bounds) = self.track_bounds.get() else {
            return;
        };
        self.scrubbing = true;
        self.scrub_fraction = Some(fraction_at_x(bounds, event.position.x));
        cx.notify();
    }

    pub(crate) fn update_scrub(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.scrubbing {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.scrubbing = false;
            self.scrub_fraction = None;
            cx.notify();
            return;
        }
        let Some(bounds) = self.track_bounds.get() else {
            return;
        };
        self.scrub_fraction = Some(fraction_at_x(bounds, event.position.x));
        cx.notify();
    }

    pub(crate) fn finish_scrub(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if !self.scrubbing {
            return;
        }

        self.scrubbing = false;
        let fraction = self
            .track_bounds
            .get()
            .map(|bounds| fraction_at_x(bounds, event.position.x))
            .or(self.scrub_fraction);
        self.scrub_fraction = None;

        if let (Some(fraction), Some(duration_ms)) = (fraction, self.duration_ms) {
            let position_ms = scrub_position_ms(fraction, duration_ms);
            self.position_ms = position_ms;
            self.send_command(PlaybackCommand::Seek { position_ms }, cx);
        } else {
            cx.notify();
        }
    }

    fn displayed_fraction(&self) -> f32 {
        self.scrub_fraction.unwrap_or_else(|| {
            self.duration_ms
                .filter(|duration| *duration > 0)
                .map(|duration| (self.position_ms as f64 / duration as f64) as f32)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0)
        })
    }

    fn displayed_position_ms(&self) -> u64 {
        match (self.scrub_fraction, self.duration_ms) {
            (Some(fraction), Some(duration_ms)) => scrub_position_ms(fraction, duration_ms),
            _ => self.position_ms,
        }
    }

    fn render_now_playing(&self) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w(px(330.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(52.))
                    .flex_none()
                    .overflow_hidden()
                    .rounded(px(theme::RADIUS_SM))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::bg_elevated())
                    .child(
                        svg()
                            .path("icons/list-music.svg")
                            .size(px(22.))
                            .text_color(theme::text_muted()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(15.))
                            .text_color(theme::text_primary())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(self.secondary.clone()),
                    ),
            )
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.displayed_fraction();
        let track_bounds = Rc::clone(&self.track_bounds);
        let play_icon = if self.playback_state == PlaybackState::Playing {
            "icons/pause.svg"
        } else {
            "icons/play.svg"
        };

        div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .child(transport_icon("icons/shuffle.svg"))
                    .child(transport_icon("icons/skip-back.svg"))
                    .child(
                        div()
                            .id("playback-toggle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(theme::RADIUS_MD))
                            .bg(theme::accent())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx)))
                            .child(
                                svg()
                                    .path(play_icon)
                                    .size(px(16.))
                                    .text_color(theme::bg_inset()),
                            ),
                    )
                    .child(transport_icon("icons/skip-forward.svg"))
                    .child(transport_icon("icons/repeat-2.svg")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .child(time_label(format_time(self.displayed_position_ms())))
                    .child(
                        div()
                            .id("progress-target")
                            .flex_1()
                            .flex()
                            .items_center()
                            .h(px(16.))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event, _, cx| this.begin_scrub(event, cx)),
                            )
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .h(px(4.))
                                    .rounded(px(2.))
                                    .bg(theme::bg_inset())
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .bottom_0()
                                            .w(relative(progress))
                                            .rounded(px(2.))
                                            .bg(theme::accent()),
                                    )
                                    .child(
                                        canvas(
                                            move |bounds, _, _| track_bounds.set(Some(bounds)),
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .top_0()
                                        .right_0()
                                        .bottom_0()
                                        .left_0(),
                                    ),
                            ),
                    )
                    .child(time_label(
                        self.duration_ms
                            .map(format_time)
                            .unwrap_or_else(|| "--:--".to_string()),
                    )),
            )
    }

    fn render_output(&self) -> impl IntoElement {
        let (quality, quality_color) = self
            .format
            .map(|format| {
                (
                    format_quality(self.source_path.as_deref(), format),
                    theme::quality(),
                )
            })
            .unwrap_or_else(|| ("—".to_string(), theme::text_muted()));
        let device = match (self.format, &self.device_name) {
            (Some(format), Some(device)) => {
                format!("{} · {device}", format_sample_rate(format.sample_rate))
            }
            (_, Some(device)) => device.clone(),
            (_, None) => "No output selected".to_string(),
        };

        div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(14.))
            .w(px(300.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .w(px(132.))
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(12.))
                            .text_color(quality_color)
                            .whitespace_nowrap()
                            .child(quality),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(device),
                    ),
            )
            .child(
                svg()
                    .path("icons/speaker.svg")
                    .size(px(17.))
                    .text_color(theme::text_secondary()),
            )
            .child(
                div()
                    .relative()
                    .w(px(38.))
                    .h(px(34.))
                    .child(
                        svg()
                            .path("icons/list-music.svg")
                            .absolute()
                            .left_0()
                            .top(px(8.))
                            .size(px(17.))
                            .text_color(theme::text_secondary()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .right(px(-2.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded(px(10.))
                            .bg(theme::bg_surface())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(16.))
                                    .rounded(px(8.))
                                    .bg(theme::accent())
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(10.))
                                    .text_color(theme::bg_inset())
                                    .child(if self.source_path.is_some() { "1" } else { "0" }),
                            ),
                    ),
            )
    }
}

impl Render for PlaybackRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(22.))
            .w_full()
            .h(px(92.))
            .flex_none()
            .px(px(20.))
            .py(px(12.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .child(self.render_now_playing())
            .child(self.render_transport(cx))
            .child(self.render_output())
    }
}

fn transport_icon(path: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(17.))
        .child(
            svg()
                .path(path)
                .size(px(17.))
                .text_color(theme::text_secondary()),
        )
}

fn time_label(value: String) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .text_color(theme::text_muted())
        .child(value)
}

fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn track_title(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn track_secondary(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn format_quality(path: Option<&Path>, format: PcmFormat) -> String {
    let container = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "PCM".to_string());
    format!("{container} · {}-bit", format.bits_per_sample)
}

fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1_000) {
        format!("{} kHz", sample_rate / 1_000)
    } else {
        format!("{:.1} kHz", sample_rate as f64 / 1_000.0)
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes >= 60 {
        format!("{}:{:02}:{seconds:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn playback_state_label(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Idle => "idle",
        PlaybackState::Loading => "loading",
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopping => "stopping",
        PlaybackState::Ended => "ended",
        PlaybackState::Error => "in error",
    }
}

fn scrub_position_ms(fraction: f32, duration_ms: u64) -> u64 {
    (duration_ms as f64 * f64::from(fraction)).round() as u64
}

fn fraction_at_x(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    if bounds.size.width <= px(0.) {
        return 0.0;
    }
    ((x - bounds.origin.x) / bounds.size.width).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use gpui::{bounds, point, px, size};

    use super::*;

    #[test]
    fn accepts_only_supported_audio_extensions() {
        for path in [
            "track.flac",
            "track.FLAC",
            "track.m4a",
            "track.aif",
            "track.aiff",
            "track.wav",
        ] {
            assert!(is_supported_audio(Path::new(path)), "{path}");
        }
        for path in ["track.mp3", "track.aac", "track"] {
            assert!(!is_supported_audio(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn derives_display_text_from_the_path() {
        let path = Path::new("/Music/Album/Track 01.flac");
        assert_eq!(track_title(path), "Track 01");
        assert_eq!(track_secondary(path), "Album");
    }

    #[test]
    fn formats_time_for_tracks_and_long_programs() {
        assert_eq!(format_time(0), "00:00");
        assert_eq!(format_time(185_999), "03:05");
        assert_eq!(format_time(3_661_000), "1:01:01");
    }

    #[test]
    fn clamps_progress_positions_to_the_track() {
        let track_bounds = bounds(point(px(100.), px(20.)), size(px(200.), px(4.)));
        assert_eq!(fraction_at_x(track_bounds, px(50.)), 0.0);
        assert_eq!(fraction_at_x(track_bounds, px(200.)), 0.5);
        assert_eq!(fraction_at_x(track_bounds, px(350.)), 1.0);
        assert_eq!(
            fraction_at_x(
                bounds(point(px(100.), px(20.)), size(px(0.), px(4.))),
                px(100.)
            ),
            0.0
        );
    }

    #[test]
    fn maps_scrub_fraction_to_position() {
        assert_eq!(scrub_position_ms(0.0, 268_000), 0);
        assert_eq!(scrub_position_ms(0.5, 268_000), 134_000);
        assert_eq!(scrub_position_ms(1.0, 268_000), 268_000);

        let bounds = bounds(point(px(100.), px(20.)), size(px(200.), px(4.)));
        assert_eq!(
            scrub_position_ms(fraction_at_x(bounds, px(250.)), 268_000),
            201_000
        );
    }

    #[test]
    fn formats_reported_pcm_without_inventing_codec_details() {
        let format = PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 24,
            channels: 2,
        };
        assert_eq!(
            format_quality(Some(Path::new("track.flac")), format),
            "FLAC · 24-bit"
        );
        assert_eq!(
            format_quality(Some(Path::new("track.m4a")), format),
            "M4A · 24-bit"
        );
        assert_eq!(format_sample_rate(format.sample_rate), "44.1 kHz");
    }

    #[test]
    fn derives_row_state_from_playback_events() {
        let mut row = PlaybackRow::initial();
        row.error = Some("old error".to_string());

        row.handle_event(PlaybackEvent::NowPlaying {
            source: pulse_engine::PlayableSource {
                path: PathBuf::from("/Music/Blonde/Nights.flac"),
                duration_ms: Some(268_000),
            },
            format: PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 16,
                channels: 2,
            },
        });
        row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));
        row.handle_event(PlaybackEvent::Position {
            position_ms: 134_000,
            duration_ms: Some(268_000),
        });

        assert_eq!(row.title, "Nights");
        assert_eq!(row.secondary, "Blonde");
        assert_eq!(row.playback_state, PlaybackState::Playing);
        assert_eq!(row.displayed_fraction(), 0.5);
        assert!(row.error.is_none());

        row.handle_event(PlaybackEvent::Ended);
        assert_eq!(row.playback_state, PlaybackState::Ended);
        assert_eq!(row.position_ms, 268_000);
    }

    #[test]
    fn maps_real_transport_states_to_toggle_commands() {
        let mut row = PlaybackRow::initial();
        assert_eq!(row.toggle_command(), None);

        row.playback_state = PlaybackState::Playing;
        assert_eq!(row.toggle_command(), Some(PlaybackCommand::Pause));

        row.playback_state = PlaybackState::Paused;
        assert_eq!(row.toggle_command(), Some(PlaybackCommand::Resume));

        row.source_path = Some(PathBuf::from("/Music/track.flac"));
        row.playback_state = PlaybackState::Ended;
        assert_eq!(
            row.toggle_command(),
            Some(PlaybackCommand::PlayFile {
                path: PathBuf::from("/Music/track.flac")
            })
        );
    }
}
