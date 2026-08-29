use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, IntoElement, RenderOnce, SharedString,
    Window, div, prelude::*, px,
};

use crate::theme;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct SettingsCard {
    children: Vec<AnyElement>,
}

impl SettingsCard {
    pub(crate) fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub(crate) fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for SettingsCard {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w_full()
            .px(px(16.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .children(self.children)
    }
}

#[derive(IntoElement)]
pub(crate) struct SettingsRow {
    id: Option<ElementId>,
    title: SharedString,
    description: SharedString,
    trailing: AnyElement,
    divider: bool,
    on_click: Option<ClickHandler>,
}

impl SettingsRow {
    pub(crate) fn new(
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        trailing: impl IntoElement,
    ) -> Self {
        Self {
            id: None,
            title: title.into(),
            description: description.into(),
            trailing: trailing.into_any_element(),
            divider: false,
            on_click: None,
        }
    }

    pub(crate) fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub(crate) fn divider(mut self, divider: bool) -> Self {
        self.divider = divider;
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let row = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .w_full()
            .py(px(13.))
            .when(self.divider, |row| {
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
                            .child(self.title),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(12.))
                            .text_color(theme::text_muted())
                            .child(self.description),
                    ),
            )
            .child(self.trailing);
        if let Some(id) = self.id {
            let mut row = row.id(id);
            if let Some(on_click) = self.on_click {
                row = row
                    .cursor_pointer()
                    .on_click(move |event, window, cx| on_click(event, window, cx));
            }
            row.into_any_element()
        } else {
            row.into_any_element()
        }
    }
}
