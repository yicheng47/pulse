use gpui::{
    Context, DragMoveEvent, ElementId, IntoElement, MouseButton, MouseDownEvent, Pixels, Render,
    ScrollHandle, Window, canvas, div, point, prelude::*, px,
};

use crate::theme;

const SCROLLBAR_INSET_PX: f32 = 4.;
const SCROLLBAR_MIN_THUMB_PX: f32 = 36.;

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
) -> Option<ScrollbarMetrics> {
    if viewport_height <= px(0.) || max_scroll <= px(0.) {
        return None;
    }
    let rail_height = viewport_height - px(SCROLLBAR_INSET_PX * 2.);
    if rail_height <= px(SCROLLBAR_MIN_THUMB_PX) {
        return None;
    }
    let content_height = viewport_height + max_scroll;
    let thumb_height = (rail_height * (viewport_height / content_height))
        .clamp(px(SCROLLBAR_MIN_THUMB_PX), rail_height);
    let travel = rail_height - thumb_height;
    if travel <= px(0.) {
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
        div().size(px(1.))
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
        self.estimated_content_height = Some(height.max(px(0.)));
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
            .unwrap_or(px(0.));
        measured.max(estimated).max(px(0.))
    }

    fn current_metrics(&self) -> Option<ScrollbarMetrics> {
        scrollbar_metrics(
            self.scroll.bounds().size.height,
            self.max_scroll(),
            self.scroll.offset().y,
        )
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(metrics) = self.current_metrics() else {
            return;
        };
        let local = event.position.y - self.scroll.bounds().top() - px(SCROLLBAR_INSET_PX);
        self.drag_offset = Some(
            if local >= metrics.thumb_top && local <= metrics.thumb_top + metrics.thumb_height {
                local - metrics.thumb_top
            } else {
                metrics.thumb_height / 2.
            },
        );
        self.scroll_for_pointer(event.position.y);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_drag_move(
        &mut self,
        event: &DragMoveEvent<ScrollbarDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag_offset.is_some() {
            self.scroll_for_pointer(event.event.position.y);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn scroll_for_pointer(&self, pointer_y: Pixels) {
        let Some(metrics) = self.current_metrics() else {
            return;
        };
        let target_top = (pointer_y
            - self.scroll.bounds().top()
            - px(SCROLLBAR_INSET_PX)
            - self.drag_offset.unwrap_or(metrics.thumb_height / 2.))
        .clamp(px(0.), metrics.travel);
        let progress = target_top / metrics.travel;
        let max_scroll = self.max_scroll();
        self.scroll
            .set_offset(point(px(0.), -(max_scroll * progress)));
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_offset.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Scrollbar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.current_metrics();
        let dragging = self.drag_offset.is_some();
        let entity = cx.entity();
        div()
            .id(self.id.clone())
            .absolute()
            .top(px(SCROLLBAR_INSET_PX))
            .right(px(2.))
            .bottom(px(SCROLLBAR_INSET_PX))
            .w(px(4.))
            .rounded(px(2.))
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
                            let metrics = scrollbar.current_metrics();
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
                    .rounded(px(2.))
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
        assert_eq!(scrollbar_metrics(px(500.), px(0.), px(0.)), None);
        assert_eq!(scrollbar_metrics(px(40.), px(1_000.), px(0.)), None);
        assert_eq!(scrollbar_metrics(px(500.), px(f32::EPSILON), px(0.)), None);

        let top = scrollbar_metrics(px(500.), px(1_000.), px(0.)).unwrap();
        assert_eq!(top.thumb_top, px(0.));

        let bottom = scrollbar_metrics(px(500.), px(1_000.), px(-1_000.)).unwrap();
        assert_eq!(bottom.thumb_top, bottom.travel);

        let deep = scrollbar_metrics(px(500.), px(100_000.), px(0.)).unwrap();
        assert_eq!(deep.thumb_height, px(SCROLLBAR_MIN_THUMB_PX));
    }
}
