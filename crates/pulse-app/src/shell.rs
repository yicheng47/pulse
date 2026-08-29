use std::{
    ops::Range,
    path::Path,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Local};
use gpui::{
    AnyElement, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, ExternalPaths,
    FocusHandle, FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, ScrollHandle, SharedString,
    UTF16Selection, Window, WindowControlArea, canvas, div, img, linear_color_stop,
    linear_gradient, prelude::*, px, svg,
};

use crate::{
    app_store::global_app_store,
    device_management::DeviceManagementPage,
    library::{Album, PlaylistSummary, Track},
    library_ui::{
        LibraryView,
        view_model::{self, SearchSelection, SearchViewModel},
    },
    menu::{About, CheckForUpdates, FocusSearch, OpenSettings},
    playback::PlaybackAction,
    playback_row::PlaybackRow,
    settings::{AboutLink, SettingsSection, SettingsViewModel},
    text_input::{self, TextInput},
    theme, ui,
    updater::Updater,
};

pub(crate) const SIDEBAR_WIDTH: f32 = 236.0;
const SIDEBAR_TOP_PADDING: f32 = 56.0;
pub(crate) const TOP_BAR_HEIGHT: f32 = 74.0;
const SEARCH_WIDTH: f32 = 420.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
const UPDATE_CHECK_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Destination {
    Albums,
    Tracks,
    Playlists,
    Storage,
    Devices,
}

impl Destination {
    fn label(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Tracks => "Tracks",
            Self::Playlists => "Playlists",
            Self::Storage => "Storage",
            Self::Devices => "Devices",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Albums => "icons/library.svg",
            Self::Tracks => "icons/music.svg",
            Self::Playlists => "icons/list-music.svg",
            Self::Storage => "icons/database.svg",
            Self::Devices => "icons/speaker.svg",
        }
    }
}

const NAV_GROUPS: &[(&str, &[Destination])] = &[
    (
        "LIBRARY",
        &[
            Destination::Albums,
            Destination::Tracks,
            Destination::Playlists,
        ],
    ),
    ("MANAGE", &[Destination::Storage]),
    ("OUTPUT", &[Destination::Devices]),
];

pub struct Shell {
    destination: Destination,
    row: Entity<PlaybackRow>,
    devices: Entity<DeviceManagementPage>,
    library: Entity<LibraryView>,
    search_input: TextInput,
    search: SearchViewModel,
    search_open: bool,
    search_loading: bool,
    search_revision: u64,
    search_scroll: ScrollHandle,
    search_focus: FocusHandle,
    settings_section: Option<SettingsSection>,
    settings_output_toggle_press_closed: bool,
    updater: Entity<Updater>,
    update_check_poll_generation: u64,
    titlebar_drag_armed: bool,
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

    fn open_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.search_open = false;
        self.search_input.unmark_text();
        self.row.update(cx, |row, cx| row.enter_settings(cx));
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.row.update(cx, |row, cx| row.leave_settings(cx));
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    fn select_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.row.update(cx, |row, cx| row.close_output_popover(cx));
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    fn toggle_check_updates_on_launch(&mut self, cx: &mut Context<Self>) {
        if !self.updater.read(cx).is_available() {
            return;
        }
        let enabled = !self.updater.read(cx).automatically_checks_for_updates();
        self.updater.update(cx, |updater, cx| {
            updater.set_automatically_checks_for_updates(enabled, cx)
        });
    }

    fn sync_update_check_polling(&mut self, cx: &mut Context<Self>) {
        self.update_check_poll_generation = self.update_check_poll_generation.wrapping_add(1);
        if self.settings_section != Some(SettingsSection::Update) {
            return;
        }

        let generation = self.update_check_poll_generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(UPDATE_CHECK_POLL_INTERVAL)
                    .await;
                let keep_polling = this
                    .update(cx, |this, cx| {
                        if this.settings_section != Some(SettingsSection::Update)
                            || this.update_check_poll_generation != generation
                        {
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.move_to_end();
        window.focus(&self.search_focus, cx);
        if !self.search_input.text().is_empty() {
            self.search_open = true;
        }
        cx.notify();
    }

    fn handle_search_input(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "down" => {
                if self.search_open {
                    self.search.move_next();
                    self.scroll_to_search_selection();
                    cx.notify();
                }
            }
            "up" => {
                if self.search_open {
                    self.search.move_previous();
                    self.scroll_to_search_selection();
                    cx.notify();
                }
            }
            "enter" => {
                if self.search_open
                    && let Some(selection) = self.search.selected()
                {
                    self.activate_search_selection(selection, window, cx);
                }
            }
            "escape" => {
                self.search_open = false;
                self.search_input.unmark_text();
                window.blur();
                cx.notify();
            }
            _ => {
                let outcome = text_input::handle_key_down(&mut self.search_input, event, cx);
                if outcome.content_changed {
                    self.search_query_changed(cx);
                } else if outcome.handled {
                    cx.notify();
                }
            }
        }
    }

    fn search_query_changed(&mut self, cx: &mut Context<Self>) {
        self.search_revision = self.search_revision.wrapping_add(1);
        let revision = self.search_revision;
        let query = self.search_input.text().trim().to_string();
        if query.is_empty() {
            self.search.clear();
            self.search_open = false;
            self.search_loading = false;
            cx.notify();
            return;
        }
        self.search.clear();
        self.search_open = true;
        self.search_loading = true;
        self.search_scroll = ScrollHandle::new();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_revision != revision {
                    return;
                }
                match this.library.read(cx).search_library(&query) {
                    Ok(results) => {
                        this.search.set_results(results);
                        this.search_loading = false;
                    }
                    Err(error) => {
                        this.search.clear();
                        this.search_loading = false;
                        this.search_open = false;
                        this.library.update(cx, |library, cx| {
                            library.show_error(error.to_string(), cx);
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scroll_to_search_selection(&self) {
        let Some(selection) = self.search.selected() else {
            return;
        };
        let child_index = match selection {
            SearchSelection::Album(index) => 1 + index,
            SearchSelection::Track(index) => 2 + self.search.results.albums.len() + index,
            SearchSelection::Playlist(index) => {
                3 + self.search.results.albums.len() + self.search.results.tracks.len() + index
            }
        };
        self.search_scroll.scroll_to_item(child_index);
    }

    fn activate_search_selection(
        &mut self,
        selection: SearchSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match selection {
            SearchSelection::Album(index) => {
                let Some(album) = self.search.results.albums.get(index).cloned() else {
                    return;
                };
                self.destination = Destination::Albums;
                self.library.update(cx, |library, cx| {
                    library.set_destination(Destination::Albums, cx);
                    library.open_search_album(album, cx);
                });
            }
            SearchSelection::Track(index) => {
                let Some(track) = self.search.results.tracks.get(index).cloned() else {
                    return;
                };
                self.library
                    .update(cx, |library, cx| library.play_search_track(track, cx));
            }
            SearchSelection::Playlist(index) => {
                let Some(playlist) = self.search.results.playlists.get(index) else {
                    return;
                };
                let playlist_id = playlist.playlist.id;
                self.destination = Destination::Playlists;
                self.library.update(cx, |library, cx| {
                    library.set_destination(Destination::Playlists, cx);
                    library.open_search_playlist(playlist_id, cx);
                });
            }
        }
        self.search_open = false;
        self.search_input.unmark_text();
        window.blur();
        cx.notify();
    }

    fn render_titlebar_drag_area(
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

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .gap(px(22.))
            .pt(px(SIDEBAR_TOP_PADDING))
            .pr(px(14.))
            .pb(px(16.))
            .pl(px(14.))
            .bg(theme::bg_surface())
            .border_r_1()
            .border_color(theme::border())
            .child(render_brand())
            .child(self.render_navigation(cx))
            .child(div().flex_1())
            .child(self.render_settings_footer(cx))
            .child(
                self.render_titlebar_drag_area(
                    "sidebar-titlebar-drag",
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(TOP_BAR_HEIGHT)),
                    cx,
                ),
            )
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut navigation = div().flex().flex_col().gap(px(20.)).w_full();

        for (header, destinations) in NAV_GROUPS {
            let mut items = div().flex().flex_col().gap(px(4.)).w_full();
            for destination in *destinations {
                items = items.child(self.render_nav_item(*destination, cx));
            }

            navigation = navigation.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .child(
                        div().flex().px(px(10.)).w_full().child(
                            div()
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(10.))
                                .text_color(theme::text_muted())
                                .child(*header),
                        ),
                    )
                    .child(items),
            );
        }

        navigation
    }

    fn render_nav_item(
        &self,
        destination: Destination,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.destination == destination;
        let hover_group = SharedString::from(format!("sidebar-nav-{}", destination.label()));

        div()
            .id(destination.label())
            .group(hover_group.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(active, |item| item.bg(theme::accent_soft()))
            .when(!active, |item| {
                item.hover(|style| style.bg(theme::accent_soft()))
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.destination = destination;
                if destination == Destination::Devices {
                    global_app_store(cx).update(cx, |store, store_cx| {
                        store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
                    });
                }
                this.library
                    .update(cx, |library, cx| library.set_destination(destination, cx));
                cx.notify();
            }))
            .child(
                svg()
                    .path(destination.icon())
                    .size(px(17.))
                    .flex_none()
                    .text_color(if active {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    })
                    .when(!active, |icon| {
                        icon.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::accent())
                        })
                    }),
            )
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .text_color(if active {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .when(!active, |label| {
                        label.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::text_primary())
                        })
                    })
                    .child(destination.label()),
            )
            .when(destination == Destination::Storage, |item| {
                item.child(div().flex_1()).child(render_storage_badge(
                    self.library.read(cx).storage_root_count(),
                ))
            })
    }

    fn render_settings_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.clone();
        let update_version = updater
            .read(cx)
            .available()
            .map(|update| update.version().to_owned());
        let update_hint = update_version.map(|version| {
            let click_updater = updater.clone();
            let tooltip = SharedString::from(format!("Pulse {version} is ready to install"));
            let trigger = div()
                .id("sidebar-update")
                .flex()
                .items_center()
                .justify_center()
                .size(px(36.))
                .flex_none()
                .rounded(px(theme::RADIUS_MD))
                .cursor_pointer()
                .hover(|style| style.bg(theme::accent_soft()))
                .on_click(move |_, _, cx| {
                    click_updater.read(cx).check_for_updates();
                })
                .child(
                    svg()
                        .path("icons/circle-arrow-down.svg")
                        .size(px(16.))
                        .flex_none()
                        .text_color(theme::accent()),
                );
            ui::Tooltip::new("sidebar-update-tooltip", tooltip, trigger)
        });

        div()
            .w_full()
            .pt(px(12.))
            .border_t_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .w_full()
                    .child(
                        div()
                            .id("open-settings")
                            .group("sidebar-settings")
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .min_w_0()
                            .flex_1()
                            .px(px(10.))
                            .py(px(9.))
                            .rounded(px(theme::RADIUS_MD))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::accent_soft()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.blur();
                                this.open_settings(SettingsSection::General, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/settings.svg")
                                    .size(px(18.))
                                    .flex_none()
                                    .text_color(theme::text_muted())
                                    .group_hover("sidebar-settings", |style| {
                                        style.text_color(theme::accent())
                                    }),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.))
                                    .text_color(theme::text_secondary())
                                    .group_hover("sidebar-settings", |style| {
                                        style.text_color(theme::text_primary())
                                    })
                                    .child("Settings"),
                            ),
                    )
                    .children(update_hint),
            )
    }

    fn render_settings_shell(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self
            .settings_section
            .expect("settings shell is only rendered for a settings section");
        let active_output = global_app_store(cx)
            .read(cx)
            .active_output_device()
            .cloned();
        let model = SettingsViewModel::new(section, active_output.as_ref());

        div()
            .flex()
            .size_full()
            .child(self.render_settings_sidebar(&model, cx))
            .child(self.render_settings_content(&model, cx))
    }

    fn render_settings_sidebar(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut navigation = div().flex().flex_col().gap(px(4.)).w_full();
        for section in SettingsSection::ALL {
            navigation = navigation.child(self.render_settings_nav_item(model, section, cx));
        }

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .gap(px(22.))
            .pt(px(SIDEBAR_TOP_PADDING))
            .pr(px(14.))
            .pb(px(16.))
            .pl(px(14.))
            .bg(theme::bg_surface())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("back-to-library")
                    .group("settings-back")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .px(px(10.))
                    .py(px(9.))
                    .rounded(px(theme::RADIUS_MD))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::accent_soft()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.blur();
                        this.close_settings(cx);
                    }))
                    .child(
                        svg()
                            .path("icons/chevron-left.svg")
                            .size(px(17.))
                            .flex_none()
                            .text_color(theme::text_muted())
                            .group_hover("settings-back", |style| {
                                style.text_color(theme::accent())
                            }),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .text_color(theme::text_secondary())
                            .group_hover("settings-back", |style| {
                                style.text_color(theme::text_primary())
                            })
                            .child("Back to library"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .child(
                        div().flex().px(px(10.)).w_full().child(
                            div()
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(10.))
                                .text_color(theme::text_muted())
                                .child("SETTINGS"),
                        ),
                    )
                    .child(navigation),
            )
            .child(div().flex_1())
            .child(self.render_titlebar_drag_area(
                "settings-sidebar-titlebar-drag",
                div().absolute().top_0().left_0().w_full().h(px(20.)),
                cx,
            ))
    }

    fn render_settings_nav_item(
        &self,
        model: &SettingsViewModel,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = model.is_selected(section);
        let hover_group = SharedString::from(format!("settings-nav-{}", section.label()));

        div()
            .id(SharedString::from(format!(
                "settings-section-{}",
                section.label()
            )))
            .group(hover_group.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(selected, |item| item.bg(theme::accent_soft()))
            .when(!selected, |item| {
                item.hover(|style| style.bg(theme::accent_soft()))
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_settings_section(section, cx);
            }))
            .child(
                svg()
                    .path(section.icon())
                    .size(px(17.))
                    .flex_none()
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    })
                    .when(!selected, |icon| {
                        icon.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::accent())
                        })
                    }),
            )
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .text_color(if selected {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .when(!selected, |label| {
                        label.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::text_primary())
                        })
                    })
                    .child(section.label()),
            )
    }

    fn render_settings_content(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content = match model.section {
            SettingsSection::General => self.render_general_settings(model, cx),
            SettingsSection::Update => self.render_update_settings(cx),
            SettingsSection::About => self.render_about_settings(cx),
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap(px(22.))
            .pt(px(30.))
            .pr(px(36.))
            .pb(px(24.))
            .pl(px(36.))
            .child(
                div()
                    .w_full()
                    .max_w(px(820.))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(28.))
                    .text_color(theme::text_primary())
                    .child(model.section.label()),
            )
            .child(content)
            .child(self.render_titlebar_drag_area(
                "settings-content-titlebar-drag",
                div().absolute().top_0().left_0().w_full().h(px(20.)),
                cx,
            ))
    }

    fn render_general_settings(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let output_picker = div()
            .id("settings-output-picker")
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .max_w(px(360.))
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left {
                    this.settings_output_toggle_press_closed =
                        this.row.read(cx).output_popover_open();
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                if std::mem::take(&mut this.settings_output_toggle_press_closed) {
                    this.row.update(cx, |row, cx| row.close_output_popover(cx));
                    return;
                }
                this.row
                    .update(cx, |row, cx| row.toggle_settings_output_popover(cx));
            }))
            .child(
                div()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .text_color(theme::text_secondary())
                    .child(model.output_device_name.clone()),
            )
            .child(
                svg()
                    .path("icons/chevron-right.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(theme::text_muted()),
            )
            .child(self.row.clone());

        settings_group(
            "PLAYBACK",
            ui::SettingsCard::new().child(ui::SettingsRow::new(
                "Default output device",
                "Where Pulse sends audio.",
                output_picker,
            )),
        )
        .max_w(px(820.))
        .into_any_element()
    }

    fn render_update_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let (updater_available, available_version, automatically_checks, last_checked) = {
            let updater = self.updater.read(cx);
            (
                updater.is_available(),
                updater
                    .available()
                    .map(|update| update.version().to_owned()),
                updater.automatically_checks_for_updates(),
                format_last_checked(updater.last_check_at()),
            )
        };
        let (status, status_color) = match available_version.as_deref() {
            Some(version) => (format!("Update available: v{version}"), theme::quality()),
            None if updater_available => (
                "You're on the latest version.".to_owned(),
                theme::text_muted(),
            ),
            None => (
                "Update controls are disabled in development builds.".to_owned(),
                theme::text_muted(),
            ),
        };
        let action = div()
            .id("check-for-updates")
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .py(px(9.))
            .flex_none()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_muted())
            .opacity(if updater_available { 1.0 } else { 0.45 })
            .when(updater_available, |button| {
                button
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.updater.read(cx).check_for_updates();
                    }))
            })
            .when(!updater_available, |button| button.cursor_default())
            .child(
                svg()
                    .path("icons/refresh-cw.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(theme::text_secondary()),
            )
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .child("Check for Updates"),
            );
        let hero = ui::SettingsCard::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .w_full()
                    .py(px(15.))
                    .child(settings_app_mark())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .font_family(theme::FONT_SANS)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(14.))
                                            .text_color(theme::text_primary())
                                            .child("Pulse"),
                                    )
                                    .child(version_chip()),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(status_color)
                                    .child(status),
                            ),
                    )
                    .child(action),
            )
            .child(div().w_full().h(px(1.)).flex_none().bg(theme::border()))
            .child(ui::SettingsRow::new(
                "Last checked",
                last_checked.description,
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(last_checked.value),
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(820.))
            .child(settings_group("VERSION", hero))
            .child(settings_group(
                "PREFERENCES",
                ui::SettingsCard::new().child(ui::SettingsRow::new(
                    "Check for updates on launch",
                    "Let Sparkle check GitHub for a newer signed release when its schedule is due.",
                    ui::Toggle::new("update-check-on-launch-toggle", automatically_checks)
                        .disabled(!updater_available)
                        .disabled_opacity(0.45)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_check_updates_on_launch(cx);
                        })),
                )),
            ))
            .into_any_element()
    }

    fn render_about_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let application = ui::SettingsCard::new().child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .w_full()
                .py(px(16.))
                .child(settings_app_mark())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .font_family(theme::FONT_DISPLAY)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(18.))
                                        .text_color(theme::text_primary())
                                        .child("Pulse"),
                                )
                                .child(version_chip()),
                        )
                        .child(
                            div()
                                .font_family(theme::FONT_SANS)
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child("Local music player for macOS."),
                        ),
                ),
        );

        let mut links = ui::SettingsCard::new();
        for (index, link) in AboutLink::ALL.into_iter().enumerate() {
            links = links.child(
                ui::SettingsRow::new(
                    link.label(),
                    link.description(),
                    svg()
                        .path("icons/external-link.svg")
                        .size(px(15.))
                        .flex_none()
                        .text_color(theme::text_muted()),
                )
                .divider(index + 1 < AboutLink::ALL.len())
                .id(("about-link", index))
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(link.url()))),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(820.))
            .child(settings_group("APPLICATION", application))
            .child(settings_group("LINKS", links))
            .into_any_element()
    }

    fn render_body(&self, _cx: &Context<Self>) -> AnyElement {
        if self.destination != Destination::Devices {
            return self.library.clone().into_any_element();
        }
        self.devices.clone().into_any_element()
    }

    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search_input.text().to_string();
        let focused = self.search_focus.is_focused(window);
        let input_entity = cx.entity();
        let mut search = div()
            .absolute()
            .occlude()
            .left(px(28.))
            .top(px(19.))
            .w(px(SEARCH_WIDTH))
            .track_focus(&self.search_focus)
            .on_key_down(cx.listener(|this, event, window, cx| {
                this.handle_search_input(event, window, cx);
            }))
            .child(
                div()
                    .id("library-search-input")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_inset())
                    .cursor_text()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus_search(window, cx);
                    }))
                    .child(
                        svg()
                            .path("icons/search.svg")
                            .size(px(16.))
                            .flex_none()
                            .text_color(theme::text_muted()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .when(query.is_empty() && focused, |text| {
                                text.child(ui::input_caret())
                            })
                            .when(query.is_empty(), |text| {
                                text.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(px(13.))
                                        .text_color(theme::text_muted())
                                        .child("Search library"),
                                )
                            })
                            .when(!query.is_empty(), |text| {
                                text.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(px(13.))
                                        .text_color(theme::text_primary())
                                        .child(text_input::render_text(
                                            &self.search_input,
                                            focused,
                                        )),
                                )
                            }),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                let focus = input_entity.read(cx).search_focus.clone();
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, input_entity.clone()),
                                    cx,
                                );
                            },
                        )
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0(),
                    ),
            );
        if self.search_open {
            search = search.child(self.render_search_popover(cx));
        }
        search.into_any_element()
    }

    fn render_search_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let no_results = !self.search_loading && self.search.result_count() == 0;
        let mut content = div().flex().flex_col().w_full();

        content = content.child(search_group_header(
            "ALBUMS",
            !self.search_loading && self.search.results.albums.is_empty(),
        ));
        for (index, album) in self.search.results.albums.iter().cloned().enumerate() {
            content = content.child(self.render_search_album(index, album, cx));
        }

        content = content.child(search_group_header(
            "TRACKS",
            !self.search_loading && self.search.results.tracks.is_empty(),
        ));
        for (index, track) in self.search.results.tracks.iter().cloned().enumerate() {
            content = content.child(self.render_search_track(index, track, cx));
        }

        content = content.child(search_group_header(
            "PLAYLISTS",
            !self.search_loading && self.search.results.playlists.is_empty(),
        ));
        for (index, playlist) in self.search.results.playlists.iter().cloned().enumerate() {
            content = content.child(self.render_search_playlist(index, playlist, cx));
        }

        if no_results {
            content = content.child(
                div()
                    .w_full()
                    .px(px(14.))
                    .py(px(10.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(format!(
                        "No matches for “{}”",
                        self.search_input.text().trim()
                    )),
            );
        }

        content = content.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .h(px(28.))
                .w_full()
                .flex_none()
                .px(px(14.))
                .border_t_1()
                .border_color(theme::border())
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(9.))
                .text_color(theme::text_muted())
                .child("↵ OPEN · ESC DISMISS"),
        );

        content
            .id("search-results-popover")
            .absolute()
            .left_0()
            .top(px(41.))
            .w_full()
            .max_h(px(540.))
            .overflow_y_scroll()
            .track_scroll(&self.search_scroll)
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                this.search_open = false;
                this.search_input.unmark_text();
                window.blur();
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_search_album(
        &self,
        index: usize,
        album: Album,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.album_index(index));
        let meta = format!(
            "{} · {} · {} tracks",
            album.artist,
            album
                .year
                .map(|year| year.to_string())
                .unwrap_or_else(|| "Unknown year".to_string()),
            album.track_count
        );
        div()
            .id(format!("search-album-{index}"))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(46.))
            .w_full()
            .px(px(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Album(index), window, cx);
            }))
            .child(search_cover(album.cover_art_path.as_deref()))
            .child(search_copy(album.title, meta))
    }

    fn render_search_track(
        &self,
        index: usize,
        track: Track,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.track_index(index));
        let quality = view_model::quality_label(track.bit_depth, track.sample_rate_hz);
        div()
            .id(format!("search-track-{index}"))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(44.))
            .w_full()
            .px(px(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Track(index), window, cx);
            }))
            .child(search_copy(
                view_model::track_title(&track),
                format!(
                    "{} · {}",
                    view_model::track_artist(&track),
                    view_model::track_album(&track)
                ),
            ))
            .child(div().flex_1())
            .when_some(quality, |row, quality| row.child(ui::Badge::new(quality)))
    }

    fn render_search_playlist(
        &self,
        index: usize,
        playlist: PlaylistSummary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.playlist_index(index));
        div()
            .id(format!("search-playlist-{index}"))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(44.))
            .w_full()
            .px(px(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Playlist(index), window, cx);
            }))
            .child(search_cover(playlist.cover_art_path.as_deref()))
            .child(search_copy(
                playlist.playlist.name,
                format!("{} track entries", playlist.track_count),
            ))
    }
}

impl EntityInputHandler for Shell {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.search_input.text_for_range(range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.search_input.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.search_input.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.unmark_text();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_input.replace_text_in_range(range, text) {
            self.search_query_changed(cx);
        } else {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .search_input
            .replace_and_mark_text_in_range(range, new_text, new_selected_range)
        {
            self.search_query_changed(cx);
        } else {
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.search_input.character_index_utf16())
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

fn settings_group(label: &'static str, card: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(9.))
        .w_full()
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(label),
        )
        .child(card)
}

struct LastCheckedCopy {
    description: String,
    value: String,
}

fn format_last_checked(checked_at: Option<SystemTime>) -> LastCheckedCopy {
    format_last_checked_at(checked_at, SystemTime::now())
}

fn format_last_checked_at(checked_at: Option<SystemTime>, now: SystemTime) -> LastCheckedCopy {
    let Some(checked_at) = checked_at else {
        return LastCheckedCopy {
            description: "Refreshes while this page is open".into(),
            value: "Never".into(),
        };
    };

    let checked_at_local = DateTime::<Local>::from(checked_at);
    let now_local = DateTime::<Local>::from(now);
    let checked_at_label = if checked_at_local.date_naive() == now_local.date_naive() {
        checked_at_local.format("Today at %-I:%M %p").to_string()
    } else {
        checked_at_local
            .format("%b %-d, %Y at %-I:%M %p")
            .to_string()
    };
    let elapsed_seconds = now
        .duration_since(checked_at)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let value = match elapsed_seconds {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", elapsed_seconds / 60),
        3600..86400 => format!("{} hr ago", elapsed_seconds / 3600),
        _ => format!("{} d ago", elapsed_seconds / 86400),
    };

    LastCheckedCopy {
        description: format!("{checked_at_label} — refreshes while this page is open"),
        value,
    }
}

fn settings_app_mark() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(32.))
        .flex_none()
        .rounded(px(7.))
        .overflow_hidden()
        .border_1()
        .border_color(theme::border())
        .bg(linear_gradient(
            180.,
            linear_color_stop(theme::bg_surface(), 0.),
            linear_color_stop(theme::bg_inset(), 1.),
        ))
        .child(
            svg()
                .path("icons/activity.svg")
                .size(px(22.))
                .text_color(theme::accent()),
        )
}

fn version_chip() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .px(px(7.))
        .py(px(3.))
        .flex_none()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child(format!("v{}", env!("CARGO_PKG_VERSION")))
}

fn render_brand() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(32.))
                .flex_none()
                .child(
                    svg()
                        .path("icons/activity.svg")
                        .size(px(24.))
                        .text_color(theme::accent()),
                ),
        )
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(22.))
                .text_color(theme::text_primary())
                .child("Pulse"),
        )
}

fn render_storage_badge(count: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(10.))
                .text_color(theme::text_muted())
                .child(count.to_string()),
        )
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

fn search_group_header(label: &'static str, empty: bool) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(27.))
        .w_full()
        .px(px(14.))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::text_muted())
        .child(if empty {
            format!("{label} — NO MATCHES")
        } else {
            label.to_string()
        })
}

fn search_cover(path: Option<&Path>) -> AnyElement {
    let content = match path {
        Some(path) => img(path.to_path_buf())
            .size_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => svg()
            .path("icons/list-music.svg")
            .size(px(14.))
            .text_color(theme::text_muted())
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .flex_none()
        .overflow_hidden()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(content)
        .into_any_element()
}

fn search_copy(title: String, meta: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(1.))
        .child(
            div()
                .w_full()
                .truncate()
                .font_family(theme::FONT_SANS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(11.))
                .text_color(theme::text_primary())
                .child(title),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .font_family(theme::FONT_SANS)
                .text_size(px(9.))
                .text_color(theme::text_muted())
                .child(meta),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_checked_copy_tracks_boundaries_and_local_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let cases = [
            (59, "Just now"),
            (60, "1 min ago"),
            (3_599, "59 min ago"),
            (3_600, "1 hr ago"),
            (86_399, "23 hr ago"),
            (86_400, "1 d ago"),
        ];
        for (elapsed, expected) in cases {
            assert_eq!(
                format_last_checked_at(Some(now - Duration::from_secs(elapsed)), now).value,
                expected
            );
        }

        assert_eq!(format_last_checked_at(None, now).value, "Never");
        assert!(
            format_last_checked_at(Some(now - Duration::from_secs(60)), now)
                .description
                .starts_with("Today at ")
        );
        assert!(
            !format_last_checked_at(Some(now - Duration::from_secs(86_400)), now)
                .description
                .starts_with("Today at ")
        );
    }

    #[test]
    fn every_destination_is_reachable_from_exactly_one_nav_group() {
        let listed: Vec<Destination> = NAV_GROUPS
            .iter()
            .flat_map(|(_, destinations)| destinations.iter().copied())
            .collect();

        assert_eq!(listed.len(), 5);
        for destination in [
            Destination::Albums,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
            Destination::Devices,
        ] {
            assert_eq!(
                listed
                    .iter()
                    .filter(|listed| **listed == destination)
                    .count(),
                1,
                "{}",
                destination.label()
            );
        }
    }

    #[test]
    fn each_destination_maps_to_a_bundled_icon() {
        use gpui::AssetSource;

        for destination in [
            Destination::Albums,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
            Destination::Devices,
        ] {
            assert!(
                crate::assets::Assets
                    .load(destination.icon())
                    .unwrap()
                    .is_some(),
                "{}",
                destination.icon()
            );
        }
    }
}
