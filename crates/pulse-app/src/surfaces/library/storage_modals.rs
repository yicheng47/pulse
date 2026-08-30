use crate::theme::rpx;

use gpui::{FontWeight, IntoElement, StatefulInteractiveElement, div, prelude::*, svg};

use super::*;
use crate::ui;

impl LibraryView {
    pub(super) fn render_add_storage_modal(
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
                .w(rpx(520.))
                .h(rpx(452.))
                .overflow_hidden()
                .rounded(rpx(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_surface())
                .child(render_add_storage_header(cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(rpx(14.))
                        .p(rpx(22.))
                        .child(render_field_label("FOLDER"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rpx(8.))
                                .w_full()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .h(rpx(36.))
                                        .flex_1()
                                        .min_w_0()
                                        .px(rpx(10.))
                                        .rounded(rpx(theme::RADIUS_SM))
                                        .border_1()
                                        .border_color(theme::border())
                                        .bg(theme::bg_inset())
                                        .child(
                                            svg()
                                                .path("icons/folder.svg")
                                                .size(rpx(15.))
                                                .flex_none()
                                                .text_color(theme::text_muted()),
                                        )
                                        .child(
                                            div()
                                                .ml(rpx(8.))
                                                .truncate()
                                                .font_family(theme::FONT_MONO)
                                                .text_size(theme::text::CAPTION)
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
                                        .h(rpx(36.))
                                        .px(rpx(11.))
                                        .rounded(rpx(theme::RADIUS_SM))
                                        .border_1()
                                        .border_color(theme::border())
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.choose_storage_folder(cx);
                                        }))
                                        .font_family(theme::FONT_DISPLAY)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(theme::text::BODY)
                                        .text_color(theme::text_secondary())
                                        .child("Choose…"),
                                ),
                        )
                        .child(render_field_label("DISPLAY NAME"))
                        .child(super::render_text_input(
                            "storage-display-name",
                            &self.text_input,
                            &self.input_focus,
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rpx(10.))
                                .w_full()
                                .min_h(rpx(56.))
                                .px(rpx(12.))
                                .rounded(rpx(theme::RADIUS_SM))
                                .border_1()
                                .border_color(theme::border())
                                .bg(theme::bg_muted())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(rpx(18.))
                                        .flex_none()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(theme::primary())
                                        .font_family(theme::FONT_MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(theme::text::CAPTION)
                                        .text_color(theme::primary())
                                        .child("i"),
                                )
                                .child(
                                    div()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(theme::text::SMALL)
                                        .line_height(rpx(16.))
                                        .text_color(theme::text_secondary())
                                        .child("Pulse indexes FLAC, ALAC, AIFF and WAV. Other files in this folder are ignored."),
                                ),
                        )
                        .child(
                            div()
                                .id("scan-storage-now")
                                .flex()
                                .items_start()
                                .gap(rpx(9.))
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
                                        .size(rpx(17.))
                                        .mt(rpx(1.))
                                        .rounded(rpx(3.))
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
                                                    .size(rpx(12.))
                                                    .text_color(theme::bg_inset()),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(rpx(2.))
                                        .child(
                                            div()
                                                .font_family(theme::FONT_SANS)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(theme::text::BODY)
                                                .text_color(theme::text_primary())
                                                .child("Scan this root now"),
                                        )
                                        .child(
                                            div()
                                                .font_family(theme::FONT_SANS)
                                                .text_size(theme::text::CAPTION)
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
                        .gap(rpx(9.))
                        .h(rpx(64.))
                        .flex_none()
                        .px(rpx(22.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(render_cancel_modal_button(cx))
                        .child(
                            div()
                                .id("confirm-add-storage")
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(rpx(34.))
                                .px(rpx(14.))
                                .rounded(rpx(theme::RADIUS_SM))
                                .gap(rpx(7.))
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
                                        .size(rpx(14.))
                                        .text_color(if can_confirm {
                                            theme::accent()
                                        } else {
                                            theme::text_muted()
                                        }),
                                )
                                .font_family(theme::FONT_DISPLAY)
                                .font_weight(FontWeight::BOLD)
                                .text_size(theme::text::BODY_LARGE)
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

    pub(super) fn render_remove_storage_modal(
        &self,
        root_id: StorageRootId,
        display_name: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_modal_scrim(
            div()
                .flex()
                .flex_col()
                .w(rpx(520.))
                .overflow_hidden()
                .rounded(rpx(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_surface())
                .child(render_modal_header("Remove Storage", cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(10.))
                        .p(rpx(22.))
                        .child(
                            div()
                                .font_family(theme::FONT_DISPLAY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(theme::text::TITLE)
                                .text_color(theme::text_primary())
                                .child(format!("Remove “{display_name}”?")),
                        )
                        .child(
                            div()
                                .font_family(theme::FONT_SANS)
                                .text_size(theme::text::BODY)
                                .line_height(rpx(19.))
                                .text_color(theme::text_secondary())
                                .child("Pulse will remove this root and its indexed tracks from the library. The music files on disk will not be changed."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(rpx(9.))
                        .h(rpx(64.))
                        .px(rpx(22.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(render_cancel_modal_button(cx))
                        .child(
                            ui::Button::new(
                                format!("confirm-remove-storage-{root_id}"),
                                "Remove",
                            )
                            .variant(ui::ButtonVariant::Danger)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_remove_storage(cx);
                            })),
                        ),
                ),
        )
    }
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
        .h(rpx(58.))
        .flex_none()
        .px(rpx(22.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(theme::text::HEADING_SMALL)
                .text_color(theme::text_primary())
                .child(title),
        )
        .child(
            ui::IconButton::new(
                format!("close-{}", title.to_ascii_lowercase().replace(' ', "-")),
                "icons/x.svg",
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.modal = None;
                cx.notify();
            })),
        )
}

fn render_add_storage_header(cx: &mut Context<LibraryView>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(rpx(76.))
        .flex_none()
        .px(rpx(22.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(2.))
                .child(
                    div()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::BOLD)
                        .text_size(theme::text::HEADING_SMALL)
                        .text_color(theme::text_primary())
                        .child("Add Storage Root"),
                )
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .text_size(theme::text::SMALL)
                        .text_color(theme::text_secondary())
                        .child("Pulse indexes audio files in this folder into your library."),
                ),
        )
        .child(
            ui::IconButton::new("close-add-storage", "icons/x.svg").on_click(cx.listener(
                |this, _, _, cx| {
                    this.modal = None;
                    cx.notify();
                },
            )),
        )
}

pub(super) fn render_field_label(label: &'static str) -> impl IntoElement {
    div()
        .mb(rpx(-10.))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(theme::text::CAPTION_XS)
        .text_color(theme::text_muted())
        .child(label)
}

pub(super) fn render_cancel_modal_button(cx: &mut Context<LibraryView>) -> impl IntoElement {
    ui::Button::new("cancel-storage-modal", "Cancel").on_click(cx.listener(|this, _, _, cx| {
        this.modal = None;
        cx.notify();
    }))
}
