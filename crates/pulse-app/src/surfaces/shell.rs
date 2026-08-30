use gpui::{
    AnyElement, Context, Entity, ExternalPaths, FocusHandle, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, Render, ScrollHandle, Window, WindowControlArea, div, prelude::*,
};

use crate::{
    app_store::{UpdaterBridge, global_app_store},
    backend::{PlaybackAction, SessionRoute, SessionState},
    settings::SettingsSection,
    surfaces::{Destination, DeviceManagementPage, LibraryView, PlaybackRow, SearchViewModel},
    text_input::TextInput,
    theme,
};

pub(crate) const TOP_BAR_HEIGHT: f32 = 74.0;

fn launch_settings_section(session: Option<&SessionState>) -> Option<SettingsSection> {
    session.and_then(|session| {
        matches!(&session.route, SessionRoute::Devices).then_some(SettingsSection::Output)
    })
}

pub struct Shell {
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
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(target_os = "macos")]
        cx.observe_window_bounds(window, |_, window, _| {
            crate::update_titlebar_toolbar_for_fullscreen(window.is_fullscreen());
        })
        .detach();

        let launch_session = global_app_store(cx).read(cx).launch_session();
        let launch_settings_section = launch_settings_section(launch_session.as_ref());
        let restore_output = launch_settings_section == Some(SettingsSection::Output);
        let row = cx.new(PlaybackRow::new);
        let devices = cx.new(DeviceManagementPage::new);
        let library = cx.new(LibraryView::new);
        let updater = cx.new(UpdaterBridge::new);
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

        let body = if self.settings_section.is_some() {
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
