use gpui::{
    AnyElement, Context, Entity, ExternalPaths, FocusHandle, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, Render, ScrollHandle, Window, WindowControlArea, div, prelude::*,
    px,
};

use crate::{
    app_store::global_app_store,
    menu::{About, CheckForUpdates, FocusSearch, OpenSettings},
    playback::PlaybackAction,
    settings::SettingsSection,
    surfaces::{Destination, DeviceManagementPage, LibraryView, PlaybackRow, SearchViewModel},
    text_input::TextInput,
    theme,
    updater::Updater,
};

pub(crate) const TOP_BAR_HEIGHT: f32 = 74.0;

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
    pub(super) settings_output_toggle_press_closed: bool,
    pub(super) updater: Entity<Updater>,
    pub(super) update_check_poll_generation: u64,
    pub(super) titlebar_drag_armed: bool,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let row = cx.new(PlaybackRow::new);
        let devices = cx.new(DeviceManagementPage::new);
        let library = cx.new(LibraryView::new);
        let updater = cx.new(Updater::new);
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        cx.observe(&updater, |_, _, cx| cx.notify()).detach();
        updater.read(cx).start();
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
            settings_section: None,
            settings_output_toggle_press_closed: false,
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

    fn render_body(&self, _cx: &Context<Self>) -> AnyElement {
        if self.destination != Destination::Devices {
            return self.library.clone().into_any_element();
        }
        self.devices.clone().into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("window-drop-target")
            .flex()
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
            )
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                window.blur();
                this.open_settings(SettingsSection::General, cx);
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| {
                window.blur();
                this.updater.read(cx).check_for_updates();
            }))
            .on_action(cx.listener(|this, _: &About, window, cx| {
                window.blur();
                this.open_settings(SettingsSection::About, cx);
            }));

        if self.settings_section.is_some() {
            return root.child(self.render_settings_shell(cx));
        }

        root.on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
            this.focus_search(window, cx);
        }))
        .child(self.render_sidebar(cx))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.render_titlebar_drag_area("main-titlebar-drag", render_top_bar(), cx))
                .child(self.render_body(cx))
                .child(self.row.clone())
                .child(self.render_search(window, cx)),
        )
    }
}

fn render_top_bar() -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .h(px(TOP_BAR_HEIGHT))
        .flex_none()
        .px(px(28.))
        .border_b_1()
        .border_color(theme::border())
}
