use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, KeyDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Rems, RenderOnce, Window, div, prelude::*,
};

use crate::theme;

type DismissHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type MouseMoveHandler = Rc<dyn Fn(&MouseMoveEvent, &mut Window, &mut App)>;
type MouseUpHandler = Rc<dyn Fn(&MouseUpEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct PopoverMenu {
    id: ElementId,
    children: Vec<AnyElement>,
    left: Option<Rems>,
    right: Option<Rems>,
    top: Option<Rems>,
    bottom: Option<Rems>,
    width: Rems,
    max_height: Option<Rems>,
    items_center: bool,
    focus_handle: Option<FocusHandle>,
    on_dismiss: Option<DismissHandler>,
    on_mouse_move: Option<MouseMoveHandler>,
    on_mouse_up: Option<MouseUpHandler>,
}

impl PopoverMenu {
    pub(crate) fn new(id: impl Into<ElementId>, width: Rems) -> Self {
        Self {
            id: id.into(),
            children: Vec::new(),
            left: None,
            right: None,
            top: None,
            bottom: None,
            width,
            max_height: None,
            items_center: false,
            focus_handle: None,
            on_dismiss: None,
            on_mouse_move: None,
            on_mouse_up: None,
        }
    }

    pub(crate) fn left(mut self, left: Rems) -> Self {
        self.left = Some(left);
        self
    }

    pub(crate) fn right(mut self, right: Rems) -> Self {
        self.right = Some(right);
        self
    }

    pub(crate) fn top(mut self, top: Rems) -> Self {
        self.top = Some(top);
        self
    }

    pub(crate) fn bottom(mut self, bottom: Rems) -> Self {
        self.bottom = Some(bottom);
        self
    }

    pub(crate) fn max_height(mut self, max_height: Rems) -> Self {
        self.max_height = Some(max_height);
        self
    }

    pub(crate) fn items_center(mut self) -> Self {
        self.items_center = true;
        self
    }

    pub(crate) fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub(crate) fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_mouse_move(
        mut self,
        handler: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse_move = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_mouse_up(
        mut self,
        handler: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse_up = Some(Rc::new(handler));
        self
    }

    pub(crate) fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for PopoverMenu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let escape = self.on_dismiss.clone();
        let mut menu = div()
            .id(self.id)
            .absolute()
            .when_some(self.left, |menu, left| menu.left(left))
            .when_some(self.right, |menu, right| menu.right(right))
            .when_some(self.top, |menu, top| menu.top(top))
            .when_some(self.bottom, |menu, bottom| menu.bottom(bottom))
            .flex()
            .flex_col()
            .when(self.items_center, |menu| menu.items_center())
            .gap(rpx(11.))
            .w(self.width)
            .when_some(self.max_height, |menu, max_height| menu.max_h(max_height))
            .p(rpx(14.))
            .rounded(rpx(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .occlude()
            .when_some(self.focus_handle, |menu, focus| menu.track_focus(&focus))
            .children(self.children);
        if let Some(on_dismiss) = self.on_dismiss {
            menu = menu.on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx));
        }
        if let Some(on_mouse_move) = self.on_mouse_move {
            menu = menu.on_mouse_move(move |event, window, cx| on_mouse_move(event, window, cx));
        }
        if let Some(on_mouse_up) = self.on_mouse_up {
            menu = menu.on_mouse_up(gpui::MouseButton::Left, move |event, window, cx| {
                on_mouse_up(event, window, cx)
            });
        }
        if let Some(on_escape) = escape {
            menu = menu.on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    on_escape(window, cx);
                }
            });
        }
        menu
    }
}

#[derive(IntoElement)]
pub(crate) struct ContextMenu {
    id: ElementId,
    position: gpui::Point<Pixels>,
    width: Rems,
    focus_handle: Option<FocusHandle>,
    children: Vec<AnyElement>,
    on_dismiss: Option<DismissHandler>,
}

impl ContextMenu {
    pub(crate) fn new(id: impl Into<ElementId>, position: gpui::Point<Pixels>) -> Self {
        Self {
            id: id.into(),
            position,
            width: rpx(160.),
            focus_handle: None,
            children: Vec::new(),
            on_dismiss: None,
        }
    }

    pub(crate) fn width(mut self, width: Rems) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub(crate) fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub(crate) fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let escape = self.on_dismiss.clone();
        let mut menu = div()
            .id(self.id)
            .absolute()
            .left(self.position.x)
            .top(self.position.y)
            .flex()
            .items_start()
            .w(self.width)
            .when_some(self.focus_handle, |menu, focus| menu.track_focus(&focus))
            .children(self.children);
        if let Some(on_dismiss) = self.on_dismiss {
            menu = menu.on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx));
        }
        if let Some(on_escape) = escape {
            menu = menu.on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    on_escape(window, cx);
                }
            });
        }
        menu
    }
}
