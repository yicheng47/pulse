use crate::theme::rpx;

use gpui::{
    AnyElement, Context, ElementInputHandler, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    StatefulInteractiveElement, canvas, deferred, div, prelude::*, svg,
};

use super::{LibraryView, selected_genre};
use crate::{surfaces::Destination, theme};

impl LibraryView {
    pub(super) fn render_genre_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_genre = self.active_genre_filter().map(str::to_string);
        let active = active_genre.is_some();
        let label = active_genre.unwrap_or_else(|| "Genre".to_string());
        let trigger_id = match self.destination {
            Destination::Albums => "album-genre-filter",
            Destination::Tracks => "track-genre-filter",
            _ => "genre-filter",
        };
        let mut trigger = div()
            .id(trigger_id)
            .relative()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(29.))
            .flex_none()
            .px(rpx(10.))
            .rounded(rpx(theme::RADIUS_SM))
            .bg(if active {
                theme::accent_soft()
            } else {
                theme::bg_muted()
            })
            .when(active, |trigger| {
                trigger.border_1().border_color(theme::accent())
            })
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.genre_hint_press_closed_popover = this.genre_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.genre_hint_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.artist_popover_open = false;
                this.genre_popover_open = true;
                this.text_input.reset("");
                window.focus(&this.input_focus, cx);
                cx.notify();
            }))
            .child(
                div()
                    .max_w(rpx(180.))
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(if active {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    })
                    .child(label),
            )
            .child(
                svg()
                    .path("icons/chevron-down.svg")
                    .size(rpx(12.))
                    .text_color(if active {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    }),
            );
        if self.genre_popover_open {
            trigger = trigger.child(deferred(self.render_genre_popover(cx)).with_priority(1));
        }
        trigger.into_any_element()
    }

    fn render_genre_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered = self.filtered_genres();
        let mut rows = div().flex().flex_col().w_full();
        if filtered.is_empty() {
            rows = rows.child(
                div()
                    .w_full()
                    .px(rpx(12.))
                    .py(rpx(12.))
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(theme::text_muted())
                    .child("No matching genres"),
            );
        } else {
            for (index, (genre, album_count)) in filtered.into_iter().enumerate() {
                let selected = self
                    .active_genre_filter()
                    .is_some_and(|active| active.eq_ignore_ascii_case(&genre));
                let chosen = genre.clone();
                rows = rows.child(
                    div()
                        .id(("genre-filter-option", index))
                        .flex()
                        .items_center()
                        .gap(rpx(10.))
                        .w_full()
                        .px(rpx(12.))
                        .py(rpx(8.))
                        .when(selected, |row| row.bg(theme::accent_soft()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_genre_filter(Some(chosen.clone()), cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme::FONT_SANS)
                                .text_size(theme::text::BODY_LARGE)
                                .text_color(if selected {
                                    theme::accent()
                                } else {
                                    theme::text_primary()
                                })
                                .child(genre),
                        )
                        .child(
                            div()
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(theme::text::CAPTION)
                                .text_color(theme::text_muted())
                                .child(if album_count == 1 {
                                    "1 album".to_string()
                                } else {
                                    format!("{album_count} albums")
                                }),
                        ),
                );
            }
        }

        let active = self.active_genre_filter().is_some();
        div()
            .id("genre-filter-popover")
            .absolute()
            .left_0()
            .top(rpx(37.))
            .flex()
            .flex_col()
            .w(rpx(240.))
            .max_h(rpx(420.))
            .py(rpx(6.))
            .rounded(rpx(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.genre_popover_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .w_full()
                    .px(rpx(10.))
                    .py(rpx(6.))
                    .child(self.render_genre_search_input(cx)),
            )
            .child(
                div()
                    .id("genre-filter-options-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .when(active, |popover| {
                popover
                    .child(
                        div()
                            .w_full()
                            .h(gpui::px(1.)) // physical
                            .my(rpx(4.))
                            .bg(theme::border()),
                    )
                    .child(
                        div()
                            .id("genre-filter-clear")
                            .flex()
                            .items_center()
                            .gap(rpx(10.))
                            .w_full()
                            .px(rpx(12.))
                            .py(rpx(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.choose_genre_filter(None, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/x.svg")
                                    .size(rpx(14.))
                                    .text_color(theme::text_secondary()),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(theme::text::BODY_LARGE)
                                    .text_color(theme::text_secondary())
                                    .child("Clear genre"),
                            ),
                    )
            })
    }

    fn render_genre_search_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.text_input.text().is_empty();
        let input_entity = cx.entity();
        div()
            .id("genre-search-input")
            .relative()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .cursor_text()
            .h(rpx(36.))
            .w_full()
            .px(rpx(10.))
            .rounded(rpx(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::accent())
            .bg(theme::bg_inset())
            .track_focus(&self.input_focus)
            .on_key_down(cx.listener(|this, event, _, cx| {
                this.handle_text_input(event, cx);
            }))
            .child(
                svg()
                    .path("icons/search.svg")
                    .size(rpx(14.))
                    .text_color(theme::text_muted()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(if empty {
                        theme::text_muted()
                    } else {
                        theme::text_primary()
                    })
                    .when(empty, |text| text.child("Search genres"))
                    .when(!empty, |text| {
                        text.child(crate::text_input::render_text(&self.text_input, true))
                    }),
            )
            .when(empty, |input| input.child(crate::ui::input_caret()))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let focus = input_entity.read(cx).input_focus.clone();
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
            )
    }

    fn active_genre_filter(&self) -> Option<&str> {
        selected_genre(self.destination, &self.album_filter, &self.track_filter)
    }
}
