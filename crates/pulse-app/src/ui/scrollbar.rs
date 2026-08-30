use crate::theme::rpx;

use gpui::{
    Context, DragMoveEvent, ElementId, IntoElement, MouseButton, MouseDownEvent, Pixels, Render,
    ScrollHandle, Window, canvas, div, point, prelude::*,
};

use crate::theme;

const SCROLLBAR_INSET: f32 = 4.;
const SCROLLBAR_MIN_THUMB: f32 = 36.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollbarMetrics {
    pub(crate) thumb_height: Pixels,
    pub(crate) thumb_top: Pixels,
    pub(crate) travel: Pixels,
}

pub(crate) fn scrollbar_metrics(
    viewport_height: Pixels,
    max_scroll: Pixels,
    offset: Pixels,
    rem_size: Pixels,
) -> Option<ScrollbarMetrics> {
    if viewport_height <= Pixels::ZERO || max_scroll <= Pixels::ZERO {
        return None;
    }
    let inset = rpx(SCROLLBAR_INSET).to_pixels(rem_size);
    let min_thumb = rpx(SCROLLBAR_MIN_THUMB).to_pixels(rem_size);
    let rail_height = viewport_height - inset * 2.;
    if rail_height <= min_thumb {
        return None;
    }
    let content_height = viewport_height + max_scroll;
    let thumb_height =
        (rail_height * (viewport_height / content_height)).clamp(min_thumb, rail_height);
    let travel = rail_height - thumb_height;
    if travel <= Pixels::ZERO {
        return None;
    }
    let progress = (-offset / max_scroll).clamp(0., 1.);
    Some(ScrollbarMetrics {
        thumb_height,
        thumb_top: travel * progress,
        travel,
    })
}

#[derive(Clone)]
struct ScrollbarDrag;

impl Render for ScrollbarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(rpx(1.))
    }
}

pub(crate) struct Scrollbar {
    id: ElementId,
    thumb_id: ElementId,
    scroll: ScrollHandle,
    estimated_content_height: Option<Pixels>,
    last_metrics: Option<ScrollbarMetrics>,
    drag_offset: Option<Pixels>,
}

impl Scrollbar {
    pub(crate) fn new(id: impl Into<ElementId>, scroll: ScrollHandle) -> Self {
        let id = id.into();
        Self {
            thumb_id: (id.clone(), "thumb").into(),
            id,
            scroll,
            estimated_content_height: None,
            last_metrics: None,
            drag_offset: None,
        }
    }

    pub(crate) fn thumb_id(mut self, thumb_id: impl Into<ElementId>) -> Self {
        self.thumb_id = thumb_id.into();
        self
    }

    pub(crate) fn set_estimated_content_height(&mut self, height: Pixels) {
        self.estimated_content_height = Some(height.max(Pixels::ZERO));
    }

    pub(crate) fn set_scroll_handle(&mut self, scroll: ScrollHandle) {
        self.scroll = scroll;
        self.drag_offset = None;
        self.last_metrics = None;
    }

    fn max_scroll(&self) -> Pixels {
        let viewport_height = self.scroll.bounds().size.height;
        let measured = self.scroll.max_offset().y;
        let estimated = self
            .estimated_content_height
            .map(|height| height - viewport_height)
            .unwrap_or(Pixels::ZERO);
        measured.max(estimated).max(Pixels::ZERO)
    }

    fn current_metrics(&self, rem_size: Pixels) -> Option<ScrollbarMetrics> {
        scrollbar_metrics(
            self.scroll.bounds().size.height,
            self.max_scroll(),
            self.scroll.offset().y,
            rem_size,
        )
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rem_size = window.rem_size();
        let Some(metrics) = self.current_metrics(rem_size) else {
            return;
        };
        let local = event.position.y
            - self.scroll.bounds().top()
            - rpx(SCROLLBAR_INSET).to_pixels(rem_size);
        self.drag_offset = Some(
            if local >= metrics.thumb_top && local <= metrics.thumb_top + metrics.thumb_height {
                local - metrics.thumb_top
            } else {
                metrics.thumb_height / 2.
            },
        );
        self.scroll_for_pointer(event.position.y, rem_size);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_drag_move(
        &mut self,
        event: &DragMoveEvent<ScrollbarDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag_offset.is_some() {
            self.scroll_for_pointer(event.event.position.y, window.rem_size());
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn scroll_for_pointer(&self, pointer_y: Pixels, rem_size: Pixels) {
        let Some(metrics) = self.current_metrics(rem_size) else {
            return;
        };
        let target_top = (pointer_y
            - self.scroll.bounds().top()
            - rpx(SCROLLBAR_INSET).to_pixels(rem_size)
            - self.drag_offset.unwrap_or(metrics.thumb_height / 2.))
        .clamp(Pixels::ZERO, metrics.travel);
        let progress = target_top / metrics.travel;
        let max_scroll = self.max_scroll();
        self.scroll
            .set_offset(point(Pixels::ZERO, -(max_scroll * progress)));
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_offset.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Scrollbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem_size = window.rem_size();
        let metrics = self.current_metrics(rem_size);
        let dragging = self.drag_offset.is_some();
        let entity = cx.entity();
        div()
            .id(self.id.clone())
            .absolute()
            .top(rpx(SCROLLBAR_INSET))
            .right(rpx(2.))
            .bottom(rpx(SCROLLBAR_INSET))
            .w(rpx(4.))
            .rounded(rpx(2.))
            .when(metrics.is_some(), |scrollbar| {
                scrollbar
                    .bg(theme::bg_muted())
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_drag(ScrollbarDrag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .on_drag_move::<ScrollbarDrag>(cx.listener(Self::on_drag_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.finish_drag(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.finish_drag(cx)),
                    )
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, _, cx| {
                        entity.update(cx, |scrollbar, cx| {
                            let metrics = scrollbar.current_metrics(rem_size);
                            if scrollbar.last_metrics != metrics {
                                scrollbar.last_metrics = metrics;
                                cx.notify();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(metrics.map(|metrics| {
                div()
                    .id(self.thumb_id.clone())
                    .absolute()
                    .top(metrics.thumb_top)
                    .left_0()
                    .w_full()
                    .h(metrics.thumb_height)
                    .rounded(rpx(2.))
                    .bg(if dragging {
                        theme::text_secondary()
                    } else {
                        theme::text_muted()
                    })
                    .hover(|thumb| thumb.bg(theme::text_secondary()))
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_follow_the_scroll_range() {
        let rem_size = gpui::px(16.); // physical
        let pixels = |value| rpx(value).to_pixels(rem_size);
        assert_eq!(
            scrollbar_metrics(pixels(500.), pixels(0.), pixels(0.), rem_size),
            None
        );
        assert_eq!(
            scrollbar_metrics(pixels(40.), pixels(1_000.), pixels(0.), rem_size),
            None
        );
        assert_eq!(
            scrollbar_metrics(pixels(500.), pixels(f32::EPSILON), pixels(0.), rem_size),
            None
        );

        let top = scrollbar_metrics(pixels(500.), pixels(1_000.), pixels(0.), rem_size).unwrap();
        assert_eq!(top.thumb_top, pixels(0.));

        let bottom =
            scrollbar_metrics(pixels(500.), pixels(1_000.), pixels(-1_000.), rem_size).unwrap();
        assert_eq!(bottom.thumb_top, bottom.travel);

        let deep = scrollbar_metrics(pixels(500.), pixels(100_000.), pixels(0.), rem_size).unwrap();
        assert_eq!(deep.thumb_height, pixels(SCROLLBAR_MIN_THUMB));

        let scaled_rem_size = rem_size * 1.25;
        let scaled_pixels = |value| rpx(value).to_pixels(scaled_rem_size);
        let scaled = scrollbar_metrics(
            scaled_pixels(500.),
            scaled_pixels(100_000.),
            scaled_pixels(0.),
            scaled_rem_size,
        )
        .unwrap();
        assert_eq!(scaled.thumb_height, scaled_pixels(SCROLLBAR_MIN_THUMB));
    }
}
