use std::{ops::Range, path::Path, time::Duration};

use gpui::{
    AnyElement, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, ExternalPaths,
    FocusHandle, FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, ScrollHandle, SharedString,
    UTF16Selection, Window, WindowControlArea, canvas, div, img, linear_color_stop,
    linear_gradient, prelude::*, px, svg,
};

use crate::{
    components,
    library::{Album, PlaylistSummary, Track},
    library_ui::{
        LibraryView,
        view_model::{self, SearchSelection, SearchViewModel},
    },
    menu::{About, CheckForUpdates, FocusSearch, OpenSettings},
    playback_row::PlaybackRow,
    preferences,
    settings::{AboutLink, SettingsSection, SettingsViewModel},
    theme,
    update::{self, UpdateCheckKind, UpdateState},
};

pub(crate) const SIDEBAR_WIDTH: f32 = 236.0;
const SIDEBAR_TOP_PADDING: f32 = 54.0;
pub(crate) const TOP_BAR_HEIGHT: f32 = 74.0;
const SEARCH_WIDTH: f32 = 420.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
const LAUNCH_UPDATE_CHECK_DELAY: Duration = Duration::from_millis(500);

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
    library: Entity<LibraryView>,
    search_query: String,
    search: SearchViewModel,
    search_open: bool,
    search_loading: bool,
    search_revision: u64,
    search_scroll: ScrollHandle,
    search_focus: FocusHandle,
    search_selection: Range<usize>,
    search_marked_range: Option<Range<usize>>,
    settings_section: Option<SettingsSection>,
    settings_output_toggle_press_closed: bool,
    check_updates_on_launch: bool,
    update_state: UpdateState,
    update_check_revision: u64,
    titlebar_drag_armed: bool,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let row = cx.new(PlaybackRow::new);
        let library = cx.new(|cx| LibraryView::new(row.clone(), cx));
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        let check_updates_on_launch =
            preferences::load_check_updates_on_launch().unwrap_or_else(|error| {
                eprintln!("Could not load the launch update-check preference: {error}");
                true
            });
        let mut shell = Self {
            destination: Destination::Albums,
            row,
            library,
            search_query: String::new(),
            search: SearchViewModel::default(),
            search_open: false,
            search_loading: false,
            search_revision: 0,
            search_scroll: ScrollHandle::new(),
            search_focus: cx.focus_handle(),
            search_selection: 0..0,
            search_marked_range: None,
            settings_section: None,
            settings_output_toggle_press_closed: false,
            check_updates_on_launch,
            update_state: UpdateState::default(),
            update_check_revision: 0,
            titlebar_drag_armed: false,
        };
        if let Some(kind) = UpdateCheckKind::launch_if_enabled(check_updates_on_launch) {
            shell.start_update_check(kind, cx);
        }
        shell
    }

    fn open_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.search_open = false;
        self.search_marked_range = None;
        self.row.update(cx, |row, cx| row.enter_settings(cx));
        cx.notify();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.row.update(cx, |row, cx| row.leave_settings(cx));
        cx.notify();
    }

    fn select_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.row.update(cx, |row, cx| row.close_output_popover(cx));
        cx.notify();
    }

    fn toggle_check_updates_on_launch(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.check_updates_on_launch;
        if let Err(error) = preferences::save_check_updates_on_launch(enabled) {
            eprintln!("Could not save the launch update-check preference: {error}");
            return;
        }
        self.check_updates_on_launch = enabled;
        cx.notify();
    }

    fn start_update_check(&mut self, kind: UpdateCheckKind, cx: &mut Context<Self>) {
        self.update_check_revision = self.update_check_revision.wrapping_add(1);
        let revision = self.update_check_revision;
        self.update_state.begin();
        cx.notify();

        cx.spawn(async move |this, cx| {
            if kind == UpdateCheckKind::Launch {
                cx.background_executor()
                    .timer(LAUNCH_UPDATE_CHECK_DELAY)
                    .await;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    update::fetch_latest_release().map_err(|error| {
                        eprintln!("Update check failed: {error}");
                        error.to_string()
                    })
                })
                .await;
            let _ = this.update(cx, move |this, cx| {
                if this.update_check_revision != revision {
                    return;
                }
                let launch_release = if kind == UpdateCheckKind::Launch {
                    result.as_ref().ok().and_then(|release| release.clone())
                } else {
                    None
                };
                this.update_state.finish(kind, &result);
                if let Some(release) = launch_release {
                    this.row.update(cx, |row, cx| {
                        row.show_update_available(release.version, release.url, cx);
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_selection = self.search_query.len()..self.search_query.len();
        window.focus(&self.search_focus, cx);
        if !self.search_query.is_empty() {
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
            "backspace" => {
                let range = if self.search_selection.is_empty() {
                    let end = self.search_selection.end.min(self.search_query.len());
                    let start = self.search_query[..end]
                        .char_indices()
                        .next_back()
                        .map(|(index, _)| index)
                        .unwrap_or(end);
                    start..end
                } else {
                    self.search_selection.clone()
                };
                self.search_query.replace_range(range.clone(), "");
                self.search_selection = range.start..range.start;
                self.search_marked_range = None;
                self.search_query_changed(cx);
            }
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
                self.search_marked_range = None;
                window.blur();
                cx.notify();
            }
            "v" if event.keystroke.modifiers.platform => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let text = text.replace(['\r', '\n'], " ");
                    let range = self.search_selection.clone();
                    self.search_query.replace_range(range.clone(), &text);
                    let end = range.start + text.len();
                    self.search_selection = end..end;
                    self.search_marked_range = None;
                    self.search_query_changed(cx);
                }
            }
            _ => {}
        }
    }

    fn search_query_changed(&mut self, cx: &mut Context<Self>) {
        self.search_revision = self.search_revision.wrapping_add(1);
        let revision = self.search_revision;
        let query = self.search_query.trim().to_string();
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
        self.search_marked_range = None;
        window.blur();
        cx.notify();
    }

    fn search_range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_offset_to_utf8(&self.search_query, range.start)
            ..utf16_offset_to_utf8(&self.search_query, range.end)
    }

    fn search_range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.search_query[..range.start].encode_utf16().count()
            ..self.search_query[..range.end].encode_utf16().count()
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
        div()
            .w_full()
            .pt(px(12.))
            .border_t_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("open-settings")
                    .group("sidebar-settings")
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
    }

    fn render_settings_shell(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self
            .settings_section
            .expect("settings shell is only rendered for a settings section");
        let model = SettingsViewModel::new(section, self.row.read(cx).active_output_device());

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
        let exclusive_mode = self.row.read(cx).exclusive_mode();
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
            settings_card()
                .child(settings_row(
                    "Default output device",
                    "Where Pulse sends audio.",
                    output_picker,
                    true,
                ))
                .child(settings_row(
                    "Exclusive mode",
                    "Take exclusive control of the device so nothing else can resample the stream.",
                    components::toggle("exclusive-mode-toggle", exclusive_mode).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.row.update(cx, |row, cx| row.toggle_exclusive_mode(cx));
                        }),
                    ),
                    false,
                )),
        )
        .max_w(px(820.))
        .into_any_element()
    }

    fn render_update_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let (status, status_color, icon, label, accent, enabled, release_url) =
            match &self.update_state {
                UpdateState::Idle => (
                    "You're on the latest version.".to_string(),
                    theme::text_muted(),
                    "icons/refresh-cw.svg",
                    "Check for updates",
                    false,
                    true,
                    None,
                ),
                UpdateState::Checking => (
                    "Checking for updates…".to_string(),
                    theme::text_muted(),
                    "icons/loader.svg",
                    "Checking…",
                    false,
                    false,
                    None,
                ),
                UpdateState::Available(release) => (
                    format!("Pulse {} is available.", release.version),
                    theme::quality(),
                    "icons/external-link.svg",
                    "View release",
                    true,
                    true,
                    Some(release.url.clone()),
                ),
                UpdateState::Failed => (
                    "Couldn't reach GitHub. Check your connection.".to_string(),
                    theme::warning(),
                    "icons/refresh-cw.svg",
                    "Check for updates",
                    false,
                    true,
                    None,
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
            .when(!accent, |button| {
                button
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_muted())
            })
            .when(accent, |button| button.bg(theme::accent()))
            .opacity(if enabled { 1.0 } else { 0.45 })
            .child(
                svg()
                    .path(icon)
                    .size(px(16.))
                    .flex_none()
                    .text_color(if accent {
                        theme::bg_inset()
                    } else {
                        theme::text_secondary()
                    }),
            )
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .text_color(if accent {
                        theme::bg_inset()
                    } else {
                        theme::text_primary()
                    })
                    .child(label),
            );
        let action = if let Some(url) = release_url {
            action
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                .into_any_element()
        } else if enabled {
            action
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.start_update_check(UpdateCheckKind::Manual, cx);
                }))
                .into_any_element()
        } else {
            action.cursor_default().into_any_element()
        };
        let hero = settings_card().child(
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
        );

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(820.))
            .child(settings_group("VERSION", hero))
            .child(settings_group(
                "PREFERENCES",
                settings_card().child(settings_row(
                    "Check for updates on launch",
                    "Asks GitHub once per launch whether a newer version exists. Nothing else is sent.",
                    components::toggle(
                        "update-check-on-launch-toggle",
                        self.check_updates_on_launch,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_check_updates_on_launch(cx);
                    })),
                    false,
                )),
            ))
            .into_any_element()
    }

    fn render_about_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let application = settings_card().child(
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
                                .child("Native-rate music player for macOS."),
                        ),
                ),
        );

        let mut links = settings_card();
        for (index, link) in AboutLink::ALL.into_iter().enumerate() {
            links = links.child(
                settings_row(
                    link.label(),
                    link.description(),
                    svg()
                        .path("icons/external-link.svg")
                        .size(px(15.))
                        .flex_none()
                        .text_color(theme::text_muted()),
                    index + 1 < AboutLink::ALL.len(),
                )
                .id(("about-link", index))
                .cursor_pointer()
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

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        if self.destination != Destination::Devices {
            return self.library.clone().into_any_element();
        }

        let row = self.row.read(cx);
        let error = row.error().map(str::to_string);
        let hint = match &error {
            Some(error) => error.clone(),
            None if !row.has_track() => "Drop an audio file anywhere in the window".to_string(),
            None => String::new(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(px(18.))
            .pt(px(26.))
            .pr(px(28.))
            .pb(px(24.))
            .pl(px(28.))
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(26.))
                    .text_color(theme::text_primary())
                    .child(self.destination.label()),
            )
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(theme::text_muted())
                    .child("NOT BUILT YET"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(13.))
                    .text_color(if error.is_some() {
                        theme::danger()
                    } else {
                        theme::text_muted()
                    })
                    .child(hint),
            )
            .into_any_element()
    }

    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search_query.clone();
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
                                text.child(crate::components::input_caret())
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(13.))
                                    .text_color(if query.is_empty() {
                                        theme::text_muted()
                                    } else {
                                        theme::text_primary()
                                    })
                                    .child(if query.is_empty() {
                                        "Search library".to_string()
                                    } else {
                                        query.clone()
                                    }),
                            )
                            .when(!query.is_empty() && focused, |text| {
                                text.child(crate::components::input_caret())
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
                    .child(format!("No matches for “{}”", self.search_query.trim())),
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
                this.search_marked_range = None;
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
                    .child(crate::components::playing_row_glow())
                    .child(crate::components::playing_row_bar())
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
                    .child(crate::components::playing_row_glow())
                    .child(crate::components::playing_row_bar())
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
            .when_some(quality, |row, quality| {
                row.child(crate::components::quality_badge(quality))
            })
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
                    .child(crate::components::playing_row_glow())
                    .child(crate::components::playing_row_bar())
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
        let range = self.search_range_from_utf16(&range);
        adjusted_range.replace(self.search_range_to_utf16(&range));
        Some(self.search_query[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.search_range_to_utf16(&self.search_selection),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.search_marked_range
            .as_ref()
            .map(|range| self.search_range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.search_marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|range| self.search_range_from_utf16(range))
            .or_else(|| self.search_marked_range.clone())
            .unwrap_or_else(|| self.search_selection.clone());
        self.search_query.replace_range(range.clone(), text);
        let end = range.start + text.len();
        self.search_selection = end..end;
        self.search_marked_range = None;
        self.search_query_changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|range| self.search_range_from_utf16(range))
            .or_else(|| self.search_marked_range.clone())
            .unwrap_or_else(|| self.search_selection.clone());
        self.search_query.replace_range(range.clone(), new_text);
        self.search_marked_range =
            (!new_text.is_empty()).then_some(range.start..range.start + new_text.len());
        self.search_selection = new_selected_range
            .map(|selection| {
                range.start + utf16_offset_to_utf8(new_text, selection.start)
                    ..range.start + utf16_offset_to_utf8(new_text, selection.end)
            })
            .unwrap_or_else(|| {
                let end = range.start + new_text.len();
                end..end
            });
        self.search_query_changed(cx);
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
        Some(
            self.search_query[..self.search_selection.end]
                .encode_utf16()
                .count(),
        )
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
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.row.update(cx, |row, cx| row.handle_drop(paths, cx));
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.row.update(cx, |row, cx| row.update_scrub(event, cx));
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_scrub(event, cx));
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_scrub(event, cx));
                }),
            )
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                window.blur();
                this.open_settings(SettingsSection::General, cx);
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| {
                window.blur();
                this.open_settings(SettingsSection::Update, cx);
                this.start_update_check(UpdateCheckKind::Manual, cx);
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

fn settings_card() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(16.))
        .rounded(px(theme::RADIUS_LG))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_surface())
}

fn settings_row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    trailing: impl IntoElement,
    divider: bool,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .w_full()
        .py(px(13.))
        .when(divider, |row| {
            row.border_b_1().border_color(theme::border())
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(14.))
                        .text_color(theme::text_primary())
                        .child(title.into()),
                )
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(description.into()),
                ),
        )
        .child(trailing)
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
                .path("icons/pulse-mark.svg")
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
        .child(img("app-icon/v06cM3.png").size(px(32.)).flex_none())
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(18.))
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

fn utf16_offset_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    for (utf8_offset, character) in text.char_indices() {
        if utf16_offset >= offset {
            return utf8_offset;
        }
        utf16_offset += character.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn converts_utf16_offsets_for_cjk_and_surrogate_pairs() {
        let text = "王😀菲";

        assert_eq!(utf16_offset_to_utf8(text, 0), 0);
        assert_eq!(utf16_offset_to_utf8(text, 1), "王".len());
        assert_eq!(utf16_offset_to_utf8(text, 3), "王😀".len());
        assert_eq!(utf16_offset_to_utf8(text, 4), text.len());
    }
}
