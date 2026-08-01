use std::collections::BTreeSet;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, Rgba, StatefulInteractiveElement, Window, div,
    prelude::*, px, svg,
};

use super::{
    LibraryView, Modal, StorageRootView, current_time_ms,
    view_model::{RootRowState, ScanProgressView, format_bytes, format_label, root_row_state},
};
use crate::{
    library::{ScanOutcome, StorageRootId},
    theme,
};

impl LibraryView {
    pub(super) fn render_storage(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scanning = self.scan.is_some();
        let content = if self.roots.is_empty() {
            self.render_storage_empty(cx)
        } else {
            self.render_storage_workspace(cx)
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .px(px(28.))
            .pt(px(26.))
            .pb(px(24.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .w_full()
                    .min_h(px(63.))
                    .pb(px(10.))
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(34.))
                                    .text_color(theme::text_primary())
                                    .child("Storage"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .child("Manage library folders and network mounts"),
                            ),
                    )
                    .child(
                        div()
                            .id("add-storage")
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .h(px(34.))
                            .px(px(12.))
                            .rounded(px(theme::RADIUS_SM))
                            .border_1()
                            .border_color(if scanning {
                                theme::border()
                            } else {
                                theme::accent()
                            })
                            .bg(if scanning {
                                theme::bg_muted()
                            } else {
                                theme::accent_soft()
                            })
                            .when(!scanning, |button| {
                                button.cursor_pointer().on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.begin_add_storage(window, cx);
                                    },
                                ))
                            })
                            .opacity(if scanning { 0.5 } else { 1.0 })
                            .child(
                                svg()
                                    .path("icons/folder-plus.svg")
                                    .size(px(15.))
                                    .text_color(if scanning {
                                        theme::text_muted()
                                    } else {
                                        theme::accent()
                                    }),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .text_color(if scanning {
                                        theme::text_muted()
                                    } else {
                                        theme::text_primary()
                                    })
                                    .child("Add Storage"),
                            ),
                    ),
            )
            .child(self.render_storage_summary())
            .child(content)
            .into_any_element()
    }

    fn render_storage_summary(&self) -> AnyElement {
        let storage_subtitle = if self.roots.len() == 1 {
            "1 connected root".to_string()
        } else {
            format!("{} connected roots", self.roots.len())
        };
        let scan_title;
        let scan_subtitle;
        let scan_tone;
        if let Some(scan) = &self.scan {
            let progress = scan.progress.as_ref().map(ScanProgressView::from_progress);
            scan_title = progress
                .as_ref()
                .and_then(|progress| progress.percent)
                .map(|percent| format!("{percent}%"))
                .unwrap_or_else(|| "Scanning".to_string());
            scan_subtitle = progress
                .map(|progress| progress.count_line)
                .unwrap_or_else(|| "Preparing scan".to_string());
            scan_tone = theme::accent();
        } else if let Some(latest) = self
            .roots
            .iter()
            .filter_map(|root| root.latest_scan.as_ref())
            .max_by_key(|scan| scan.started_at_ms)
        {
            scan_title = latest
                .outcome
                .map(scan_outcome_label)
                .unwrap_or("In progress")
                .to_string();
            scan_subtitle = format_relative_time(latest.started_at_ms, current_time_ms());
            scan_tone = match latest.outcome {
                Some(ScanOutcome::Failed | ScanOutcome::Offline) => theme::danger(),
                Some(ScanOutcome::CompletedWithErrors) => theme::warning(),
                _ => theme::quality(),
            };
        } else {
            scan_title = "Never".to_string();
            scan_subtitle = "No scans yet".to_string();
            scan_tone = theme::text_muted();
        }

        div()
            .flex()
            .w_full()
            .h(px(92.))
            .flex_none()
            .gap(px(14.))
            .mb(px(18.))
            .child(render_summary_card(
                "LIBRARY",
                format!("{} albums", self.catalog_summary.album_count),
                format!(
                    "{} tracks · {}",
                    self.catalog_summary.track_count,
                    format_bytes(self.catalog_summary.file_size_bytes)
                ),
                theme::quality(),
            ))
            .child(render_summary_card(
                "STORAGE ROOTS",
                self.roots.len().to_string(),
                storage_subtitle,
                theme::primary(),
            ))
            .child(render_summary_card(
                "LATEST SCAN",
                scan_title,
                scan_subtitle,
                scan_tone,
            ))
            .into_any_element()
    }

    fn render_storage_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .max_w(px(410.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(64.))
                            .rounded_full()
                            .bg(theme::bg_muted())
                            .child(
                                svg()
                                    .path("icons/database.svg")
                                    .size(px(29.))
                                    .text_color(theme::primary()),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(18.))
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(23.))
                            .text_color(theme::text_primary())
                            .child("Add your music folder"),
                    )
                    .child(
                        div()
                            .mt(px(7.))
                            .text_center()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(theme::text_secondary())
                            .child("Pulse scans local folders and mounted NAS volumes without moving your files."),
                    )
                    .child(
                        div()
                            .id("empty-add-storage")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .mt(px(20.))
                            .h(px(36.))
                            .px(px(14.))
                            .rounded(px(theme::RADIUS_SM))
                            .border_1()
                            .border_color(theme::accent())
                            .bg(theme::accent_soft())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_add_storage(window, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/folder-plus.svg")
                                    .size(px(16.))
                                    .text_color(theme::accent()),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.))
                                    .text_color(theme::text_primary())
                                    .child("Add Storage"),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(14.))
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::text_muted())
                            .child("LOCAL FOLDERS AND NAS MOUNTS SUPPORTED"),
                    ),
            )
            .into_any_element()
    }

    fn render_storage_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(px(18.))
            .child(self.render_roots_table(cx))
            .child(self.render_root_detail(cx))
            .into_any_element()
    }

    fn render_roots_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().w_full();
        for root in &self.roots {
            rows = rows.child(self.render_root_row(root, cx));
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(43.))
                    .flex_none()
                    .px(px(14.))
                    .bg(theme::bg_surface_alt())
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(theme::text_muted())
                    .child(div().w(px(248.)).child("NAME"))
                    .child(div().w(px(120.)).child("STATUS"))
                    .child(div().w(px(112.)).child("ALBUMS"))
                    .child(div().w(px(112.)).child("TRACKS"))
                    .child(div().flex_1().child("LAST SCAN")),
            )
            .child(
                div()
                    .id("storage-roots-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .into_any_element()
    }

    fn render_root_row(&self, root: &StorageRootView, cx: &mut Context<Self>) -> AnyElement {
        let root_id = root.root.id;
        let selected = self.selected_root_id == Some(root_id);
        let state = self.root_state(root);
        let (status, foreground, background) = status_style(&state);
        let last_scan = root
            .latest_scan
            .as_ref()
            .map(|scan| format_relative_time(scan.started_at_ms, current_time_ms()))
            .unwrap_or_else(|| "Never".to_string());
        let failure = match &state {
            RootRowState::Failed { message } => Some(message.clone()),
            RootRowState::CompletedWithErrors { error_count } => {
                Some(format!("{error_count} file errors"))
            }
            _ => None,
        };
        let scanning_elsewhere = self
            .scan
            .as_ref()
            .is_some_and(|scan| scan.root_id != root_id);

        div()
            .id(format!("storage-root-{root_id}"))
            .flex()
            .items_center()
            .w_full()
            .h(px(if failure.is_some() { 68. } else { 58. }))
            .flex_none()
            .px(px(14.))
            .border_t_1()
            .border_color(theme::border())
            .when(selected, |row| row.bg(theme::bg_selected()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_root(root_id, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(248.))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(32.))
                            .flex_none()
                            .rounded(px(theme::RADIUS_SM))
                            .bg(theme::bg_muted())
                            .child(
                                svg()
                                    .path("icons/folder.svg")
                                    .size(px(16.))
                                    .text_color(foreground),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .ml(px(10.))
                            .min_w_0()
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .text_color(theme::text_primary())
                                    .child(root.root.display_name.clone()),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(px(9.))
                                    .text_color(theme::text_muted())
                                    .child(root.root.path.display().to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .w(px(120.))
                    .child(render_status_pill(status, foreground, background))
                    .when_some(failure, |column, failure| {
                        column.child(
                            div()
                                .mt(px(3.))
                                .max_w(px(112.))
                                .truncate()
                                .font_family(theme::FONT_SANS)
                                .text_size(px(9.))
                                .text_color(if matches!(&state, RootRowState::Failed { .. }) {
                                    theme::danger()
                                } else {
                                    theme::warning()
                                })
                                .child(failure),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(112.))
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(root.summary.album_count.to_string()),
            )
            .child(
                div()
                    .w(px(112.))
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(root.summary.track_count.to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(last_scan),
                    )
                    .when(matches!(&state, RootRowState::Failed { .. }), |cell| {
                        cell.child(div().flex_1()).child(
                            div()
                                .id(format!("retry-root-{root_id}"))
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(theme::RADIUS_SM))
                                .border_1()
                                .border_color(theme::danger())
                                .when(!scanning_elsewhere, |button| {
                                    button.cursor_pointer().on_click(cx.listener(
                                        move |this, _, _, cx| this.start_scan(root_id, cx),
                                    ))
                                })
                                .opacity(if scanning_elsewhere { 0.5 } else { 1.0 })
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(9.))
                                .text_color(theme::danger())
                                .child("RETRY"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_root_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(root) = self.selected_root() else {
            return div().into_any_element();
        };
        let root_id = root.root.id;
        let root_state = self.root_state(root);
        let root_name = root.root.display_name.clone();
        let root_path = root.root.path.display().to_string();
        let formats = self
            .tracks
            .iter()
            .filter(|track| track.storage_root_id == root_id)
            .map(|track| format_label(&track.path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" / ");
        let issues = match &root_state {
            RootRowState::CompletedWithErrors { error_count } => {
                format!("{error_count} file errors")
            }
            RootRowState::Failed { message } => message.clone(),
            _ => "None".to_string(),
        };
        let progress = self
            .scan
            .as_ref()
            .filter(|scan| scan.root_id == root_id)
            .and_then(|scan| scan.progress.as_ref())
            .map(ScanProgressView::from_progress);
        let rename = self
            .rename_draft
            .as_ref()
            .filter(|draft| draft.root_id == root_id)
            .map(|draft| draft.display_name.clone());

        div()
            .flex()
            .flex_col()
            .w(px(338.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface_alt())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(18.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(10.))
                                    .text_color(theme::text_muted())
                                    .child("STORAGE DETAIL"),
                            )
                            .child({
                                let (label, foreground, background) = status_style(&root_state);
                                render_status_pill(label, foreground, background)
                            }),
                    )
                    .child(
                        div()
                            .mt(px(17.))
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(40.))
                                    .flex_none()
                                    .rounded(px(theme::RADIUS_SM))
                                    .bg(theme::bg_muted())
                                    .child(
                                        svg()
                                            .path("icons/database.svg")
                                            .size(px(20.))
                                            .text_color(status_style(&root_state).1),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .w_full()
                                            .truncate()
                                            .font_family(theme::FONT_DISPLAY)
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(px(18.))
                                            .text_color(theme::text_primary())
                                            .child(root_name),
                                    )
                                    .child(
                                        div()
                                            .mt(px(2.))
                                            .w_full()
                                            .truncate()
                                            .font_family(theme::FONT_MONO)
                                            .text_size(px(9.))
                                            .text_color(theme::text_muted())
                                            .child(root_path),
                                    ),
                            ),
                    ),
            )
            .when_some(rename, |panel, value| {
                panel.child(self.render_rename_form(value, cx))
            })
            .when_some(progress, |panel, progress| {
                panel.child(render_scan_progress(progress))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(13.))
                    .p(px(18.))
                    .child(render_detail_line(
                        "Albums",
                        root.summary.album_count.to_string(),
                    ))
                    .child(render_detail_line(
                        "Tracks",
                        root.summary.track_count.to_string(),
                    ))
                    .child(render_detail_line(
                        "Size",
                        format_bytes(root.summary.file_size_bytes),
                    ))
                    .child(render_detail_line(
                        "Formats",
                        if formats.is_empty() {
                            "—".to_string()
                        } else {
                            formats
                        },
                    ))
                    .child(render_detail_line(
                        "Last scan",
                        root.latest_scan
                            .as_ref()
                            .map(|scan| format_relative_time(scan.started_at_ms, current_time_ms()))
                            .unwrap_or_else(|| "Never".to_string()),
                    ))
                    .child(render_detail_line("Issues", issues)),
            )
            .child(div().flex_1())
            .child(self.render_root_actions(root_id, &root_state, cx))
            .into_any_element()
    }

    fn render_rename_form(&self, value: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(18.))
            .pt(px(14.))
            .child(
                div()
                    .id("rename-storage-input")
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .w_full()
                    .px(px(10.))
                    .rounded(px(theme::RADIUS_SM))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::bg_inset())
                    .track_focus(&self.input_focus)
                    .on_key_down(cx.listener(|this, event, _, cx| {
                        this.handle_text_input(event, cx);
                    }))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_primary())
                    .child(value),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("cancel-rename-storage")
                            .px(px(9.))
                            .py(px(5.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rename_draft = None;
                                cx.notify();
                            }))
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(9.))
                            .text_color(theme::text_muted())
                            .child("CANCEL"),
                    )
                    .child(
                        div()
                            .id("save-rename-storage")
                            .px(px(9.))
                            .py(px(5.))
                            .rounded(px(theme::RADIUS_SM))
                            .bg(theme::accent_soft())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.commit_rename_storage(cx);
                            }))
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(9.))
                            .text_color(theme::accent())
                            .child("SAVE"),
                    ),
            )
            .into_any_element()
    }

    fn render_root_actions(
        &self,
        root_id: StorageRootId,
        state: &RootRowState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_scan = self
            .scan
            .as_ref()
            .is_some_and(|scan| scan.root_id == root_id);
        let scan_blocked = self.scan.is_some() && !active_scan;
        let mutations_blocked = self.scan.is_some();
        let scan_label = match state {
            RootRowState::Offline => "Reconnect",
            RootRowState::Failed { .. } => "Retry",
            _ => "Rescan",
        };

        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(18.))
            .border_t_1()
            .border_color(theme::border())
            .child(if active_scan {
                div()
                    .id("cancel-storage-scan")
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(34.))
                    .w_full()
                    .rounded(px(theme::RADIUS_SM))
                    .border_1()
                    .border_color(theme::danger())
                    .bg(theme::danger_soft())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_scan(cx)))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .text_color(theme::danger())
                    .child("Cancel Scan")
                    .into_any_element()
            } else {
                div()
                    .id("scan-storage-root")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .h(px(34.))
                    .w_full()
                    .rounded(px(theme::RADIUS_SM))
                    .border_1()
                    .border_color(if scan_blocked {
                        theme::border()
                    } else {
                        theme::accent()
                    })
                    .bg(if scan_blocked {
                        theme::bg_muted()
                    } else {
                        theme::accent_soft()
                    })
                    .when(!scan_blocked, |button| {
                        button.cursor_pointer().on_click(
                            cx.listener(move |this, _, _, cx| this.start_scan(root_id, cx)),
                        )
                    })
                    .opacity(if scan_blocked { 0.5 } else { 1.0 })
                    .child(svg().path("icons/refresh-cw.svg").size(px(14.)).text_color(
                        if scan_blocked {
                            theme::text_muted()
                        } else {
                            theme::accent()
                        },
                    ))
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(if scan_blocked {
                                theme::text_muted()
                            } else {
                                theme::text_primary()
                            })
                            .child(scan_label),
                    )
                    .into_any_element()
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("edit-storage-root")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(32.))
                            .flex_1()
                            .rounded(px(theme::RADIUS_SM))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg_muted())
                            .when(!mutations_blocked, |button| {
                                button.cursor_pointer().on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.begin_rename_storage(root_id, cx);
                                        window.focus(&this.input_focus, cx);
                                    },
                                ))
                            })
                            .opacity(if mutations_blocked { 0.5 } else { 1.0 })
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .child("Edit"),
                    )
                    .child(
                        div()
                            .id("remove-storage-root")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(32.))
                            .flex_1()
                            .rounded(px(theme::RADIUS_SM))
                            .border_1()
                            .border_color(theme::border())
                            .when(!mutations_blocked, |button| {
                                button.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.request_remove_storage(root_id, cx);
                                    },
                                ))
                            })
                            .opacity(if mutations_blocked { 0.5 } else { 1.0 })
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.))
                            .text_color(theme::danger())
                            .child("Remove"),
                    ),
            )
            .into_any_element()
    }

    fn root_state(&self, root: &StorageRootView) -> RootRowState {
        root_row_state(
            &root.root,
            root.latest_scan.as_ref(),
            self.scan
                .as_ref()
                .is_some_and(|scan| scan.root_id == root.root.id),
        )
    }

    pub(super) fn render_modal(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.modal.as_ref().expect("modal exists") {
            Modal::AddStorage(draft) => self.render_add_storage_modal(
                draft.path.as_ref().map(|path| path.display().to_string()),
                draft.display_name.clone(),
                draft.scan_now,
                cx,
            ),
            Modal::RemoveStorage {
                root_id,
                display_name,
            } => self.render_remove_storage_modal(*root_id, display_name.clone(), cx),
            Modal::PlaylistName { mode, name } => {
                self.render_playlist_name_modal(*mode, name.clone(), cx)
            }
            Modal::DeletePlaylist {
                playlist_id,
                name,
                entry_count,
            } => self.render_delete_playlist_modal(*playlist_id, name.clone(), *entry_count, cx),
        }
    }

    fn render_add_storage_modal(
        &self,
        path: Option<String>,
        display_name: String,
        scan_now: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path_selected = path.is_some();
        let can_confirm = path_selected && !display_name.trim().is_empty();
        let path = path.unwrap_or_else(|| "Choose a folder".to_string());
        render_modal_scrim(
            div()
                .flex()
                .flex_col()
                .w(px(520.))
                .h(px(452.))
                .overflow_hidden()
                .rounded(px(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_surface())
                .child(render_add_storage_header(cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(px(14.))
                        .p(px(22.))
                        .child(render_field_label("FOLDER"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .w_full()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .h(px(36.))
                                        .flex_1()
                                        .min_w_0()
                                        .px(px(10.))
                                        .rounded(px(theme::RADIUS_SM))
                                        .border_1()
                                        .border_color(theme::border())
                                        .bg(theme::bg_inset())
                                        .child(
                                            svg()
                                                .path("icons/folder.svg")
                                                .size(px(15.))
                                                .flex_none()
                                                .text_color(theme::text_muted()),
                                        )
                                        .child(
                                            div()
                                                .ml(px(8.))
                                                .truncate()
                                                .font_family(theme::FONT_MONO)
                                                .text_size(px(10.))
                                                .text_color(if path_selected {
                                                    theme::text_secondary()
                                                } else {
                                                    theme::text_muted()
                                                })
                                                .child(path),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("choose-storage-again")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .h(px(36.))
                                        .px(px(11.))
                                        .rounded(px(theme::RADIUS_SM))
                                        .border_1()
                                        .border_color(theme::border())
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.choose_storage_folder(cx);
                                        }))
                                        .font_family(theme::FONT_DISPLAY)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(12.))
                                        .text_color(theme::text_secondary())
                                        .child("Choose…"),
                                ),
                        )
                        .child(render_field_label("DISPLAY NAME"))
                        .child(
                            div()
                                .id("storage-display-name")
                                .flex()
                                .items_center()
                                .h(px(36.))
                                .w_full()
                                .px(px(10.))
                                .rounded(px(theme::RADIUS_SM))
                                .border_1()
                                .border_color(theme::accent())
                                .bg(theme::bg_inset())
                                .track_focus(&self.input_focus)
                                .on_key_down(cx.listener(|this, event, _, cx| {
                                    this.handle_text_input(event, cx);
                                }))
                                .font_family(theme::FONT_SANS)
                                .text_size(px(12.))
                                .text_color(theme::text_primary())
                                .child(display_name),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .w_full()
                                .min_h(px(56.))
                                .px(px(12.))
                                .rounded(px(theme::RADIUS_SM))
                                .border_1()
                                .border_color(theme::border())
                                .bg(theme::bg_muted())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(18.))
                                        .flex_none()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(theme::primary())
                                        .font_family(theme::FONT_MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(10.))
                                        .text_color(theme::primary())
                                        .child("i"),
                                )
                                .child(
                                    div()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(px(11.))
                                        .line_height(px(16.))
                                        .text_color(theme::text_secondary())
                                        .child("Pulse indexes FLAC, ALAC, AIFF and WAV. Other files in this folder are ignored."),
                                ),
                        )
                        .child(
                            div()
                                .id("scan-storage-now")
                                .flex()
                                .items_start()
                                .gap(px(9.))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(Modal::AddStorage(draft)) = &mut this.modal {
                                        draft.scan_now = !draft.scan_now;
                                    }
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(17.))
                                        .mt(px(1.))
                                        .rounded(px(3.))
                                        .border_1()
                                        .border_color(if scan_now {
                                            theme::accent()
                                        } else {
                                            theme::border_strong()
                                        })
                                        .bg(if scan_now {
                                            theme::accent()
                                        } else {
                                            theme::bg_inset()
                                        })
                                        .when(scan_now, |checkbox| {
                                            checkbox.child(
                                                svg()
                                                    .path("icons/check.svg")
                                                    .size(px(12.))
                                                    .text_color(theme::bg_inset()),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .child(
                                            div()
                                                .font_family(theme::FONT_SANS)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(12.))
                                                .text_color(theme::text_primary())
                                                .child("Scan this root now"),
                                        )
                                        .child(
                                            div()
                                                .font_family(theme::FONT_SANS)
                                                .text_size(px(10.))
                                                .text_color(theme::text_muted())
                                                .child("Large network folders can take several minutes."),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(9.))
                        .h(px(64.))
                        .flex_none()
                        .px(px(22.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(render_cancel_modal_button(cx))
                        .child(
                            div()
                                .id("confirm-add-storage")
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(34.))
                                .px(px(14.))
                                .rounded(px(theme::RADIUS_SM))
                                .gap(px(7.))
                                .bg(if can_confirm {
                                    theme::accent_soft()
                                } else {
                                    theme::bg_muted()
                                })
                                .border_1()
                                .border_color(if can_confirm {
                                    theme::accent()
                                } else {
                                    theme::border()
                                })
                                .opacity(if can_confirm { 1.0 } else { 0.5 })
                                .when(can_confirm, |button| {
                                    button.cursor_pointer().on_click(cx.listener(
                                        |this, _, _, cx| this.confirm_add_storage(cx),
                                    ))
                                })
                                .child(
                                    svg()
                                        .path("icons/plus.svg")
                                        .size(px(14.))
                                        .text_color(if can_confirm {
                                            theme::accent()
                                        } else {
                                            theme::text_muted()
                                        }),
                                )
                                .font_family(theme::FONT_DISPLAY)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(13.))
                                .text_color(if can_confirm {
                                    theme::text_primary()
                                } else {
                                    theme::text_muted()
                                })
                                .child("Add Storage"),
                        ),
                ),
        )
    }

    fn render_remove_storage_modal(
        &self,
        root_id: StorageRootId,
        display_name: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_modal_scrim(
            div()
                .flex()
                .flex_col()
                .w(px(520.))
                .overflow_hidden()
                .rounded(px(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_surface())
                .child(render_modal_header("Remove Storage", cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .p(px(22.))
                        .child(
                            div()
                                .font_family(theme::FONT_DISPLAY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(17.))
                                .text_color(theme::text_primary())
                                .child(format!("Remove “{display_name}”?")),
                        )
                        .child(
                            div()
                                .font_family(theme::FONT_SANS)
                                .text_size(px(12.))
                                .line_height(px(19.))
                                .text_color(theme::text_secondary())
                                .child("Pulse will remove this root and its indexed tracks from the library. The music files on disk will not be changed."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(9.))
                        .h(px(64.))
                        .px(px(22.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(render_cancel_modal_button(cx))
                        .child(
                            div()
                                .id(format!("confirm-remove-storage-{root_id}"))
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(34.))
                                .px(px(14.))
                                .rounded(px(theme::RADIUS_SM))
                                .bg(theme::danger())
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_remove_storage(cx);
                                }))
                                .font_family(theme::FONT_DISPLAY)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(13.))
                                .text_color(theme::bg_inset())
                                .child("Remove"),
                        ),
                ),
        )
    }
}

fn render_summary_card(
    label: &'static str,
    title: String,
    subtitle: String,
    tone: Rgba,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .justify_center()
        .px(px(16.))
        .rounded(px(theme::RADIUS_MD))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_surface())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(9.))
                .text_color(theme::text_muted())
                .child(label),
        )
        .child(
            div()
                .mt(px(5.))
                .truncate()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(20.))
                .text_color(tone)
                .child(title),
        )
        .child(
            div()
                .mt(px(2.))
                .truncate()
                .font_family(theme::FONT_SANS)
                .text_size(px(10.))
                .text_color(theme::text_muted())
                .child(subtitle),
        )
}

fn render_status_pill(label: &'static str, foreground: Rgba, background: Rgba) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(20.))
        .px(px(7.))
        .rounded(px(theme::RADIUS_SM))
        .bg(background)
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(foreground)
        .child(label)
}

fn status_style(state: &RootRowState) -> (&'static str, Rgba, Rgba) {
    match state {
        RootRowState::Online => ("ONLINE", theme::quality(), theme::quality_soft()),
        RootRowState::Offline => ("OFFLINE", theme::primary(), theme::primary_soft()),
        RootRowState::Scanning => ("SCANNING", theme::accent(), theme::accent_soft()),
        RootRowState::Failed { .. } => ("FAILED", theme::danger(), theme::danger_soft()),
        RootRowState::CompletedWithErrors { .. } => {
            ("ISSUES", theme::warning(), theme::danger_soft())
        }
    }
}

fn render_detail_line(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(12.))
        .w_full()
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(label),
        )
        .child(
            div()
                .max_w(px(205.))
                .text_right()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(10.))
                .text_color(if label == "Issues" && value != "None" {
                    theme::warning()
                } else {
                    theme::text_secondary()
                })
                .child(value),
        )
}

fn render_scan_progress(progress: ScanProgressView) -> impl IntoElement {
    let percent = progress.percent.unwrap_or_default();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .mx(px(18.))
        .mt(px(16.))
        .p(px(12.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(
            div()
                .flex()
                .justify_between()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(10.))
                .text_color(theme::text_secondary())
                .child("SCANNING")
                .child(
                    progress
                        .percent
                        .map(|percent| format!("{percent}%"))
                        .unwrap_or_else(|| "DISCOVERING".to_string()),
                ),
        )
        .child(
            div()
                .relative()
                .w_full()
                .h(px(4.))
                .rounded(px(2.))
                .bg(theme::bg_inset())
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h(px(4.))
                        .w(px(2.78 * percent as f32))
                        .rounded(px(2.))
                        .bg(theme::accent()),
                ),
        )
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(10.))
                .text_color(theme::text_muted())
                .child(progress.count_line),
        )
}

pub(super) fn render_modal_scrim(modal: impl IntoElement) -> AnyElement {
    div()
        .absolute()
        .left_0()
        .top_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::scrim())
        .child(modal)
        .into_any_element()
}

pub(super) fn render_modal_header(
    title: &'static str,
    cx: &mut Context<LibraryView>,
) -> impl IntoElement {
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
                .child(title),
        )
        .child(
            div()
                .id(format!(
                    "close-{}",
                    title.to_ascii_lowercase().replace(' ', "-")
                ))
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(theme::RADIUS_SM))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.modal = None;
                    cx.notify();
                }))
                .child(
                    svg()
                        .path("icons/x.svg")
                        .size(px(16.))
                        .text_color(theme::text_muted()),
                ),
        )
}

fn render_add_storage_header(cx: &mut Context<LibraryView>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(76.))
        .flex_none()
        .px(px(22.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(20.))
                        .text_color(theme::text_primary())
                        .child("Add Storage Root"),
                )
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .text_size(px(11.))
                        .text_color(theme::text_secondary())
                        .child("Pulse indexes audio files in this folder into your library."),
                ),
        )
        .child(
            div()
                .id("close-add-storage")
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(theme::RADIUS_SM))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.modal = None;
                    cx.notify();
                }))
                .child(
                    svg()
                        .path("icons/x.svg")
                        .size(px(16.))
                        .text_color(theme::text_muted()),
                ),
        )
}

pub(super) fn render_field_label(label: &'static str) -> impl IntoElement {
    div()
        .mb(px(-10.))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::text_muted())
        .child(label)
}

pub(super) fn render_cancel_modal_button(cx: &mut Context<LibraryView>) -> impl IntoElement {
    div()
        .id("cancel-storage-modal")
        .flex()
        .items_center()
        .justify_center()
        .h(px(34.))
        .px(px(13.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.modal = None;
            cx.notify();
        }))
        .font_family(theme::FONT_DISPLAY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child("Cancel")
}

fn scan_outcome_label(outcome: ScanOutcome) -> &'static str {
    match outcome {
        ScanOutcome::Completed => "Complete",
        ScanOutcome::CompletedWithErrors => "Issues",
        ScanOutcome::Offline => "Offline",
        ScanOutcome::Failed => "Failed",
    }
}

fn format_relative_time(timestamp_ms: i64, now_ms: i64) -> String {
    let age_ms = now_ms.saturating_sub(timestamp_ms);
    let minutes = age_ms / 60_000;
    if minutes < 1 {
        "Just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 24 * 60 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / (24 * 60))
    }
}
