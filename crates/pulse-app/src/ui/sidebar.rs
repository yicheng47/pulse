use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, IntoElement, RenderOnce, SharedString,
    Window, div, prelude::*, svg,
};

use crate::theme;

pub(crate) const SIDEBAR_ISLAND_WIDTH: f32 = 236.0;
pub(crate) const SIDEBAR_SLOT_WIDTH: f32 = 248.0;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct SidebarIsland {
    children: Vec<AnyElement>,
}

impl SidebarIsland {
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

impl RenderOnce for SidebarIsland {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(rpx(SIDEBAR_SLOT_WIDTH))
            .h_full()
            .pt(rpx(12.))
            .pb(rpx(12.))
            .pl(rpx(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w(rpx(SIDEBAR_ISLAND_WIDTH))
                    .pt(rpx(20.))
                    .pr(rpx(12.))
                    .pb(rpx(16.))
                    .pl(rpx(12.))
                    .overflow_hidden()
                    .rounded(rpx(12.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_surface())
                    .children(self.children),
            )
    }
}

#[derive(IntoElement)]
pub(crate) struct SidebarSection {
    header: SharedString,
    children: Vec<AnyElement>,
}

impl SidebarSection {
    pub(crate) fn new(header: impl Into<SharedString>) -> Self {
        Self {
            header: header.into(),
            children: Vec::new(),
        }
    }

    pub(crate) fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for SidebarSection {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(rpx(10.))
            .w_full()
            .child(
                div().flex().px(rpx(12.)).w_full().child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .font_weight(FontWeight::BOLD)
                        .text_size(theme::text::CAPTION)
                        .text_color(theme::text_muted())
                        .child(self.header),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(4.))
                    .w_full()
                    .children(self.children),
            )
    }
}

#[derive(IntoElement)]
pub(crate) struct SidebarItem {
    id: ElementId,
    label: SharedString,
    icon: SharedString,
    selected: bool,
    accessory: Option<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl SidebarItem {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        icon: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: icon.into(),
            selected: false,
            accessory: None,
            on_click: None,
        }
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn accessory(mut self, accessory: impl IntoElement) -> Self {
        self.accessory = Some(accessory.into_any_element());
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

impl RenderOnce for SidebarItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hover_group = SharedString::from(format!("sidebar-item-{}", self.label));
        let mut item = div()
            .id(self.id)
            .group(hover_group.clone())
            .flex()
            .items_center()
            .gap(rpx(12.))
            .w_full()
            .px(rpx(12.))
            .py(rpx(10.))
            .rounded(rpx(theme::RADIUS_MD))
            .when(self.selected, |item| item.bg(theme::bg_elevated()))
            .when(!self.selected, |item| {
                item.hover(|style| style.bg(theme::accent_soft()))
            })
            .cursor_pointer()
            .child(
                svg()
                    .path(self.icon)
                    .size(rpx(17.))
                    .flex_none()
                    .text_color(if self.selected {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    })
                    .when(!self.selected, |icon| {
                        icon.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::accent())
                        })
                    }),
            )
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::text::LABEL_LARGE)
                    .text_color(if self.selected {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .when(!self.selected, |label| {
                        label.group_hover(hover_group, |style| {
                            style.text_color(theme::text_primary())
                        })
                    })
                    .child(self.label),
            );
        if let Some(accessory) = self.accessory {
            item = item.child(div().flex_1()).child(accessory);
        }
        if let Some(on_click) = self.on_click {
            item = item.on_click(move |event, window, cx| on_click(event, window, cx));
        }
        item
    }
}
