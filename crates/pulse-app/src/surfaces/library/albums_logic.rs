use crate::{backend::AlbumSortOrder, surfaces::SIDEBAR_WIDTH};

pub(super) const ALBUM_BODY_HORIZONTAL_PADDING: f32 = 28.;
pub(super) const ALBUM_GRID_GAP: f32 = 14.;
pub(super) const ALBUM_CARD_MIN_WIDTH: f32 = 200.;

pub(super) fn album_grid_columns(viewport_width: f32) -> u16 {
    let content_width =
        (viewport_width - SIDEBAR_WIDTH - ALBUM_BODY_HORIZONTAL_PADDING * 2.).max(0.);
    (((content_width + ALBUM_GRID_GAP) / (ALBUM_CARD_MIN_WIDTH + ALBUM_GRID_GAP)).floor() as u16)
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
        assert_eq!(album_grid_columns(1440.), 5);
        assert_eq!(album_grid_columns(1600.), 6);

        let five_column_threshold = SIDEBAR_WIDTH
            + ALBUM_BODY_HORIZONTAL_PADDING * 2.
            + ALBUM_CARD_MIN_WIDTH * 5.
            + ALBUM_GRID_GAP * 4.;
        assert_eq!(album_grid_columns(five_column_threshold - 1.), 4);
        assert_eq!(album_grid_columns(five_column_threshold), 5);
        assert_eq!(album_grid_columns(0.), 1);
    }
}
