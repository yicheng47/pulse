use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Context, Entity, FocusHandle, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, Subscription, Window, canvas, div,
    img, prelude::*, px, relative, svg,
};
use pulse_engine::{PlaybackState, device};

use crate::{
    app_store::{AppStore, StoreRevisions, global_app_store},
    backend::{
        DeviceMessage, PlaybackAction, PlaybackNotice, PlaybackSnapshot, RepeatMode,
        VolumeIconState, format_output_device, format_quality, format_time, fraction_at_x,
        fraction_at_y, scrub_position_ms, volume_icon_state,
    },
    theme,
    ui::{self, IconButtonVariant},
};

use super::playback_row_logic::{
    PendingToggle, begin_pending_toggle, reconcile_pending_toggle, transport_presentation,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum PlaybackSurface {
    Transport,
    SettingsOutputPicker,
}

pub(crate) struct PlaybackRow {
    pub(super) app_store: Entity<AppStore>,
    pub(super) store_revisions: StoreRevisions,
    pub(super) snapshot: PlaybackSnapshot,
    pub(super) surface: PlaybackSurface,
    pub(super) volume_popover_open: bool,
    pub(super) volume_toggle_press_closed_popover: bool,
    pub(super) output_popover_open: bool,
    pub(super) output_toggle_press_closed_popover: bool,
    pub(super) queue_popover_open: bool,
    pub(super) queue_toggle_press_closed_popover: bool,
    pub(super) hovered_upcoming: Option<usize>,
    pub(super) volume_popover_focus: Option<FocusHandle>,
    pub(super) queue_popover_focus: Option<FocusHandle>,
    pub(super) track_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(super) scrubbing: bool,
    pub(super) scrub_fraction: Option<f32>,
    pub(super) volume_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(super) volume_dragging: bool,
    pending_toggle: Option<PendingToggle>,
    _store_subscription: Subscription,
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
            pending_toggle: None,
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
            if reactions.playback {
                self.pending_toggle =
                    reconcile_pending_toggle(self.pending_toggle, self.snapshot.playback_state);
            }
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

    pub(super) fn toggle_volume_mute(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::ToggleVolumeMute, cx);
    }

    fn set_volume_level(&mut self, level: f32, cx: &mut Context<Self>) {
        self.send(PlaybackAction::SetVolumeLevel(level), cx);
    }

    fn persist_volume(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::PersistVolume, cx);
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        self.pending_toggle = begin_pending_toggle(
            self.snapshot.playback_state,
            self.snapshot.source_path.is_some(),
        );
        self.send(PlaybackAction::TogglePlayback, cx);
        cx.notify();
    }

    fn next_track(&mut self, cx: &mut Context<Self>) {
        self.pending_toggle = None;
        self.send(PlaybackAction::NextTrack, cx);
        cx.notify();
    }

    fn previous_track(&mut self, cx: &mut Context<Self>) {
        self.pending_toggle = None;
        self.send(PlaybackAction::PreviousTrack, cx);
        cx.notify();
    }

    fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::ToggleShuffle, cx);
    }

    fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::CycleRepeat, cx);
    }

    pub(super) fn jump_to_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.pending_toggle = None;
        self.send(PlaybackAction::JumpToQueueEntry(index), cx);
        cx.notify();
    }

    pub(super) fn remove_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.hovered_upcoming = None;
        self.send(PlaybackAction::RemoveQueueEntry(index), cx);
    }

    pub(super) fn clear_upcoming_queue(&mut self, cx: &mut Context<Self>) {
        self.hovered_upcoming = None;
        self.send(PlaybackAction::ClearUpcomingQueue, cx);
    }

    fn retry_playback(&mut self, cx: &mut Context<Self>) {
        self.pending_toggle = None;
        self.send(PlaybackAction::RetryPlayback, cx);
        cx.notify();
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.send(PlaybackAction::DismissNotice, cx);
    }

    pub(super) fn displayed_device_message(&self) -> Option<DeviceMessage> {
        self.snapshot.device_message.clone()
    }

    pub(super) fn exclusive_mode_is_automatic(&self) -> bool {
        self.snapshot.exclusive_mode_automatic
    }

    pub(super) fn toggle_exclusive_mode(&mut self, cx: &mut Context<Self>) {
        let Some(active_device) = self.snapshot.active_device.clone() else {
            return;
        };
        self.send(
            PlaybackAction::ToggleDeviceExclusiveMode {
                device_uid: active_device.uid,
                default: self.snapshot.default_exclusive_mode,
            },
            cx,
        );
    }

    pub(super) fn reset_exclusive_mode_to_auto(&mut self, cx: &mut Context<Self>) {
        let Some(active_device) = self.snapshot.active_device.clone() else {
            return;
        };
        self.send(
            PlaybackAction::ResetDeviceExclusiveMode {
                device_uid: active_device.uid,
                default: self.snapshot.default_exclusive_mode,
            },
            cx,
        );
    }

    pub(super) fn select_output_device(
        &mut self,
        output_device: device::Device,
        cx: &mut Context<Self>,
    ) {
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
            self.snapshot.playback_state,
            PlaybackState::Playing | PlaybackState::Paused
        ) || self
            .snapshot
            .duration_ms
            .is_none_or(|duration_ms| duration_ms == 0)
        {
            return;
        }
        let Some(bounds) = self.track_bounds.get() else {
            return;
        };
        self.scrubbing = true;
        self.scrub_fraction = Some(surface_fraction_at_x(bounds, event.position.x));
        cx.notify();
    }

    pub(super) fn begin_volume_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.volume_bounds.get() else {
            return;
        };
        self.volume_dragging = true;
        self.set_volume_level(surface_fraction_at_y(bounds, event.position.y), cx);
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
            self.scrub_fraction = Some(surface_fraction_at_x(bounds, event.position.x));
            cx.notify();
        }
        if self.volume_dragging
            && let Some(bounds) = self.volume_bounds.get()
        {
            self.set_volume_level(surface_fraction_at_y(bounds, event.position.y), cx);
        }
    }

    pub(crate) fn finish_drag(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.scrubbing {
            self.scrubbing = false;
            let fraction = self
                .track_bounds
                .get()
                .map(|bounds| surface_fraction_at_x(bounds, event.position.x))
                .or(self.scrub_fraction);
            self.scrub_fraction = None;
            if let (Some(fraction), Some(duration_ms)) = (fraction, self.snapshot.duration_ms) {
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
                self.set_volume_level(surface_fraction_at_y(bounds, event.position.y), cx);
            }
            self.persist_volume(cx);
            cx.notify();
        }
    }

    fn displayed_fraction(&self) -> f32 {
        self.scrub_fraction
            .unwrap_or_else(|| self.snapshot.displayed_fraction())
    }

    pub(super) fn displayed_position_ms(&self) -> u64 {
        match (self.scrub_fraction, self.snapshot.duration_ms) {
            (Some(fraction), Some(duration_ms)) => scrub_position_ms(fraction, duration_ms),
            _ => self.snapshot.position_ms,
        }
    }

    fn render_now_playing(&self) -> impl IntoElement {
        let cover = match &self.snapshot.cover_art_path {
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
                                    .child(self.snapshot.title.clone()),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(theme::text_secondary())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(self.snapshot.secondary.clone()),
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
        let transport = transport_presentation(
            self.snapshot.playback_state,
            self.pending_toggle,
            self.snapshot.source_path.is_some(),
            self.snapshot.queue.can_advance(),
        );
        let play_icon = if transport.show_pause {
            "icons/pause.svg"
        } else {
            "icons/play.svg"
        };
        let shuffle_enabled = self.snapshot.queue.shuffle_enabled();
        let repeat_mode = self.snapshot.queue.repeat_mode();
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
                        ui::IconButton::new("playback-shuffle", "icons/shuffle.svg")
                            .variant(if shuffle_enabled {
                                IconButtonVariant::Accent
                            } else {
                                IconButtonVariant::Secondary
                            })
                            .button_size(28.)
                            .icon_size(19.)
                            .horizontal_margin(-4.5)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))),
                    )
                    .child(
                        ui::IconButton::new("playback-previous", "icons/skip-back.svg")
                            .variant(IconButtonVariant::Secondary)
                            .button_size(28.)
                            .icon_size(19.)
                            .horizontal_margin(-4.5)
                            .disabled(!transport.previous_enabled)
                            .disabled_opacity(0.35)
                            .on_click(cx.listener(|this, _, _, cx| this.previous_track(cx))),
                    )
                    .child(
                        ui::IconButton::new("playback-toggle", play_icon)
                            .variant(IconButtonVariant::Primary)
                            .button_size(34.)
                            .icon_size(18.)
                            .corner_radius(theme::RADIUS_MD)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx))),
                    )
                    .child(
                        ui::IconButton::new("playback-next", "icons/skip-forward.svg")
                            .variant(IconButtonVariant::Secondary)
                            .button_size(28.)
                            .icon_size(19.)
                            .horizontal_margin(-4.5)
                            .disabled(!transport.next_enabled)
                            .disabled_opacity(0.35)
                            .on_click(cx.listener(|this, _, _, cx| this.next_track(cx))),
                    )
                    .child(
                        ui::IconButton::new("playback-repeat", repeat_icon)
                            .variant(if repeat_mode == RepeatMode::Off {
                                IconButtonVariant::Secondary
                            } else {
                                IconButtonVariant::Accent
                            })
                            .button_size(28.)
                            .icon_size(19.)
                            .horizontal_margin(-4.5)
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
                    ),
            )
            .child(
                div().flex().items_center().w(px(44.)).flex_none().child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(10.))
                        .text_color(theme::text_muted())
                        .child(
                            self.snapshot
                                .duration_ms
                                .map(format_time)
                                .unwrap_or_else(|| "--:--".to_string()),
                        ),
                ),
            )
    }

    fn render_output(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let remaining = self.snapshot.queue.remaining_count();
        let volume_icon = volume_icon_state(self.snapshot.volume_level, self.snapshot.volume_muted);
        let (quality, quality_color) = self
            .snapshot
            .format
            .map(|format| {
                (
                    format_quality(self.snapshot.source_path.as_deref(), format),
                    theme::quality(),
                )
            })
            .unwrap_or_else(|| ("—".to_string(), theme::text_muted()));
        let device = match (self.snapshot.format, &self.snapshot.active_device) {
            (Some(format), Some(device)) => format_output_device(
                format.sample_rate,
                &device.name,
                self.snapshot.playback_exclusive_mode,
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
            .when_some(self.snapshot.notice.clone(), |column, notice| {
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
            PlaybackNotice::Dropouts { text } => (text, theme::warning(), false),
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

fn surface_fraction_at_x(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    fraction_at_x(bounds.origin.x.into(), bounds.size.width.into(), x.into())
}

fn surface_fraction_at_y(bounds: Bounds<Pixels>, y: Pixels) -> f32 {
    fraction_at_y(bounds.origin.y.into(), bounds.size.height.into(), y.into())
}

// The saved UID is deliberately left untouched on fallback: an absent device
// (undocked DAC, sleeping AirPods) is a normal state, and the preference must
// win again the next time the device is present.
