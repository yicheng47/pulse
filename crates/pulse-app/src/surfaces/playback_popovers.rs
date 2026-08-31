use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ListSizingBehavior, MouseButton, MouseMoveEvent,
    MouseUpEvent, canvas, div, prelude::*, relative, svg, uniform_list,
};
use pulse_engine::device;

use crate::{
    backend::{
        SignalPathVerdict, StoredOutputMode, TrackRef, displayed_volume_level,
        format_device_capabilities, format_quality, format_queue_meta, format_queue_time,
        format_sample_rate, format_volume_percent, output_mode_meta, signal_path_verdict,
        volume_control_level, volume_control_muted,
    },
    surfaces::PlaybackRow,
    theme, ui,
};

impl PlaybackRow {
    pub(super) fn render_volume_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let volume_bounds = Rc::clone(&self.volume_bounds);
        let fixed = self.snapshot.volume_state.domain == pulse_engine::VolumeDomain::Fixed;
        let volume_level =
            volume_control_level(self.snapshot.volume_level, self.snapshot.volume_state);
        let volume_muted =
            volume_control_muted(self.snapshot.volume_muted, self.snapshot.volume_state);
        let volume_fill = displayed_volume_level(volume_level, volume_muted);
        let volume_dragging = self.volume_dragging;
        let domain = match self.snapshot.volume_state.domain {
            pulse_engine::VolumeDomain::Device => "DEVICE",
            pulse_engine::VolumeDomain::Software => "SOFTWARE",
            pulse_engine::VolumeDomain::Fixed => "FIXED",
        };
        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("volume-popover", rpx(56.))
            .left(rpx(-19.5))
            .bottom(rpx(54.))
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
                    .text_size(theme::text::SMALL)
                    .text_center()
                    .text_color(
                        if self
                            .snapshot
                            .volume_state
                            .transparent(self.snapshot.volume_level, self.snapshot.volume_muted)
                        {
                            theme::quality()
                        } else {
                            theme::text_primary()
                        },
                    )
                    .whitespace_nowrap()
                    .child(format_volume_percent(volume_level)),
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
                    .h(rpx(120.))
                    .flex_none()
                    .when(fixed, |slider| slider.cursor_default().opacity(0.45))
                    .when(!fixed, |slider| slider.cursor_pointer())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.begin_volume_drag(event, cx)),
                    )
                    .child(
                        div()
                            .relative()
                            .w(rpx(4.))
                            .h_full()
                            .rounded(rpx(2.))
                            .bg(theme::bg_inset())
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(relative(volume_fill))
                                    .rounded(rpx(2.))
                                    .bg(if fixed {
                                        theme::text_muted()
                                    } else {
                                        theme::accent()
                                    })
                                    .child(
                                        div()
                                            .absolute()
                                            .top(rpx(-6.))
                                            .left(rpx(-4.))
                                            .size(rpx(12.))
                                            .rounded(rpx(6.))
                                            .bg(if fixed {
                                                theme::text_muted()
                                            } else {
                                                theme::accent()
                                            })
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
                    .size(rpx(17.))
                    .flex_none()
                    .when(fixed, |mute| mute.cursor_default().opacity(0.35))
                    .when(!fixed, |mute| {
                        mute.cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_volume_mute(cx)))
                    })
                    .child(
                        svg()
                            .path("icons/volume-x.svg")
                            .size(rpx(17.))
                            .text_color(theme::text_secondary()),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme::text::CAPTION_XXS)
                    .text_center()
                    .text_color(theme::text_muted())
                    .child(domain),
            );
        if let Some(focus) = &self.volume_popover_focus {
            popover = popover.focus_handle(focus.clone());
        }
        popover.into_any_element()
    }

    pub(super) fn render_signal_path_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let verdict = signal_path_verdict(
            self.snapshot.bit_perfect_active,
            self.snapshot.volume_state,
            self.snapshot.volume_level,
            self.snapshot.volume_muted,
        );
        let (verdict_label, verdict_color, verdict_background, verdict_border, footer) =
            match verdict {
                SignalPathVerdict::BitPerfect => (
                    "BIT-PERFECT",
                    theme::quality(),
                    theme::quality_soft(),
                    theme::quality_border(),
                    "Source integers reach the DAC unmodified.",
                ),
                SignalPathVerdict::Transparent => (
                    "TRANSPARENT",
                    theme::primary(),
                    theme::primary_soft(),
                    theme::border_strong(),
                    "Samples leave Pulse untouched. Core Audio owns the conversion past the client boundary.",
                ),
                SignalPathVerdict::Processed => (
                    "PROCESSED",
                    theme::warning(),
                    theme::bg_elevated(),
                    theme::border_strong(),
                    "Software gain multiplies samples in the float path. Set volume to 100% for a transparent path.",
                ),
            };
        let source_detail = self.snapshot.format.map_or_else(
            || "No source loaded".to_string(),
            |format| {
                format!(
                    "{} / {}",
                    format_quality(self.snapshot.source_path.as_deref(), format),
                    format_sample_rate(format.sample_rate)
                )
            },
        );
        let volume_detail = match self.snapshot.volume_state.domain {
            pulse_engine::VolumeDomain::Device => "Device volume · DAC attenuator".to_string(),
            pulse_engine::VolumeDomain::Software
                if self
                    .snapshot
                    .volume_state
                    .software_unity(self.snapshot.volume_level, self.snapshot.volume_muted) =>
            {
                "Software · unity — untouched".to_string()
            }
            pulse_engine::VolumeDomain::Software => {
                software_volume_detail(self.snapshot.volume_level, self.snapshot.volume_muted)
            }
            pulse_engine::VolumeDomain::Fixed => "Fixed · 100% — no gain stage".to_string(),
        };
        let engine_detail = if self.snapshot.bit_perfect_active {
            "Bit-perfect · hog held · mixing off"
        } else {
            "Universal · AUHAL · float32"
        };
        let output_detail = match (&self.snapshot.active_device, self.snapshot.format) {
            (Some(device), Some(format)) if self.snapshot.bit_perfect_active => format!(
                "{} · {}/{} integer",
                device.name,
                format.bits_per_sample,
                compact_sample_rate(format.sample_rate)
            ),
            (Some(device), Some(_)) => {
                format!("{} · Core Audio client · float32", device.name)
            }
            (Some(device), None) => device.name.clone(),
            (None, _) => "No active output".to_string(),
        };
        let (source_dot, decode_dot, volume_dot, engine_dot, output_dot, volume_text) =
            match verdict {
                SignalPathVerdict::BitPerfect => (
                    theme::quality(),
                    theme::quality(),
                    theme::quality(),
                    theme::quality(),
                    theme::quality(),
                    theme::text_secondary(),
                ),
                SignalPathVerdict::Transparent => (
                    theme::quality(),
                    theme::quality(),
                    theme::quality(),
                    theme::text_muted(),
                    theme::text_muted(),
                    theme::text_secondary(),
                ),
                SignalPathVerdict::Processed => (
                    theme::quality(),
                    theme::quality(),
                    theme::warning(),
                    theme::text_muted(),
                    theme::text_muted(),
                    theme::warning(),
                ),
            };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(section_label("SIGNAL PATH"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(5.))
                    .px(rpx(8.))
                    .py(rpx(3.))
                    .rounded(rpx(theme::RADIUS_SM))
                    .border_1()
                    .border_color(verdict_border)
                    .bg(verdict_background)
                    .child(div().size(rpx(6.)).rounded(rpx(3.)).bg(verdict_color))
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::CAPTION_XS)
                            .text_color(verdict_color)
                            .child(verdict_label),
                    ),
            );
        let chain = div()
            .flex()
            .flex_col()
            .w_full()
            .child(signal_path_stage(
                "Source",
                source_detail,
                source_dot,
                theme::text_secondary(),
            ))
            .child(signal_path_connector())
            .child(signal_path_stage(
                "Decode",
                "PCM decode · no Pulse resampling".to_string(),
                decode_dot,
                theme::text_secondary(),
            ))
            .child(signal_path_connector())
            .child(signal_path_stage(
                "Volume",
                volume_detail,
                volume_dot,
                volume_text,
            ))
            .child(signal_path_connector())
            .child(signal_path_stage(
                "Engine",
                engine_detail.to_string(),
                engine_dot,
                theme::text_secondary(),
            ))
            .child(signal_path_connector())
            .child(signal_path_stage(
                "Output",
                output_detail,
                output_dot,
                theme::text_secondary(),
            ));
        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("signal-path-popover", rpx(372.))
            .right(rpx(0.))
            .bottom(rpx(31.))
            .on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.signal_popover_open = false;
                    cx.notify();
                });
            })
            .child(header)
            .child(chain)
            .child(
                div()
                    .w_full()
                    .h(gpui::px(1.)) // physical
                    .flex_none()
                    .bg(theme::border()),
            )
            .child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::CAPTION)
                    .line_height(rpx(14.5))
                    .text_color(theme::text_muted())
                    .child(footer),
            );
        if let Some(focus) = &self.signal_popover_focus {
            popover = popover.focus_handle(focus.clone());
        }
        popover.into_any_element()
    }

    pub(super) fn render_output_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_name = self
            .snapshot
            .active_device
            .as_ref()
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "No active output".to_string());
        let capability = self
            .snapshot
            .device_capabilities
            .map(format_device_capabilities)
            .unwrap_or_else(|| "Capabilities unavailable".to_string());
        let bit_perfect_available = self
            .snapshot
            .device_capabilities
            .is_some_and(|capabilities| {
                capabilities.max_bits_per_channel.is_some()
                    && capabilities.transport.supports_bit_perfect()
            });
        let shared = ui::output_mode_segment(
            "output-mode-shared",
            "Shared",
            self.snapshot.output_mode == StoredOutputMode::Shared,
            false,
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.set_output_mode(StoredOutputMode::Shared, cx);
        }))
        .into_any_element();
        let exclusive = ui::output_mode_segment(
            "output-mode-exclusive",
            "Exclusive",
            self.snapshot.output_mode == StoredOutputMode::Exclusive,
            false,
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.set_output_mode(StoredOutputMode::Exclusive, cx);
        }))
        .into_any_element();
        let bit_perfect = ui::output_mode_segment(
            "output-mode-bit-perfect",
            "Bit-perfect",
            self.snapshot.output_mode == StoredOutputMode::BitPerfect,
            true,
            !bit_perfect_available,
        );
        let bit_perfect = if bit_perfect_available {
            bit_perfect
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_output_mode(StoredOutputMode::BitPerfect, cx);
                }))
                .into_any_element()
        } else {
            bit_perfect.into_any_element()
        };
        let mut direct_devices = div().flex().flex_col().gap(rpx(2.)).w_full();
        for (index, output_device) in self.snapshot.devices.iter().cloned().enumerate() {
            direct_devices =
                direct_devices.child(self.render_output_device_row(output_device, index, cx));
        }
        if self.snapshot.devices.is_empty() {
            direct_devices = direct_devices.child(
                div()
                    .px(rpx(10.))
                    .py(rpx(9.))
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(theme::text_muted())
                    .child("No direct output devices found"),
            );
        }

        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("output-device-popover", rpx(360.))
            .right(rpx(-52.))
            .bottom(rpx(54.))
            .on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.output_popover_open = false;
                    cx.notify();
                });
            });
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
                            .text_size(theme::text::TITLE)
                            .text_color(theme::text_primary())
                            .child("Choose audio output"),
                    )
                    .child(
                        svg()
                            .path("icons/settings.svg")
                            .size(rpx(16.))
                            .text_color(theme::text_secondary()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .p(rpx(12.))
                    .rounded(rpx(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::bg_inset())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(12.))
                            .w_full()
                            .child(
                                svg()
                                    .path("icons/speaker.svg")
                                    .size(rpx(22.))
                                    .flex_none()
                                    .text_color(theme::accent()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_col()
                                    .gap(rpx(3.))
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_DISPLAY)
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(theme::text::TITLE)
                                            .text_color(theme::text_primary())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(active_name),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_SANS)
                                            .text_size(theme::text::BODY)
                                            .text_color(theme::text_secondary())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(output_mode_meta(
                                                self.snapshot.playback_output_mode,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .font_family(theme::FONT_MONO)
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(theme::text::SMALL)
                                            .text_color(
                                                if self.snapshot.device_capabilities.is_some() {
                                                    theme::quality()
                                                } else {
                                                    theme::warning()
                                                },
                                            )
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(capability),
                                    ),
                            )
                            .when(self.snapshot.active_device.is_some(), |device| {
                                device.child(
                                    svg()
                                        .path("icons/check.svg")
                                        .size(rpx(18.))
                                        .flex_none()
                                        .text_color(theme::accent()),
                                )
                            }),
                    )
                    .when(self.snapshot.active_device.is_some(), |card| {
                        card.child(
                            div()
                                .w_full()
                                .h(gpui::px(1.)) // physical
                                .my(rpx(10.))
                                .bg(theme::border()),
                        )
                        .child(ui::output_mode_control(
                            "Mode",
                            self.output_mode_is_automatic(),
                            bit_perfect_available,
                            ui::output_mode_reset_link("output-mode-reset-auto")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reset_output_mode_to_auto(cx);
                                }))
                                .into_any_element(),
                            ui::output_mode_segments(shared, exclusive, bit_perfect)
                                .into_any_element(),
                        ))
                    }),
            );

        if let Some(message) = self.displayed_device_message() {
            popover = popover.child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::SMALL)
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
                    .text_size(theme::text::BODY)
                    .text_color(theme::text_muted())
                    .child("No network devices found"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .pt(rpx(12.))
                    .pr(rpx(2.))
                    .pb(rpx(2.))
                    .pl(rpx(2.))
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(theme::text::LABEL)
                            .text_color(theme::text_secondary())
                            .child("Can't find your device?"),
                    )
                    .child(
                        svg()
                            .path("icons/log-in.svg")
                            .size(rpx(16.))
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
            .snapshot
            .active_device
            .as_ref()
            .is_some_and(|active| active.uid == output_device.uid);
        let selected_device = output_device.clone();

        div()
            .id(("output-device", index))
            .flex()
            .items_center()
            .gap(rpx(12.))
            .w_full()
            .px(rpx(10.))
            .py(rpx(9.))
            .rounded(rpx(theme::RADIUS_MD))
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
                    .size(rpx(18.))
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
                    .text_size(theme::text::LABEL)
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
                        .size(rpx(16.))
                        .flex_none()
                        .text_color(theme::accent()),
                )
            })
    }

    pub(super) fn render_queue_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let upcoming_count = self.snapshot.queue.remaining_count();

        let mut header = div().flex().items_center().gap(rpx(10.)).w_full().child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(theme::text::TITLE)
                .text_color(theme::text_primary())
                .child("Queue"),
        );
        if upcoming_count > 0 {
            header = header
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .font_weight(FontWeight::BOLD)
                        .text_size(theme::text::CAPTION)
                        .text_color(theme::text_muted())
                        .child(format_queue_meta(
                            upcoming_count,
                            self.snapshot.queue.upcoming_duration_ms(),
                        )),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("queue-clear")
                        .flex()
                        .items_center()
                        .h(rpx(23.))
                        .px(rpx(8.))
                        .flex_none()
                        .rounded(rpx(theme::RADIUS_SM))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg_muted())
                        .cursor_pointer()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::BOLD)
                        .text_size(theme::text::BODY)
                        .text_color(theme::text_secondary())
                        .child("Clear")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_upcoming_queue(cx))),
                );
        }

        let entity = cx.entity();
        let mut popover = ui::PopoverMenu::new("queue-popover", rpx(376.))
            .right(rpx(0.))
            .bottom(rpx(71.))
            .max_height(rpx(541.))
            .on_dismiss(move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.queue_popover_open = false;
                    cx.notify();
                });
            })
            .child(header);

        if let Some((title, secondary)) = self.snapshot.now_playing_lines() {
            popover = popover.child(section_label("NOW PLAYING")).child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(10.))
                    .w_full()
                    .h(rpx(58.))
                    .flex_none()
                    .px(rpx(10.))
                    .relative()
                    .overflow_hidden()
                    .rounded(rpx(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_inset())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
                    .child(
                        svg()
                            .path("icons/audio-lines.svg")
                            .size(rpx(16.))
                            .flex_none()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(rpx(2.))
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme::text::LABEL)
                                    .text_color(theme::text_primary())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(theme::text::SMALL)
                                    .text_color(theme::text_secondary())
                                    .child(secondary),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::CAPTION)
                            .text_color(theme::quality())
                            .child(format!(
                                "{} / {}",
                                format_queue_time(self.displayed_position_ms()),
                                self.snapshot
                                    .duration_ms
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
                    .px(rpx(10.))
                    .py(rpx(6.))
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
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
                            .snapshot
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
            .gap(rpx(10.))
            .w_full()
            .h(rpx(50.))
            .flex_none()
            .px(rpx(10.))
            .rounded(rpx(theme::RADIUS_MD))
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
                    .min_w(rpx(18.))
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(theme::text::SMALL)
                    .text_color(theme::text_muted())
                    .child(position.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_col()
                    .gap(rpx(1.))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text::BODY_LARGE)
                            .text_color(theme::text_primary())
                            .child(track.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(theme::text::SMALL)
                            .text_color(theme::text_secondary())
                            .child(format!("{} · {}", track.artist, track.album)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(theme::text::SMALL)
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
                        .size(rpx(14.))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_queue_entry(index, cx);
                        }))
                        .child(
                            svg()
                                .path("icons/x.svg")
                                .size(rpx(14.))
                                .text_color(theme::text_muted()),
                        ),
                )
            });

        div()
            .w_full()
            .h(rpx(52.))
            .pb(rpx(2.))
            .child(row)
            .into_any_element()
    }
}

fn software_volume_detail(level: f32, muted: bool) -> String {
    if muted {
        return "Software · muted".to_string();
    }
    let percent = format_volume_percent(level);
    let gain = pulse_engine::volume_gain_for_level(level);
    if gain == 0.0 {
        format!("Software · {percent} (−∞ dB)")
    } else {
        format!("Software · {percent} (−{:.1} dB)", -(20.0 * gain.log10()))
    }
}

fn compact_sample_rate(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1_000) {
        (sample_rate / 1_000).to_string()
    } else {
        format!("{:.1}", f64::from(sample_rate) / 1_000.0)
    }
}

fn signal_path_stage(
    stage: &'static str,
    detail: String,
    dot_color: gpui::Rgba,
    detail_color: gpui::Rgba,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(10.))
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(7.))
                .flex_none()
                .child(div().size(rpx(5.)).rounded(rpx(2.5)).bg(dot_color))
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(theme::text::BODY)
                        .text_color(theme::text_primary())
                        .child(stage),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme::FONT_MONO)
                .text_size(theme::text::CAPTION)
                .text_color(detail_color)
                .text_right()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(detail),
        )
}

fn signal_path_connector() -> impl IntoElement {
    div().w_full().h(rpx(8.)).pl(rpx(2.)).child(
        div()
            .w(gpui::px(1.)) // physical
            .h_full()
            .bg(theme::border_strong()),
    )
}

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(theme::text::CAPTION)
        .text_color(theme::text_muted())
        .child(label)
}
