use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ListSizingBehavior, MouseButton, MouseMoveEvent,
    MouseUpEvent, canvas, div, prelude::*, relative, svg, uniform_list,
};
use pulse_engine::device;

use crate::{
    backend::{
        TrackRef, displayed_volume_level, format_device_capabilities, format_queue_meta,
        format_queue_time, format_volume_percent,
    },
    surfaces::PlaybackRow,
    theme, ui,
};

impl PlaybackRow {
    pub(super) fn render_volume_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let volume_bounds = Rc::clone(&self.volume_bounds);
        let volume_fill =
            displayed_volume_level(self.snapshot.volume_level, self.snapshot.volume_muted);
        let volume_dragging = self.volume_dragging;
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
                    .text_color(theme::text_secondary())
                    .whitespace_nowrap()
                    .child(format_volume_percent(self.snapshot.volume_level)),
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
                    .cursor_pointer()
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
                                    .bg(theme::accent())
                                    .child(
                                        div()
                                            .absolute()
                                            .top(rpx(-6.))
                                            .left(rpx(-4.))
                                            .size(rpx(12.))
                                            .rounded(rpx(6.))
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
                    .size(rpx(17.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_volume_mute(cx)))
                    .child(
                        svg()
                            .path("icons/volume-x.svg")
                            .size(rpx(17.))
                            .text_color(theme::text_secondary()),
                    ),
            );
        if let Some(focus) = &self.volume_popover_focus {
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
                                            .child(if self.snapshot.playback_exclusive_mode {
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
                        .child(ui::exclusive_mode_control(
                            self.exclusive_mode_is_automatic(),
                            ui::exclusive_mode_reset_link("exclusive-mode-reset-auto")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reset_exclusive_mode_to_auto(cx);
                                }))
                                .into_any_element(),
                            ui::Toggle::new("exclusive-mode-toggle", self.snapshot.exclusive_mode)
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

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(theme::text::CAPTION)
        .text_color(theme::text_muted())
        .child(label)
}
