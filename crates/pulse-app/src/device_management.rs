use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Render, ScrollHandle, Size, StatefulInteractiveElement, Window, div,
    point, prelude::*, px, svg,
};

use crate::{
    components,
    playback_row::{ManagedDevice, PlaybackRow, format_stored_device_capabilities},
    theme,
};

const SCROLLBAR_INSET_PX: f32 = 4.;
const SCROLLBAR_MIN_THUMB_PX: f32 = 36.;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarMetrics {
    thumb_height: Pixels,
    thumb_top: Pixels,
    travel: Pixels,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutSignature {
    connected: usize,
    not_connected: usize,
    messages: Vec<(String, bool)>,
}

pub(crate) struct DeviceManagementPage {
    row: Entity<PlaybackRow>,
    forget_device_uid: Option<String>,
    scroll: ScrollHandle,
    scroll_drag_offset: Option<Pixels>,
    last_window_size: Option<Size<Pixels>>,
    last_layout_signature: Option<LayoutSignature>,
    scrollbar_measure_pending: bool,
}

impl DeviceManagementPage {
    pub(crate) fn new(row: Entity<PlaybackRow>, cx: &mut Context<Self>) -> Self {
        cx.observe(&row, |_, _, cx| cx.notify()).detach();
        Self {
            row,
            forget_device_uid: None,
            scroll: ScrollHandle::new(),
            scroll_drag_offset: None,
            last_window_size: None,
            last_layout_signature: None,
            scrollbar_measure_pending: true,
        }
    }

    fn render_group(
        &self,
        label: &'static str,
        devices: &[ManagedDevice],
        index_offset: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut cards = div().flex().flex_col().gap(px(10.)).w_full();
        for (index, device) in devices.iter().enumerate() {
            cards = cards.child(self.render_device_card(device, index_offset + index, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .w_full()
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(theme::text_muted())
                    .child(label),
            )
            .child(cards)
            .into_any_element()
    }

    fn render_device_card(
        &self,
        device: &ManagedDevice,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let class = device_class(device);
        let capability = device
            .capabilities
            .map(format_stored_device_capabilities)
            .unwrap_or_else(|| "Capabilities unavailable".to_string());
        let metadata = format!("{class} · {capability}");
        let status_color = if device.connected {
            theme::quality()
        } else {
            theme::text_muted()
        };
        let icon = if device
            .capabilities
            .is_some_and(|capabilities| capabilities.max_bits_per_channel.is_none())
        {
            "icons/bluetooth.svg"
        } else {
            "icons/speaker.svg"
        };

        let mut actions = div().flex().items_center().gap(px(8.)).flex_none();
        if device.active {
            actions = actions.child(status_pill("Active", true));
        }
        if device.saved_default {
            actions = actions.child(status_pill("Default", false));
        } else if device.can_set_as_default() {
            let row = self.row.clone();
            let uid = device.uid.clone();
            actions = actions.child(
                device_action_button(("set-default-device", index), "Set as default").on_click(
                    move |_, _, cx| {
                        row.update(cx, |row, cx| {
                            row.set_managed_device_as_default(&uid, cx);
                        });
                    },
                ),
            );
        }
        if device.can_forget() {
            let uid = device.uid.clone();
            actions = actions.child(
                device_action_button(("forget-device", index), "Forget").on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.forget_device_uid = Some(uid.clone());
                        cx.notify();
                    },
                )),
            );
        }

        let mut copy = div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(17.))
                    .text_color(theme::text_primary())
                    .child(device.name.clone()),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .text_color(status_color)
                    .child(metadata),
            );
        if !device.connected {
            copy = copy.child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(format_last_seen(device.last_seen_unix_seconds)),
            );
        }

        let reset_row = self.row.clone();
        let reset_uid = device.uid.clone();
        let reset_default = device.default_exclusive_mode;
        let toggle_row = self.row.clone();
        let toggle_uid = device.uid.clone();
        let toggle_default = device.default_exclusive_mode;
        div()
            .id(("managed-device", index))
            .flex()
            .flex_col()
            .gap(px(10.))
            .w_full()
            .p(px(14.))
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(if device.active {
                theme::accent()
            } else {
                theme::border()
            })
            .bg(theme::bg_inset())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .child(svg().path(icon).size(px(22.)).flex_none().text_color(
                        if device.connected {
                            theme::accent()
                        } else {
                            theme::text_muted()
                        },
                    ))
                    .child(copy)
                    .child(actions),
            )
            .child(div().w_full().h(px(1.)).bg(theme::border()))
            .child(components::exclusive_mode_control(
                device.automatic,
                components::exclusive_mode_reset_link(("device-mode-reset", index))
                    .on_click(move |_, _, cx| {
                        reset_row.update(cx, |row, cx| {
                            row.reset_device_exclusive_mode_to_auto(
                                reset_uid.clone(),
                                reset_default,
                                cx,
                            );
                        });
                    })
                    .into_any_element(),
                components::toggle(("device-mode-toggle", index), device.exclusive_mode)
                    .on_click(move |_, _, cx| {
                        toggle_row.update(cx, |row, cx| {
                            row.toggle_device_exclusive_mode(
                                toggle_uid.clone(),
                                toggle_default,
                                cx,
                            );
                        });
                    })
                    .into_any_element(),
            ))
            .into_any_element()
    }

    fn render_scrollbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let viewport_bounds = self.scroll.bounds();
        let viewport_height = viewport_bounds.size.height;
        let max_scroll = self.scroll.max_offset().y.max(px(0.));
        let metrics = scrollbar_metrics(viewport_height, max_scroll, self.scroll.offset().y)?;
        let viewport_top = viewport_bounds.top();

        Some(
            div()
                .id("device-management-scrollbar")
                .absolute()
                .top(px(SCROLLBAR_INSET_PX))
                .right(px(2.))
                .bottom(px(SCROLLBAR_INSET_PX))
                .w(px(4.))
                .rounded(px(2.))
                .bg(theme::bg_muted())
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let target_top = (event.position.y
                            - viewport_top
                            - px(SCROLLBAR_INSET_PX)
                            - metrics.thumb_height / 2.)
                            .clamp(px(0.), metrics.travel);
                        let progress = target_top / metrics.travel;
                        this.scroll
                            .set_offset(point(px(0.), -(max_scroll * progress)));
                        this.scroll_drag_offset = Some(metrics.thumb_height / 2.);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("device-management-scrollbar-thumb")
                        .absolute()
                        .top(metrics.thumb_top)
                        .left_0()
                        .w_full()
                        .h(metrics.thumb_height)
                        .rounded(px(2.))
                        .bg(if self.scroll_drag_offset.is_some() {
                            theme::text_secondary()
                        } else {
                            theme::text_muted()
                        })
                        .hover(|thumb| thumb.bg(theme::text_secondary()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.scroll_drag_offset = Some(
                                    event.position.y
                                        - viewport_top
                                        - px(SCROLLBAR_INSET_PX)
                                        - metrics.thumb_top,
                                );
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn update_scrollbar_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag_offset) = self.scroll_drag_offset else {
            return;
        };
        if !event.dragging() {
            self.scroll_drag_offset = None;
            return;
        }
        let viewport_bounds = self.scroll.bounds();
        let viewport_height = viewport_bounds.size.height;
        let max_scroll = self.scroll.max_offset().y.max(px(0.));
        let Some(metrics) = scrollbar_metrics(viewport_height, max_scroll, self.scroll.offset().y)
        else {
            self.scroll_drag_offset = None;
            return;
        };
        let target_top =
            (event.position.y - viewport_bounds.top() - px(SCROLLBAR_INSET_PX) - drag_offset)
                .clamp(px(0.), metrics.travel);
        let progress = target_top / metrics.travel;
        self.scroll
            .set_offset(point(px(0.), -(max_scroll * progress)));
        cx.notify();
    }

    fn finish_scrollbar_drag(&mut self, cx: &mut Context<Self>) {
        if self.scroll_drag_offset.take().is_some() {
            cx.notify();
        }
    }

    fn render_forget_modal(&self, device: &ManagedDevice, cx: &mut Context<Self>) -> AnyElement {
        let uid = device.uid.clone();
        let row = self.row.clone();
        div()
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(500.))
                    .overflow_hidden()
                    .rounded(px(theme::RADIUS_LG))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::bg_surface())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(58.))
                            .flex_none()
                            .px(px(22.))
                            .border_b_1()
                            .border_color(theme::border())
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(20.))
                                    .text_color(theme::text_primary())
                                    .child("Forget Device"),
                            )
                            .child(
                                div()
                                    .id("close-forget-device")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(28.))
                                    .rounded(px(theme::RADIUS_SM))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.forget_device_uid = None;
                                        cx.notify();
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/x.svg")
                                            .size(px(16.))
                                            .text_color(theme::text_muted()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(9.))
                            .p(px(22.))
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(17.))
                                    .text_color(theme::text_primary())
                                    .child(format!("Forget “{}”?", device.name)),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .text_color(theme::text_secondary())
                                    .child(
                                        "This removes the saved device details and exclusive-mode setting. If it reconnects, Pulse will probe it again and return it to Auto.",
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(9.))
                            .h(px(62.))
                            .flex_none()
                            .px(px(22.))
                            .border_t_1()
                            .border_color(theme::border())
                            .child(
                                components::secondary_button("cancel-forget-device", "Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.forget_device_uid = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                components::danger_button(
                                    "confirm-forget-device",
                                    "Forget Device",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if row.update(cx, |row, cx| {
                                        row.forget_managed_device(&uid, cx)
                                    }) {
                                        this.forget_device_uid = None;
                                        cx.notify();
                                    }
                                })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for DeviceManagementPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_size = window.viewport_size();
        if self.last_window_size != Some(window_size) {
            self.last_window_size = Some(window_size);
            self.scrollbar_measure_pending = true;
        }
        let row = self.row.read(cx);
        let groups = row.managed_device_groups();
        let messages = row.device_management_messages();
        let layout_signature = LayoutSignature {
            connected: groups.connected.len(),
            not_connected: groups.not_connected.len(),
            messages: messages.clone(),
        };
        if self.last_layout_signature.as_ref() != Some(&layout_signature) {
            self.last_layout_signature = Some(layout_signature);
            self.scrollbar_measure_pending = true;
        }
        if self.scrollbar_measure_pending {
            self.scrollbar_measure_pending = false;
            window.request_animation_frame();
        }
        let modal_device = self.forget_device_uid.as_deref().and_then(|uid| {
            groups
                .not_connected
                .iter()
                .find(|device| device.uid == uid)
                .cloned()
        });

        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .w_full()
            .max_w(px(720.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .w_full()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(34.))
                            .text_color(theme::text_primary())
                            .child("Devices"),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(13.))
                            .text_color(theme::text_secondary())
                            .child("Every output Pulse knows — settings stick to each device whether it’s connected or not. Exclusive mode follows the device’s Auto default until you pin it."),
                    ),
            );
        for (message, is_error) in messages {
            content = content.child(
                div()
                    .w_full()
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(if is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(if is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .child(message),
            );
        }
        if !groups.connected.is_empty() {
            content = content.child(self.render_group("CONNECTED", &groups.connected, 0, cx));
        }
        if !groups.not_connected.is_empty() {
            content = content.child(self.render_group(
                "NOT CONNECTED",
                &groups.not_connected,
                groups.connected.len(),
                cx,
            ));
        }

        let scrollbar = self.render_scrollbar(cx);
        let scroll = div()
            .id("device-management-page-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .w_full()
                    .px(px(28.))
                    .pt(px(26.))
                    .pb(px(24.))
                    .child(content),
            );

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .on_mouse_move(cx.listener(|this, event, _, cx| {
                this.update_scrollbar_drag(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_scrollbar_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_scrollbar_drag(cx)),
            )
            .child(scroll)
            .when_some(scrollbar, |page, scrollbar| page.child(scrollbar))
            .when_some(modal_device, |page, device| {
                page.child(self.render_forget_modal(&device, cx))
            })
    }
}

fn device_class(device: &ManagedDevice) -> &'static str {
    match device.capabilities {
        Some(capabilities) if capabilities.max_bits_per_channel.is_some() => "DAC",
        Some(_) => "Bluetooth",
        None => "Unknown",
    }
}

fn status_pill(label: &'static str, active: bool) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(9.))
        .py(px(5.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(if active {
            theme::accent()
        } else {
            theme::border_strong()
        })
        .bg(if active {
            theme::accent_soft()
        } else {
            theme::bg_elevated()
        })
        .when(active, |pill| {
            pill.child(div().size(px(6.)).rounded_full().bg(theme::accent()))
        })
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(11.))
                .text_color(if active {
                    theme::accent()
                } else {
                    theme::text_secondary()
                })
                .child(label),
        )
}

fn device_action_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(10.))
        .py(px(4.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border_strong())
        .cursor_pointer()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child(label)
}

fn scrollbar_metrics(
    viewport_height: Pixels,
    max_scroll: Pixels,
    offset: Pixels,
) -> Option<ScrollbarMetrics> {
    if viewport_height <= px(0.) || max_scroll <= px(0.) {
        return None;
    }
    let rail_height = viewport_height - px(SCROLLBAR_INSET_PX * 2.);
    if rail_height <= px(SCROLLBAR_MIN_THUMB_PX) {
        return None;
    }
    let content_height = viewport_height + max_scroll;
    let thumb_height = (rail_height * (viewport_height / content_height))
        .clamp(px(SCROLLBAR_MIN_THUMB_PX), rail_height);
    let travel = rail_height - thumb_height;
    if travel <= px(0.) {
        return None;
    }
    let progress = (-offset / max_scroll).clamp(0., 1.);
    Some(ScrollbarMetrics {
        thumb_height,
        thumb_top: travel * progress,
        travel,
    })
}

fn format_last_seen(last_seen_unix_seconds: Option<u64>) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_last_seen_at(last_seen_unix_seconds, now)
}

fn format_last_seen_at(last_seen_unix_seconds: Option<u64>, now: u64) -> String {
    let Some(last_seen) = last_seen_unix_seconds else {
        return "Last seen before this version of Pulse".to_string();
    };
    let elapsed = now.saturating_sub(last_seen);
    if elapsed < 60 {
        return "Last seen just now".to_string();
    }
    let relative = if elapsed < 3_600 {
        format_unit(elapsed / 60, "minute")
    } else if elapsed < 86_400 {
        format_unit(elapsed / 3_600, "hour")
    } else if elapsed < 604_800 {
        format_unit(elapsed / 86_400, "day")
    } else if elapsed < 2_592_000 {
        format_unit(elapsed / 604_800, "week")
    } else if elapsed < 31_536_000 {
        format_unit(elapsed / 2_592_000, "month")
    } else {
        format_unit(elapsed / 31_536_000, "year")
    };
    format!("Last seen {relative} ago")
}

fn format_unit(value: u64, unit: &str) -> String {
    if value == 1 {
        format!("1 {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_seen_copy_uses_readable_time_units() {
        assert_eq!(format_unit(1, "day"), "1 day");
        assert_eq!(format_unit(3, "day"), "3 days");
        assert_eq!(
            format_last_seen_at(None, 1_000),
            "Last seen before this version of Pulse"
        );
        assert_eq!(format_last_seen_at(Some(990), 1_000), "Last seen just now");
        assert_eq!(
            format_last_seen_at(Some(1_000), 1_000 + 3 * 86_400),
            "Last seen 3 days ago"
        );
    }

    #[test]
    fn scrollbar_metrics_follow_the_scroll_range() {
        assert_eq!(scrollbar_metrics(px(500.), px(0.), px(0.)), None);
        assert_eq!(scrollbar_metrics(px(40.), px(1_000.), px(0.)), None);

        let top = scrollbar_metrics(px(500.), px(1_000.), px(0.)).unwrap();
        assert_eq!(top.thumb_top, px(0.));

        let bottom = scrollbar_metrics(px(500.), px(1_000.), px(-1_000.)).unwrap();
        assert_eq!(bottom.thumb_top, bottom.travel);

        let deep = scrollbar_metrics(px(500.), px(100_000.), px(0.)).unwrap();
        assert_eq!(deep.thumb_height, px(SCROLLBAR_MIN_THUMB_PX));
    }
}
