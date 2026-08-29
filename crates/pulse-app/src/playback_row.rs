use std::{cell::Cell, ops::Deref, rc::Rc};

use gpui::{
    AnyElement, Bounds, Context, Entity, FocusHandle, FontWeight, IntoElement, ListSizingBehavior,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render,
    Subscription, Window, canvas, div, img, prelude::*, px, relative, svg, uniform_list,
};
use pulse_engine::{PlaybackState, device};

use crate::{
    app_store::{AppStore, StoreRevisions, global_app_store},
    playback::*,
    queue::{RepeatMode, TrackRef},
    theme, ui,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum PlaybackSurface {
    Transport,
    SettingsOutputPicker,
}

pub(crate) struct PlaybackRow {
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    snapshot: PlaybackSnapshot,
    surface: PlaybackSurface,
    volume_popover_open: bool,
    volume_toggle_press_closed_popover: bool,
    output_popover_open: bool,
    output_toggle_press_closed_popover: bool,
    queue_popover_open: bool,
    queue_toggle_press_closed_popover: bool,
    hovered_upcoming: Option<usize>,
    volume_popover_focus: Option<FocusHandle>,
    queue_popover_focus: Option<FocusHandle>,
    track_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scrubbing: bool,
    scrub_fraction: Option<f32>,
    volume_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    volume_dragging: bool,
    _store_subscription: Subscription,
}

impl Deref for PlaybackRow {
    type Target = PlaybackSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}

impl PlaybackRow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let app_store = global_app_store(cx);
        let store_revisions = app_store.read(cx).revisions;
        let snapshot = app_store.read(cx).playback_snapshot();
        Self {
            app_store: app_store.clone(),
            store_revisions,
            snapshot,
            surface: PlaybackSurface::Transport,
            volume_popover_open: false,
            volume_toggle_press_closed_popover: false,
            output_popover_open: false,
            output_toggle_press_closed_popover: false,
            queue_popover_open: false,
            queue_toggle_press_closed_popover: false,
            hovered_upcoming: None,
            volume_popover_focus: Some(cx.focus_handle()),
            queue_popover_focus: Some(cx.focus_handle()),
            track_bounds: Rc::new(Cell::new(None)),
            scrubbing: false,
            scrub_fraction: None,
            volume_bounds: Rc::new(Cell::new(None)),
            volume_dragging: false,
            _store_subscription: cx.observe(&app_store, |this, _, cx| {
                this.handle_store_update(cx);
            }),
        }
    }

    fn handle_store_update(&mut self, cx: &mut Context<Self>) {
        let revisions = self.app_store.read(cx).revisions;
        let reactions = revisions.reactions_since(self.store_revisions);
        self.store_revisions = revisions;
        if reactions.playback || reactions.queue || reactions.devices || reactions.settings {
            self.snapshot = self.app_store.read(cx).playback_snapshot();
            cx.notify();
        }
    }

    fn send(&self, command: PlaybackAction, cx: &mut Context<Self>) -> bool {
        self.app_store
            .update(cx, |store, store_cx| store.send_command(command, store_cx))
    }

    pub(crate) fn output_popover_open(&self) -> bool {
        self.output_popover_open
    }

    pub(crate) fn enter_settings(&mut self, cx: &mut Context<Self>) {
        self.surface = PlaybackSurface::SettingsOutputPicker;
        self.volume_popover_open = false;
        self.output_popover_open = false;
        self.queue_popover_open = false;
        cx.notify();
    }

    pub(crate) fn leave_settings(&mut self, cx: &mut Context<Self>) {
        self.surface = PlaybackSurface::Transport;
        self.volume_popover_open = false;
        self.output_popover_open = false;
        cx.notify();
    }

    pub(crate) fn close_output_popover(&mut self, cx: &mut Context<Self>) {
        if self.output_popover_open {
            self.output_popover_open = false;
            cx.notify();
        }
    }

    pub(crate) fn toggle_settings_output_popover(&mut self, cx: &mut Context<Self>) {
        self.toggle_output_popover(cx);
    }

    fn toggle_volume_mute(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::ToggleVolumeMute, cx);
    }

    fn set_volume_level(&mut self, level: f32, cx: &mut Context<Self>) {
        self.send(PlaybackAction::SetVolumeLevel(level), cx);
    }

    fn persist_volume(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::PersistVolume, cx);
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::TogglePlayback, cx);
    }

    fn next_track(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::NextTrack, cx);
    }

    fn previous_track(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::PreviousTrack, cx);
    }

    fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::ToggleShuffle, cx);
    }

    fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::CycleRepeat, cx);
    }

    fn jump_to_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.send(PlaybackAction::JumpToQueueEntry(index), cx);
    }

    fn remove_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.hovered_upcoming = None;
        self.send(PlaybackAction::RemoveQueueEntry(index), cx);
    }

    fn clear_upcoming_queue(&mut self, cx: &mut Context<Self>) {
        self.hovered_upcoming = None;
        self.send(PlaybackAction::ClearUpcomingQueue, cx);
    }

    fn retry_playback(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::RetryPlayback, cx);
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::DismissNotice, cx);
    }

    fn displayed_device_message(&self) -> Option<DeviceMessage> {
        self.snapshot.device_message.clone()
    }

    fn exclusive_mode_is_automatic(&self) -> bool {
        self.snapshot.exclusive_mode_automatic
    }

    fn toggle_exclusive_mode(&mut self, cx: &mut Context<Self>) {
        let Some(active_device) = self.active_device.clone() else {
            return;
        };
        self.send(
            PlaybackAction::ToggleDeviceExclusiveMode {
                device_uid: active_device.uid,
                default: self.default_exclusive_mode,
            },
            cx,
        );
    }

    fn reset_exclusive_mode_to_auto(&mut self, cx: &mut Context<Self>) {
        let Some(active_device) = self.active_device.clone() else {
            return;
        };
        self.send(
            PlaybackAction::ResetDeviceExclusiveMode {
                device_uid: active_device.uid,
                default: self.default_exclusive_mode,
            },
            cx,
        );
    }

    fn select_output_device(&mut self, output_device: device::Device, cx: &mut Context<Self>) {
        self.send(PlaybackAction::SelectOutputDevice(output_device), cx);
    }

    fn toggle_output_popover(&mut self, cx: &mut Context<Self>) {
        self.output_popover_open = !self.output_popover_open;
        if self.output_popover_open {
            self.send(PlaybackAction::RefreshOutputDevices, cx);
        }
        cx.notify();
    }

    fn toggle_queue_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue_popover_open = !self.queue_popover_open;
        if self.queue_popover_open {
            self.hovered_upcoming = None;
            if let Some(focus) = &self.queue_popover_focus {
                window.focus(focus, cx);
            }
        }
        cx.notify();
    }

    fn toggle_volume_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.volume_popover_open = !self.volume_popover_open;
        if self.volume_popover_open
            && let Some(focus) = &self.volume_popover_focus
        {
            window.focus(focus, cx);
        }
        cx.notify();
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

    fn begin_volume_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.volume_bounds.get() else {
            return;
        };
        self.volume_dragging = true;
        self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
    }

    pub(crate) fn update_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            let was_dragging = self.scrubbing || self.volume_dragging;
            self.scrubbing = false;
            self.scrub_fraction = None;
            if self.volume_dragging {
                self.volume_dragging = false;
                self.persist_volume(cx);
            }
            if was_dragging {
                cx.notify();
            }
            return;
        }
        if self.scrubbing
            && let Some(bounds) = self.track_bounds.get()
        {
            self.scrub_fraction = Some(fraction_at_x(bounds, event.position.x));
            cx.notify();
        }
        if self.volume_dragging
            && let Some(bounds) = self.volume_bounds.get()
        {
            self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
        }
    }

    pub(crate) fn finish_drag(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.scrubbing {
            self.scrubbing = false;
            let fraction = self
                .track_bounds
                .get()
                .map(|bounds| fraction_at_x(bounds, event.position.x))
                .or(self.scrub_fraction);
            self.scrub_fraction = None;
            if let (Some(fraction), Some(duration_ms)) = (fraction, self.duration_ms) {
                self.send(
                    PlaybackAction::Seek(scrub_position_ms(fraction, duration_ms)),
                    cx,
                );
            }
            cx.notify();
        }
        if self.volume_dragging {
            self.volume_dragging = false;
            if let Some(bounds) = self.volume_bounds.get() {
                self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
            }
            self.persist_volume(cx);
            cx.notify();
        }
    }

    fn displayed_fraction(&self) -> f32 {
        self.scrub_fraction
            .unwrap_or_else(|| self.snapshot.displayed_fraction())
    }

    fn displayed_position_ms(&self) -> u64 {
        match (self.scrub_fraction, self.duration_ms) {
            (Some(fraction), Some(duration_ms)) => scrub_position_ms(fraction, duration_ms),
            _ => self.position_ms,
        }
    }

    fn render_now_playing(&self) -> impl IntoElement {
        let cover = match &self.cover_art_path {
            Some(path) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => svg()
                .path("icons/list-music.svg")
                .size(px(22.))
                .text_color(theme::text_muted())
                .into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .w(px(317.))
            .flex_none()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(60.))
                            .flex_none()
                            .overflow_hidden()
                            .rounded(px(theme::RADIUS_SM))
                            .border_1()
                            .border_color(theme::border_strong())
                            .bg(theme::bg_elevated())
                            .child(cover),
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
                    ),
            )
            .child(div().w(px(1.)).h(px(44.)).flex_none().bg(theme::border()))
    }

    fn render_progress_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.displayed_fraction();
        let scrubbing = self.scrubbing;
        let track_bounds = Rc::clone(&self.track_bounds);

        div()
            .relative()
            .w_full()
            .h(px(3.))
            .flex_none()
            .bg(theme::bg_elevated())
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(relative(progress))
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
            )
            .child(
                div()
                    .id("progress-target")
                    .group("progress-strip")
                    .absolute()
                    .top_0()
                    .right_0()
                    .left_0()
                    .h(px(12.))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.begin_scrub(event, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h(px(10.))
                            .w(relative(progress))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-3.5))
                                    .right(px(-5.))
                                    .size(px(10.))
                                    .rounded(px(5.))
                                    .bg(theme::accent())
                                    .opacity(if scrubbing { 1.0 } else { 0.0 })
                                    .when(!scrubbing, |thumb| {
                                        thumb.group_hover("progress-strip", |style| {
                                            style.opacity(1.0)
                                        })
                                    }),
                            ),
                    ),
            )
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let previous_enabled = self.source_path.is_some()
            && !matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Stopping
            );
        let next_enabled = self.queue.can_advance()
            && !matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Stopping
            );
        let play_icon = if self.playback_state == PlaybackState::Playing {
            "icons/pause.svg"
        } else {
            "icons/play.svg"
        };
        let shuffle_enabled = self.queue.shuffle_enabled();
        let repeat_mode = self.queue.repeat_mode();
        let repeat_icon = if repeat_mode == RepeatMode::One {
            "icons/repeat-1.svg"
        } else {
            "icons/repeat-2.svg"
        };

        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_center()
            .gap(px(28.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .w(px(44.))
                    .flex_none()
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px(10.))
                            .text_color(theme::text_secondary())
                            .child(format_time(self.displayed_position_ms())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .id("playback-shuffle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(19.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx)))
                            .child(svg().path("icons/shuffle.svg").size(px(19.)).text_color(
                                if shuffle_enabled {
                                    theme::accent()
                                } else {
                                    theme::text_secondary()
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("playback-previous")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(19.))
                            .opacity(if previous_enabled { 1.0 } else { 0.35 })
                            .when(previous_enabled, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.previous_track(cx);
                                    }))
                            })
                            .child(
                                svg()
                                    .path("icons/skip-back.svg")
                                    .size(px(19.))
                                    .text_color(theme::text_secondary()),
                            ),
                    )
                    .child(
                        div()
                            .id("playback-toggle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(34.))
                            .rounded(px(theme::RADIUS_MD))
                            .bg(theme::accent())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx)))
                            .child(
                                svg()
                                    .path(play_icon)
                                    .size(px(18.))
                                    .text_color(theme::bg_inset()),
                            ),
                    )
                    .child(
                        div()
                            .id("playback-next")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(19.))
                            .opacity(if next_enabled { 1.0 } else { 0.35 })
                            .when(next_enabled, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.next_track(cx)))
                            })
                            .child(
                                svg()
                                    .path("icons/skip-forward.svg")
                                    .size(px(19.))
                                    .text_color(theme::text_secondary()),
                            ),
                    )
                    .child(
                        div()
                            .id("playback-repeat")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(19.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx)))
                            .child(svg().path(repeat_icon).size(px(19.)).text_color(
                                if repeat_mode == RepeatMode::Off {
                                    theme::text_secondary()
                                } else {
                                    theme::accent()
                                },
                            )),
                    ),
            )
            .child(
                div().flex().items_center().w(px(44.)).flex_none().child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(10.))
                        .text_color(theme::text_muted())
                        .child(
                            self.duration_ms
                                .map(format_time)
                                .unwrap_or_else(|| "--:--".to_string()),
                        ),
                ),
            )
    }

    fn render_output(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let remaining = self.queue.remaining_count();
        let volume_icon = volume_icon_state(self.volume_level, self.volume_muted);
        let (quality, quality_color) = self
            .format
            .map(|format| {
                (
                    format_quality(self.source_path.as_deref(), format),
                    theme::quality(),
                )
            })
            .unwrap_or_else(|| ("—".to_string(), theme::text_muted()));
        let device = match (self.format, &self.active_device) {
            (Some(format), Some(device)) => format_output_device(
                format.sample_rate,
                &device.name,
                self.playback_exclusive_mode,
            ),
            (_, Some(device)) => device.name.clone(),
            (_, None) => "No output selected".to_string(),
        };
        let mut output_details = div()
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
            );
        if let Some(message) = self.displayed_device_message() {
            output_details = output_details.child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(10.))
                    .text_color(if message.is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(message.text),
            );
        }

        let mut volume = div()
            .id("volume-toggle")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(17.))
            .flex_none()
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.volume_toggle_press_closed_popover = this.volume_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.volume_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_volume_popover(window, cx);
            }))
            .child(svg().path(volume_icon.path()).size(px(17.)).text_color(
                if volume_icon == VolumeIconState::Muted {
                    theme::text_muted()
                } else if self.volume_popover_open {
                    theme::accent()
                } else {
                    theme::text_secondary()
                },
            ));
        if self.volume_popover_open {
            volume = volume.child(self.render_volume_popover(cx));
        }

        let mut speaker = div()
            .id("output-device-toggle")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(17.))
            .flex_none()
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.output_toggle_press_closed_popover = this.output_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                if std::mem::take(&mut this.output_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_output_popover(cx);
            }))
            .child(
                svg().path("icons/speaker.svg").size(px(17.)).text_color(
                    if self
                        .displayed_device_message()
                        .is_some_and(|message| message.is_error)
                    {
                        theme::danger()
                    } else if self.output_popover_open {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    },
                ),
            );
        if self.output_popover_open {
            speaker = speaker.child(self.render_output_popover(cx));
        }

        let mut queue_button = div()
            .id("queue-toggle")
            .relative()
            .w(px(38.))
            .h(px(34.))
            .cursor_pointer()
            // Same press-closed guard as the output picker and artist filter:
            // clicking the open trigger closes rather than closes-then-reopens.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.queue_toggle_press_closed_popover = this.queue_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.queue_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_queue_popover(window, cx);
            }))
            .child(
                svg()
                    .path("icons/list-music.svg")
                    .absolute()
                    .left_0()
                    .top(px(8.))
                    .size(px(17.))
                    .text_color(if self.queue_popover_open {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    }),
            );
        if self.queue_popover_open {
            queue_button = queue_button.child(self.render_queue_popover(cx));
        }

        div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(14.))
            .w(px(300.))
            .child(output_details)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(volume)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(speaker)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(queue_button.when(remaining > 0, |button| {
                button.child(
                    div()
                        .absolute()
                        .top(px(-2.))
                        .right(px(-2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(20.))
                        .min_w(px(20.))
                        .px(px(2.))
                        .rounded(px(10.))
                        .bg(theme::bg_surface())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(16.))
                                .min_w(px(16.))
                                .px(px(3.))
                                .rounded(px(8.))
                                .bg(theme::accent())
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(10.))
                                .text_color(theme::bg_inset())
                                .child(remaining.to_string()),
                        ),
                )
            }))
    }

    fn render_volume_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let volume_bounds = Rc::clone(&self.volume_bounds);
        let volume_fill = displayed_volume_level(self.volume_level, self.volume_muted);
        let volume_dragging = self.volume_dragging;
        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("volume-popover", px(56.))
            .left(px(-19.5))
            .bottom(px(54.))
            .items_center()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.update_drag(event, cx);
            }))
            .on_mouse_up(cx.listener(|this, event: &MouseUpEvent, _, cx| {
                this.finish_drag(event, cx);
            }))
            .on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.volume_popover_open = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(11.))
                    .text_center()
                    .text_color(theme::text_secondary())
                    .whitespace_nowrap()
                    .child(format_volume_percent(self.volume_level)),
            )
            .child(
                div()
                    .id("volume-slider-target")
                    .group("volume-slider")
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .h(px(120.))
                    .flex_none()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.begin_volume_drag(event, cx)),
                    )
                    .child(
                        div()
                            .relative()
                            .w(px(4.))
                            .h_full()
                            .rounded(px(2.))
                            .bg(theme::bg_inset())
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(relative(volume_fill))
                                    .rounded(px(2.))
                                    .bg(theme::accent())
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(-6.))
                                            .left(px(-4.))
                                            .size(px(12.))
                                            .rounded(px(6.))
                                            .bg(theme::accent())
                                            .opacity(if volume_dragging { 1.0 } else { 0.0 })
                                            .when(!volume_dragging, |thumb| {
                                                thumb.group_hover("volume-slider", |style| {
                                                    style.opacity(1.0)
                                                })
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| volume_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0(),
                    ),
            )
            .child(
                div()
                    .id("volume-mute-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(17.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_volume_mute(cx)))
                    .child(
                        svg()
                            .path("icons/volume-x.svg")
                            .size(px(17.))
                            .text_color(theme::text_secondary()),
                    ),
            );
        if let Some(focus) = &self.volume_popover_focus {
            popover = popover.focus_handle(focus.clone());
        }
        popover.into_any_element()
    }

    fn render_output_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_name = self
            .active_device
            .as_ref()
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "No active output".to_string());
        let capability = self
            .device_capabilities
            .map(format_device_capabilities)
            .unwrap_or_else(|| "Capabilities unavailable".to_string());
        let mut direct_devices = div().flex().flex_col().gap(px(2.)).w_full();
        for (index, output_device) in self.devices.iter().cloned().enumerate() {
            direct_devices =
                direct_devices.child(self.render_output_device_row(output_device, index, cx));
        }
        if self.devices.is_empty() {
            direct_devices = direct_devices.child(
                div()
                    .px(px(10.))
                    .py(px(9.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("No direct output devices found"),
            );
        }

        let entity = cx.entity();
        let mut popover =
            ui::PopoverMenu::new("output-device-popover", px(360.)).on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.output_popover_open = false;
                    cx.notify();
                });
            });
        popover = if self.surface == PlaybackSurface::SettingsOutputPicker {
            popover.right(px(0.)).top(px(30.))
        } else {
            popover.right(px(-52.)).bottom(px(54.))
        };
        popover = popover
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(17.))
                            .text_color(theme::text_primary())
                            .child("Choose audio output"),
                    )
                    .child(
                        svg()
                            .path("icons/settings.svg")
                            .size(px(16.))
                            .text_color(theme::text_secondary()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .p(px(12.))
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::bg_inset())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .w_full()
                            .child(
                                svg()
                                    .path("icons/speaker.svg")
                                    .size(px(22.))
                                    .flex_none()
                                    .text_color(theme::accent()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_DISPLAY)
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(px(17.))
                                            .text_color(theme::text_primary())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(active_name),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_SANS)
                                            .text_size(px(12.))
                                            .text_color(theme::text_secondary())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(if self.playback_exclusive_mode {
                                                "CoreAudio · Exclusive during playback"
                                            } else {
                                                "CoreAudio · Shared playback"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_MONO)
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(px(11.))
                                            .text_color(if self.device_capabilities.is_some() {
                                                theme::quality()
                                            } else {
                                                theme::warning()
                                            })
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(capability),
                                    ),
                            )
                            .when(self.active_device.is_some(), |device| {
                                device.child(
                                    svg()
                                        .path("icons/check.svg")
                                        .size(px(18.))
                                        .flex_none()
                                        .text_color(theme::accent()),
                                )
                            }),
                    )
                    .when(self.active_device.is_some(), |card| {
                        card.child(div().w_full().h(px(1.)).my(px(10.)).bg(theme::border()))
                            .child(ui::exclusive_mode_control(
                                self.exclusive_mode_is_automatic(),
                                ui::exclusive_mode_reset_link("exclusive-mode-reset-auto")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.reset_exclusive_mode_to_auto(cx);
                                    }))
                                    .into_any_element(),
                                ui::Toggle::new("exclusive-mode-toggle", self.exclusive_mode)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_exclusive_mode(cx);
                                    }))
                                    .into_any_element(),
                            ))
                    }),
            );

        if let Some(message) = self.displayed_device_message() {
            popover = popover.child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(11.))
                    .text_color(if message.is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .child(message.text),
            );
        }

        popover
            .child(section_label("DIRECT DEVICES"))
            .child(direct_devices)
            .child(section_label("NETWORK DEVICES"))
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("No network devices found"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .pt(px(12.))
                    .pr(px(2.))
                    .pb(px(2.))
                    .pl(px(2.))
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(14.))
                            .text_color(theme::text_secondary())
                            .child("Can't find your device?"),
                    )
                    .child(
                        svg()
                            .path("icons/log-in.svg")
                            .size(px(16.))
                            .text_color(theme::text_muted()),
                    ),
            )
    }

    fn render_output_device_row(
        &self,
        output_device: device::Device,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .active_device
            .as_ref()
            .is_some_and(|active| active.uid == output_device.uid);
        let selected_device = output_device.clone();

        div()
            .id(("output-device", index))
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(selected, |row| {
                row.border_1()
                    .border_color(theme::accent())
                    .bg(theme::accent_soft())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_output_device(selected_device.clone(), cx);
            }))
            .child(
                svg()
                    .path("icons/speaker.svg")
                    .size(px(18.))
                    .flex_none()
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .text_color(if selected {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(output_device.name),
            )
            .when(selected, |row| {
                row.child(
                    svg()
                        .path("icons/check.svg")
                        .size(px(16.))
                        .flex_none()
                        .text_color(theme::accent()),
                )
            })
    }

    fn render_queue_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let upcoming_count = self.queue.remaining_count();

        let mut header = div().flex().items_center().gap(px(10.)).w_full().child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(17.))
                .text_color(theme::text_primary())
                .child("Queue"),
        );
        if upcoming_count > 0 {
            header = header
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(10.))
                        .text_color(theme::text_muted())
                        .child(format_queue_meta(
                            upcoming_count,
                            self.queue.upcoming_duration_ms(),
                        )),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("queue-clear")
                        .flex()
                        .items_center()
                        .h(px(23.))
                        .px(px(8.))
                        .flex_none()
                        .rounded(px(theme::RADIUS_SM))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg_muted())
                        .cursor_pointer()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .child("Clear")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_upcoming_queue(cx))),
                );
        }

        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("queue-popover", px(376.))
            .right(px(0.))
            .bottom(px(71.))
            .max_height(px(541.))
            .on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.queue_popover_open = false;
                    cx.notify();
                });
            })
            .child(header);

        if let Some((title, secondary)) = self.now_playing_lines() {
            popover = popover.child(section_label("NOW PLAYING")).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .h(px(58.))
                    .flex_none()
                    .px(px(10.))
                    .relative()
                    .overflow_hidden()
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_inset())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
                    .child(
                        svg()
                            .path("icons/audio-lines.svg")
                            .size(px(16.))
                            .flex_none()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(14.))
                                    .text_color(theme::text_primary())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(11.))
                                    .text_color(theme::text_secondary())
                                    .child(secondary),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::quality())
                            .child(format!(
                                "{} / {}",
                                format_queue_time(self.displayed_position_ms()),
                                self.duration_ms
                                    .map(format_queue_time)
                                    .unwrap_or_else(|| "--:--".to_string())
                            )),
                    ),
            );
        }

        popover = popover.child(section_label("UP NEXT"));
        if upcoming_count == 0 {
            popover = popover.child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("Nothing up next"),
            );
        } else {
            // Virtualized: only the visible rows are built, so a full-library
            // queue stays cheap through the 100 ms position ticks.
            popover = popover.child(
                uniform_list(
                    "queue-upcoming-list",
                    upcoming_count,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let visible = this
                            .queue
                            .upcoming()
                            .skip(range.start)
                            .take(range.len())
                            .map(|(position, track)| (position, track.clone()))
                            .collect::<Vec<_>>();
                        visible
                            .into_iter()
                            .map(|(position, track)| this.render_upcoming_row(position, track, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .min_h_0(),
            );
        }

        if let Some(focus) = &self.queue_popover_focus {
            popover = popover.focus_handle(focus.clone());
        }
        popover.into_any_element()
    }

    fn render_upcoming_row(
        &self,
        position: usize,
        track: TrackRef,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = position - 1;
        let hovered = self.hovered_upcoming == Some(index);
        let duration = track
            .duration_ms
            .map(format_queue_time)
            .unwrap_or_else(|| "--:--".to_string());

        // Uniform 52px stride for the virtualized list: the 50px design row
        // plus its 2px gap as bottom padding.
        let row = div()
            .id(("queue-upcoming", index))
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .h(px(50.))
            .flex_none()
            .px(px(10.))
            .rounded(px(theme::RADIUS_MD))
            .when(hovered, |row| row.bg(theme::bg_muted()))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, &hovered, _, cx| {
                if hovered {
                    this.hovered_upcoming = Some(index);
                } else if this.hovered_upcoming == Some(index) {
                    this.hovered_upcoming = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.jump_to_queue_entry(index, cx)))
            .child(
                div()
                    .min_w(px(18.))
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(position.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(track.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_secondary())
                            .child(format!("{} · {}", track.artist, track.album)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(duration),
            )
            .when(hovered, |row| {
                row.child(
                    div()
                        .id(("queue-remove", index))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(14.))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_queue_entry(index, cx);
                        }))
                        .child(
                            svg()
                                .path("icons/x.svg")
                                .size(px(14.))
                                .text_color(theme::text_muted()),
                        ),
                )
            });

        div()
            .w_full()
            .h(px(52.))
            .pb(px(2.))
            .child(row)
            .into_any_element()
    }
}

impl Render for PlaybackRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.surface == PlaybackSurface::SettingsOutputPicker {
            return div()
                .relative()
                .size(px(0.))
                .when(self.output_popover_open, |anchor| {
                    anchor.child(self.render_output_popover(cx))
                });
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .flex_none()
            .when_some(self.notice.clone(), |column, notice| {
                column.child(self.render_notice(notice, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .h(px(92.))
                    .flex_none()
                    .bg(theme::bg_surface())
                    .child(self.render_progress_strip(cx))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap(px(22.))
                            .w_full()
                            .px(px(20.))
                            .py(px(12.))
                            .child(self.render_now_playing())
                            .child(self.render_transport(cx))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .gap(px(16.))
                                    .w(px(317.))
                                    .flex_none()
                                    .child(
                                        div().w(px(1.)).h(px(44.)).flex_none().bg(theme::border()),
                                    )
                                    .child(self.render_output(cx)),
                            ),
                    ),
            )
    }
}

impl PlaybackRow {
    fn render_notice(&self, notice: PlaybackNotice, cx: &mut Context<Self>) -> impl IntoElement {
        let (text, color, recovery) = match notice {
            PlaybackNotice::Skip { text } => (text, theme::warning(), false),
            PlaybackNotice::ExclusiveFallback { text } => (text, theme::warning(), false),
            PlaybackNotice::Stopped { text } => (text, theme::danger(), false),
            PlaybackNotice::DeviceFailure { text } => (text, theme::danger(), true),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .px(px(20.))
            .py(px(7.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(color)
                    .child(text),
            )
            .when(recovery, |banner| {
                banner
                    .child(
                        ui::Button::new("playback-notice-retry", "Try again")
                            .size(ui::ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, _, cx| this.retry_playback(cx))),
                    )
                    .child(
                        ui::Button::new("playback-notice-outputs", "Choose output")
                            .size(ui::ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_output_popover(cx))),
                    )
            })
            .child(
                div()
                    .id("playback-notice-dismiss")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_notice(cx)))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(px(13.))
                            .text_color(theme::text_muted()),
                    ),
            )
    }
}

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.))
        .text_color(theme::text_muted())
        .child(label)
}

// The saved UID is deliberately left untouched on fallback: an absent device
// (undocked DAC, sleeping AirPods) is a normal state, and the preference must
// win again the next time the device is present.
