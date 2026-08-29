use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, Pixels, Render, ScrollHandle, Size,
    StatefulInteractiveElement, Subscription, Window, div, prelude::*, px, svg,
};

use crate::{
    app_store::{AppStore, StoreRevisions, global_app_store},
    playback::{ManagedDevice, PlaybackAction, format_stored_device_capabilities},
    theme,
    ui::{self, Scrollbar},
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutSignature {
    connected: usize,
    not_connected: usize,
    messages: Vec<(String, bool)>,
}

pub(crate) struct DeviceManagementPage {
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    forget_device_uid: Option<String>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    last_window_size: Option<Size<Pixels>>,
    last_layout_signature: Option<LayoutSignature>,
    scrollbar_measure_pending: bool,
    _store_subscription: Subscription,
}

impl DeviceManagementPage {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let app_store = global_app_store(cx);
        let store_revisions = app_store.read(cx).revisions;
        let scroll = ScrollHandle::new();
        let scrollbar = cx.new(|_| {
            Scrollbar::new("device-management-scrollbar", scroll.clone())
                .thumb_id("device-management-scrollbar-thumb")
        });
        Self {
            app_store: app_store.clone(),
            store_revisions,
            forget_device_uid: None,
            scroll,
            scrollbar,
            last_window_size: None,
            last_layout_signature: None,
            scrollbar_measure_pending: true,
            _store_subscription: cx.observe(&app_store, |this, _, cx| {
                let revisions = this.app_store.read(cx).revisions;
                let reactions = revisions.reactions_since(this.store_revisions);
                this.store_revisions = revisions;
                if reactions.devices {
                    cx.notify();
                }
            }),
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
            actions = actions.child(ui::pill("Active", true));
        }
        if device.saved_default {
            actions = actions.child(ui::pill("Default", false));
        } else if device.can_set_as_default() {
            let app_store = self.app_store.clone();
            let uid = device.uid.clone();
            actions = actions.child(
                device_action_button(("set-default-device", index), "Set as default").on_click(
                    move |_, _, cx| {
                        app_store.update(cx, |store, store_cx| {
                            store.send_command(
                                PlaybackAction::SetManagedDeviceAsDefault(uid.clone()),
                                store_cx,
                            );
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

        let reset_store = self.app_store.clone();
        let reset_uid = device.uid.clone();
        let reset_default = device.default_exclusive_mode;
        let toggle_store = self.app_store.clone();
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
            .child(ui::exclusive_mode_control(
                device.automatic,
                ui::exclusive_mode_reset_link(("device-mode-reset", index))
                    .on_click(move |_, _, cx| {
                        reset_store.update(cx, |store, store_cx| {
                            store.send_command(
                                PlaybackAction::ResetDeviceExclusiveMode {
                                    device_uid: reset_uid.clone(),
                                    default: reset_default,
                                },
                                store_cx,
                            );
                        });
                    })
                    .into_any_element(),
                ui::Toggle::new(("device-mode-toggle", index), device.exclusive_mode)
                    .on_click(move |_, _, cx| {
                        toggle_store.update(cx, |store, store_cx| {
                            store.send_command(
                                PlaybackAction::ToggleDeviceExclusiveMode {
                                    device_uid: toggle_uid.clone(),
                                    default: toggle_default,
                                },
                                store_cx,
                            );
                        });
                    })
                    .into_any_element(),
            ))
            .into_any_element()
    }

    fn render_forget_modal(&self, device: &ManagedDevice, cx: &mut Context<Self>) -> AnyElement {
        let uid = device.uid.clone();
        let app_store = self.app_store.clone();
        let body = div()
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
            );
        ui::ConfirmDialog::new("forget-device-dialog", "Forget Device", body)
            .cancel_id("cancel-forget-device")
            .confirm_id("confirm-forget-device")
            .close_id("close-forget-device")
            .confirm_label("Forget Device")
            .on_cancel(cx.listener(|this, _, _, cx| {
                this.forget_device_uid = None;
                cx.notify();
            }))
            .on_confirm(cx.listener(move |this, _, _, cx| {
                if app_store.update(cx, |store, store_cx| {
                    store.send_command(PlaybackAction::ForgetManagedDevice(uid.clone()), store_cx)
                }) {
                    this.forget_device_uid = None;
                    cx.notify();
                }
            }))
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
        let groups = self.app_store.read(cx).managed_device_groups();
        let messages = self.app_store.read(cx).device_management_messages();
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
            .child(scroll)
            .child(self.scrollbar.clone())
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
}
