use crate::theme::rpx;

use gpui::{Context, FontWeight, IntoElement, div, prelude::*};

use crate::{
    surfaces::{Destination, NAV_GROUPS, Shell},
    theme, ui,
};

impl Shell {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        ui::SidebarIsland::new()
            .child(self.render_navigation(cx))
            .child(div().flex_1())
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut navigation = div().flex().flex_col().gap(rpx(32.)).w_full();

        for (header, destinations) in NAV_GROUPS {
            let mut section = ui::SidebarSection::new(*header);
            for destination in *destinations {
                section = section.child(self.render_nav_item(*destination, cx));
            }
            navigation = navigation.child(section);
        }

        navigation
    }

    fn render_nav_item(&self, destination: Destination, cx: &mut Context<Self>) -> ui::SidebarItem {
        let active = self.destination == destination;
        let mut item =
            ui::SidebarItem::new(destination.label(), destination.label(), destination.icon())
                .selected(active)
                .on_click(cx.listener(move |this, _, _, cx: &mut Context<Shell>| {
                    this.destination = destination;
                    this.library
                        .update(cx, |library, cx| library.set_destination(destination, cx));
                    cx.notify();
                }));
        if destination == Destination::Storage {
            item = item.accessory(render_storage_badge(
                self.library.read(cx).storage_root_count(),
            ));
        }
        item
    }
}

fn render_storage_badge(count: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .px(rpx(6.))
        .py(rpx(2.))
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(theme::text::CAPTION)
                .text_color(theme::text_muted())
                .child(count.to_string()),
        )
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::*;

    #[test]
    fn each_destination_maps_to_a_bundled_icon() {
        for destination in [
            Destination::Albums,
            Destination::Artists,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
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
}
