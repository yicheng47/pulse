use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, IntoElement, Rems, RenderOnce,
    SharedString, Window, div, prelude::*,
};

use crate::{
    theme,
    ui::{Button, ButtonVariant, IconButton},
};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct Modal {
    id: ElementId,
    title: SharedString,
    body: AnyElement,
    footer: Option<AnyElement>,
    width: Rems,
    busy: bool,
    close_id: ElementId,
    on_close: Option<ClickHandler>,
}

impl Modal {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        body: impl IntoElement,
    ) -> Self {
        let id = id.into();
        Self {
            close_id: (id.clone(), "close").into(),
            id,
            title: title.into(),
            body: body.into_any_element(),
            footer: None,
            width: rpx(500.),
            busy: false,
            on_close: None,
        }
    }

    pub(crate) fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub(crate) fn width(mut self, width: Rems) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub(crate) fn close_id(mut self, id: impl Into<ElementId>) -> Self {
        self.close_id = id.into();
        self
    }

    pub(crate) fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let close = self.on_close;
        div()
            .id(self.id)
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(self.width)
                    .overflow_hidden()
                    .rounded(rpx(theme::RADIUS_LG))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::bg_surface())
                    .child(
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
                                    .child(self.title),
                            )
                            .children(
                                (!self.busy)
                                    .then(|| {
                                        close.map(|close| {
                                            IconButton::new(self.close_id, "icons/x.svg").on_click(
                                                move |event, window, cx| close(event, window, cx),
                                            )
                                        })
                                    })
                                    .flatten(),
                            ),
                    )
                    .child(self.body)
                    .children(self.footer.map(|footer| {
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(rpx(9.))
                            .h(rpx(62.))
                            .flex_none()
                            .px(rpx(22.))
                            .border_t_1()
                            .border_color(theme::border())
                            .child(footer)
                    })),
            )
    }
}

#[derive(IntoElement)]
pub(crate) struct ConfirmDialog {
    id: ElementId,
    title: SharedString,
    body: AnyElement,
    width: Rems,
    busy: bool,
    confirm_label: SharedString,
    busy_label: SharedString,
    cancel_id: ElementId,
    confirm_id: ElementId,
    close_id: ElementId,
    on_cancel: Option<ClickHandler>,
    on_confirm: Option<ClickHandler>,
}

impl ConfirmDialog {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        body: impl IntoElement,
    ) -> Self {
        let id = id.into();
        Self {
            cancel_id: (id.clone(), "cancel").into(),
            confirm_id: (id.clone(), "confirm").into(),
            close_id: (id.clone(), "close").into(),
            id,
            title: title.into(),
            body: body.into_any_element(),
            width: rpx(500.),
            busy: false,
            confirm_label: "Confirm".into(),
            busy_label: "Working…".into(),
            on_cancel: None,
            on_confirm: None,
        }
    }

    pub(crate) fn width(mut self, width: Rems) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub(crate) fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    pub(crate) fn busy_label(mut self, label: impl Into<SharedString>) -> Self {
        self.busy_label = label.into();
        self
    }

    pub(crate) fn cancel_id(mut self, id: impl Into<ElementId>) -> Self {
        self.cancel_id = id.into();
        self
    }

    pub(crate) fn confirm_id(mut self, id: impl Into<ElementId>) -> Self {
        self.confirm_id = id.into();
        self
    }

    pub(crate) fn close_id(mut self, id: impl Into<ElementId>) -> Self {
        self.close_id = id.into();
        self
    }

    pub(crate) fn on_cancel(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_cancel = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_confirm(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_confirm = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ConfirmDialog {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let cancel = self.on_cancel;
        let confirm = self.on_confirm;
        let cancel_button = Button::new(self.cancel_id, "Cancel").disabled(self.busy);
        let cancel_button = if let Some(cancel) = cancel.clone() {
            cancel_button.on_click(move |event, window, cx| cancel(event, window, cx))
        } else {
            cancel_button.disabled(true)
        };
        let confirm_button = Button::new(
            self.confirm_id,
            if self.busy {
                self.busy_label
            } else {
                self.confirm_label
            },
        )
        .variant(ButtonVariant::Danger)
        .disabled(self.busy);
        let confirm_button = if let Some(confirm) = confirm {
            confirm_button.on_click(move |event, window, cx| confirm(event, window, cx))
        } else {
            confirm_button.disabled(true)
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(rpx(9.))
            .child(cancel_button)
            .child(confirm_button);
        let modal = Modal::new(self.id, self.title, self.body)
            .width(self.width)
            .busy(self.busy)
            .close_id(self.close_id)
            .footer(footer);
        if let Some(cancel) = cancel {
            modal
                .on_close(move |event, window, cx| cancel(event, window, cx))
                .into_any_element()
        } else {
            modal.into_any_element()
        }
    }
}
