use crate::{backend::AlbumSortOrder, ui::SIDEBAR_SLOT_WIDTH};

pub(super) const ALBUM_BODY_HORIZONTAL_PADDING: f32 = 28.;
pub(super) const ALBUM_GRID_GAP: f32 = 14.;
pub(super) const ALBUM_CARD_MIN_WIDTH: f32 = 200.;

pub(super) fn album_grid_columns(viewport_width: f32, scale: f32) -> u16 {
    let content_width =
        (viewport_width - SIDEBAR_SLOT_WIDTH * scale - ALBUM_BODY_HORIZONTAL_PADDING * 2. * scale)
            .max(0.);
    (((content_width + ALBUM_GRID_GAP * scale) / ((ALBUM_CARD_MIN_WIDTH + ALBUM_GRID_GAP) * scale))
        .floor() as u16)
        .max(1)
}

pub(super) fn album_sort_label(sort: AlbumSortOrder) -> &'static str {
    match sort {
        AlbumSortOrder::Title => "TITLE",
        AlbumSortOrder::Artist => "ARTIST",
        AlbumSortOrder::DateAdded => "DATE ADDED",
        AlbumSortOrder::ReleaseYear => "YEAR",
        AlbumSortOrder::Duration => "DURATION",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_grid_respects_the_card_minimum_across_window_sizes() {
        let default_scale = 1.0; // 16 px rem size divided by the 16 px baseline.
        assert_eq!(album_grid_columns(1440., default_scale), 5);
        assert_eq!(album_grid_columns(1600., default_scale), 6);

        let five_column_threshold = SIDEBAR_SLOT_WIDTH
            + ALBUM_BODY_HORIZONTAL_PADDING * 2.
            + ALBUM_CARD_MIN_WIDTH * 5.
            + ALBUM_GRID_GAP * 4.;
        assert_eq!(
            album_grid_columns(five_column_threshold - 1., default_scale),
            4
        );
        assert_eq!(album_grid_columns(five_column_threshold, default_scale), 5);
        assert_eq!(album_grid_columns(0., default_scale), 1);
        assert_eq!(album_grid_columns(1440., 1.25), 4);
    }
}
