use crate::backend::TrackSortOrder;

pub(super) fn track_sort_label(sort: TrackSortOrder) -> &'static str {
    match sort {
        TrackSortOrder::Title => "TITLE",
        TrackSortOrder::Artist => "ARTIST",
        TrackSortOrder::Album => "ALBUM",
        TrackSortOrder::DateAdded => "DATE ADDED",
        TrackSortOrder::ReleaseYear => "YEAR",
        TrackSortOrder::Duration => "DURATION",
    }
}

pub(super) fn format_relative_time(timestamp_ms: i64, now_ms: i64) -> String {
    let elapsed = now_ms.saturating_sub(timestamp_ms).max(0);
    let minutes = elapsed / 60_000;
    if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 24 * 60 {
        format!("{} hr", minutes / 60)
    } else {
        format!("{} days", minutes / (24 * 60))
    }
}
