use crate::theme::rpx;

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, Pixels, Render, ScrollHandle, Size,
    StatefulInteractiveElement, Subscription, Window, div, prelude::*, svg,
};

use crate::{
    app_store::{AppStore, StoreRevisions, global_app_store},
    backend::{
        ManagedDevice, PlaybackAction, StoredDeviceTransport, StoredOutputMode,
        format_stored_device_capabilities,
    },
    surfaces::devices_logic::{device_class, format_last_seen},
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
        let mut cards = div().flex().flex_col().gap(rpx(10.)).w_full();
        for (index, device) in devices.iter().enumerate() {
            cards = cards.child(self.render_device_card(device, index_offset + index, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(rpx(10.))
            .w_full()
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme::text::CAPTION)
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
        let metadata = if device.hardware_volume_available {
            format!("{class} · {capability} · Device volume")
        } else {
            format!("{class} · {capability}")
        };
        let status_color = if device.connected {
            theme::quality()
        } else {
            theme::text_muted()
        };
        let icon = if device.capabilities.is_some_and(|capabilities| {
            matches!(
                capabilities.transport,
                Some(StoredDeviceTransport::Bluetooth | StoredDeviceTransport::BluetoothLe)
            )
        }) {
            "icons/bluetooth.svg"
        } else {
            "icons/speaker.svg"
        };

        let mut actions = div().flex().items_center().gap(rpx(8.)).flex_none();
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
            .gap(rpx(3.))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme::text::TITLE)
                    .text_color(theme::text_primary())
                    .child(device.name.clone()),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme::text::SMALL)
                    .text_color(status_color)
                    .child(metadata),
            );
        if !device.connected {
            copy = copy.child(
                div()
                    .w_full()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::SMALL)
                    .text_color(theme::text_muted())
                    .child(format_last_seen(device.last_seen_unix_seconds)),
            );
        }

        let reset_store = self.app_store.clone();
        let reset_uid = device.uid.clone();
        let shared_store = self.app_store.clone();
        let shared_uid = device.uid.clone();
        let exclusive_store = self.app_store.clone();
        let exclusive_uid = device.uid.clone();
        let bit_perfect_store = self.app_store.clone();
        let bit_perfect_uid = device.uid.clone();
        let shared = ui::output_mode_segment(
            ("device-mode-shared", index),
            "Shared",
            device.output_mode == StoredOutputMode::Shared,
            false,
            false,
        )
        .on_click(move |_, _, cx| {
            shared_store.update(cx, |store, store_cx| {
                store.send_command(
                    PlaybackAction::SetDeviceOutputMode {
                        device_uid: shared_uid.clone(),
                        mode: StoredOutputMode::Shared,
                    },
                    store_cx,
                );
            });
        })
        .into_any_element();
        let exclusive = ui::output_mode_segment(
            ("device-mode-exclusive", index),
            "Exclusive",
            device.output_mode == StoredOutputMode::Exclusive,
            false,
            false,
        )
        .on_click(move |_, _, cx| {
            exclusive_store.update(cx, |store, store_cx| {
                store.send_command(
                    PlaybackAction::SetDeviceOutputMode {
                        device_uid: exclusive_uid.clone(),
                        mode: StoredOutputMode::Exclusive,
                    },
                    store_cx,
                );
            });
        })
        .into_any_element();
        let bit_perfect = ui::output_mode_segment(
            ("device-mode-bit-perfect", index),
            "Bit-perfect",
            device.output_mode == StoredOutputMode::BitPerfect,
            true,
            !device.bit_perfect_available,
        );
        let bit_perfect = if device.bit_perfect_available {
            bit_perfect
                .on_click(move |_, _, cx| {
                    bit_perfect_store.update(cx, |store, store_cx| {
                        store.send_command(
                            PlaybackAction::SetDeviceOutputMode {
                                device_uid: bit_perfect_uid.clone(),
                                mode: StoredOutputMode::BitPerfect,
                            },
                            store_cx,
                        );
                    });
                })
                .into_any_element()
        } else {
            bit_perfect.into_any_element()
        };
        div()
            .id(("managed-device", index))
            .flex()
            .flex_col()
            .gap(rpx(10.))
            .w_full()
            .p(rpx(14.))
            .rounded(rpx(theme::RADIUS_MD))
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
                    .gap(rpx(12.))
                    .w_full()
                    .child(svg().path(icon).size(rpx(22.)).flex_none().text_color(
                        if device.connected {
                            theme::accent()
                        } else {
                            theme::text_muted()
                        },
                    ))
                    .child(copy)
                    .child(actions),
            )
            .child(
                div()
                    .w_full()
                    .h(gpui::px(1.)) // physical
                    .bg(theme::border()),
            )
            .child(ui::output_mode_control(
                "Output mode",
                device.automatic,
                device.bit_perfect_available,
                ui::output_mode_reset_link(("device-mode-reset", index))
                    .on_click(move |_, _, cx| {
                        reset_store.update(cx, |store, store_cx| {
                            store.send_command(
                                PlaybackAction::ResetDeviceOutputMode(reset_uid.clone()),
                                store_cx,
                            );
                        });
                    })
                    .into_any_element(),
                ui::output_mode_segments(shared, exclusive, bit_perfect).into_any_element(),
            ))
            .into_any_element()
    }

    fn render_forget_modal(&self, device: &ManagedDevice, cx: &mut Context<Self>) -> AnyElement {
        let uid = device.uid.clone();
        let app_store = self.app_store.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap(rpx(9.))
            .p(rpx(22.))
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::text::TITLE)
                    .text_color(theme::text_primary())
                    .child(format!("Forget “{}”?", device.name)),
            )
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .line_height(rpx(18.))
                    .text_color(theme::text_secondary())
                    .child(
                        "This removes the saved device details and output-mode setting. If it reconnects, Pulse will probe it again and return it to Auto.",
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
            .gap(rpx(18.))
            .w_full()
            .max_w(rpx(720.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(4.))
                    .w_full()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::DISPLAY)
                            .text_color(theme::text_primary())
                            .child("Devices"),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_SANS)
                            .text_size(theme::text::BODY_LARGE)
                            .text_color(theme::text_secondary())
                            .child("Every output Pulse knows — settings stick to each device whether it’s connected or not. Output mode follows the device’s Auto default until you pin it."),
                    ),
            );
        for (message, is_error) in messages {
            content = content.child(
                div()
                    .w_full()
                    .px(rpx(12.))
                    .py(rpx(9.))
                    .rounded(rpx(theme::RADIUS_MD))
                    .border_1()
                    .border_color(if is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
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
                    .px(rpx(28.))
                    .pt(rpx(26.))
                    .pb(rpx(24.))
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

fn device_action_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(rpx(10.))
        .py(rpx(4.))
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border_strong())
        .cursor_pointer()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text::SMALL)
        .text_color(theme::text_secondary())
        .child(label)
}
