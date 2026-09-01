use gpui::{
    AnyElement, Context, Entity, ExternalPaths, FocusHandle, FontWeight, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, Render, ScrollHandle, Subscription, Window, WindowControlArea,
    div, prelude::*, svg,
};

use crate::{
    app_store::{StoreRevisions, UpdaterBridge, global_app_store},
    backend::{PlaybackAction, SessionRoute, SessionState},
    settings::SettingsSection,
    surfaces::{Destination, DeviceManagementPage, LibraryView, PlaybackRow, SearchViewModel},
    text_input::TextInput,
    theme::{self, rpx},
    toast::{ToastEntry, ToastKind},
};

pub(crate) const TOP_BAR_HEIGHT: f32 = 74.0;

fn launch_settings_section(session: Option<&SessionState>) -> Option<SettingsSection> {
    session.and_then(|session| {
        matches!(&session.route, SessionRoute::Devices).then_some(SettingsSection::Output)
    })
}

pub struct Shell {
    pub(super) app_store: Entity<crate::app_store::AppStore>,
    pub(super) store_revisions: StoreRevisions,
    pub(super) destination: Destination,
    pub(super) row: Entity<PlaybackRow>,
    pub(super) devices: Entity<DeviceManagementPage>,
    pub(super) library: Entity<LibraryView>,
    pub(super) search_input: TextInput,
    pub(super) search: SearchViewModel,
    pub(super) search_open: bool,
    pub(super) search_loading: bool,
    pub(super) search_revision: u64,
    pub(super) search_scroll: ScrollHandle,
    pub(super) search_focus: FocusHandle,
    pub(super) settings_section: Option<SettingsSection>,
    pub(super) updater: Entity<UpdaterBridge>,
    pub(super) update_check_poll_generation: u64,
    pub(super) titlebar_drag_armed: bool,
    pub(super) _store_subscription: Subscription,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(target_os = "macos")]
        cx.observe_window_bounds(window, |_, window, _| {
            crate::update_titlebar_toolbar_for_fullscreen(window.is_fullscreen());
        })
        .detach();

        let app_store = global_app_store(cx);
        let store_revisions = app_store.read(cx).revisions;
        let launch_session = app_store.read(cx).launch_session();
        let launch_settings_section = launch_settings_section(launch_session.as_ref());
        let restore_output = launch_settings_section == Some(SettingsSection::Output);
        let row = cx.new(PlaybackRow::new);
        let devices = cx.new(DeviceManagementPage::new);
        let library = cx.new(LibraryView::new);
        let updater = cx.new(UpdaterBridge::new);
        let store_subscription = cx.observe(&app_store, |this, _, cx| {
            let revisions = this.app_store.read(cx).revisions;
            let reactions = revisions.reactions_since(this.store_revisions);
            this.store_revisions = revisions;
            if reactions.toasts {
                cx.notify();
            }
        });
        cx.observe(&library, |this, library, cx| {
            this.destination = library.read(cx).destination();
            cx.notify();
        })
        .detach();
        cx.observe(&updater, |_, _, cx| cx.notify()).detach();
        updater.read(cx).start();
        if restore_output {
            row.update(cx, |row, cx| row.enter_settings(cx));
            global_app_store(cx).update(cx, |store, store_cx| {
                store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
            });
        }
        Self {
            app_store,
            store_revisions,
            destination: Destination::Albums,
            row,
            devices,
            library,
            search_input: TextInput::default(),
            search: SearchViewModel::default(),
            search_open: false,
            search_loading: false,
            search_revision: 0,
            search_scroll: ScrollHandle::new(),
            search_focus: cx.focus_handle(),
            settings_section: launch_settings_section,
            updater,
            update_check_poll_generation: 0,
            titlebar_drag_armed: false,
            _store_subscription: store_subscription,
        }
    }

    pub(super) fn render_titlebar_drag_area(
        &self,
        id: &'static str,
        area: gpui::Div,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        area.id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.titlebar_drag_armed = false;
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = false;
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = true;
                }),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag_armed {
                    this.titlebar_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
    }

    fn render_body(&self) -> AnyElement {
        self.library.clone().into_any_element()
    }

    fn render_toasts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let toasts = self.app_store.read(cx).toasts().to_vec();
        div()
            .absolute()
            .right(rpx(20.))
            .bottom(rpx(16.))
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .children(toasts.into_iter().map(|entry| self.render_toast(entry, cx)))
    }

    fn render_toast(&self, entry: ToastEntry, cx: &mut Context<Self>) -> AnyElement {
        let ToastEntry { id, toast, .. } = entry;
        let (color, icon) = match toast.kind {
            ToastKind::Error => (theme::danger(), "icons/circle-alert.svg"),
            ToastKind::Warning => (theme::warning(), "icons/triangle-alert.svg"),
        };
        let action = toast.action.clone();
        div()
            .id(format!("toast-{id:?}"))
            .flex()
            .items_start()
            .gap(rpx(12.))
            .w(rpx(400.))
            .px(rpx(14.))
            .py(rpx(12.))
            .rounded(rpx(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_elevated())
            .occlude()
            .on_hover(cx.listener(move |this, &hovered, _, cx| {
                this.app_store.update(cx, |store, store_cx| {
                    store.set_toast_hovered(id, hovered, store_cx);
                });
            }))
            .child(
                svg()
                    .path(icon)
                    .size(rpx(18.))
                    .flex_none()
                    .text_color(color),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rpx(4.))
                    .child(
                        div()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text::BODY_LARGE)
                            .text_color(theme::text_primary())
                            .child(toast.title),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_SANS)
                            .text_size(theme::text::BODY)
                            .text_color(theme::text_secondary())
                            .child(toast.body),
                    )
                    .children(action.map(|action| {
                        div()
                            .id(format!("toast-action-{id:?}"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .self_start()
                            .gap(rpx(6.))
                            .h(rpx(30.))
                            .mt(rpx(6.))
                            .px(rpx(11.))
                            .rounded(rpx(theme::RADIUS_MD))
                            .border_1()
                            .border_color(theme::accent())
                            .bg(theme::accent_soft())
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::bg_selected()))
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::BODY)
                            .text_color(theme::text_primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.app_store.update(cx, |store, store_cx| {
                                    store.activate_toast_action(id, store_cx);
                                });
                            }))
                            .child(
                                svg()
                                    .path("icons/lock.svg")
                                    .size(rpx(13.))
                                    .text_color(theme::accent()),
                            )
                            .child(action.label())
                    })),
            )
            .child(
                div()
                    .id(format!("toast-dismiss-{id:?}"))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(20.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.app_store.update(cx, |store, store_cx| {
                            store.dismiss_toast(id, store_cx);
                        });
                    }))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(rpx(13.))
                            .text_color(theme::text_muted()),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("window-drop-target")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_page())
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(theme::accent_soft()))
            .on_drop(cx.listener(|_, paths: &ExternalPaths, _, cx| {
                global_app_store(cx).update(cx, |store, store_cx| {
                    store.send_command(
                        PlaybackAction::PlayDroppedPaths(paths.paths().to_vec()),
                        store_cx,
                    );
                });
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.row.update(cx, |row, cx| row.update_drag(event, cx));
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_drag(event, cx));
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_drag(event, cx));
                }),
            );

        let surface = if self.settings_section.is_some() {
            self.render_settings_shell(cx).into_any_element()
        } else {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(self.render_sidebar(cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.render_body()),
                )
                .into_any_element()
        };
        let body = div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(surface)
            .child(self.render_toasts(cx));

        root.child(self.render_header(window, cx))
            .child(body)
            .child(self.row.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_devices_session_opens_output_settings() {
        let mut session = SessionState::default();
        assert_eq!(launch_settings_section(Some(&session)), None);

        session.route = SessionRoute::Devices;
        assert_eq!(
            launch_settings_section(Some(&session)),
            Some(SettingsSection::Output)
        );
    }
}
