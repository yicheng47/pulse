use super::{ScanCompletion, logic::FilterChip};
use crate::{
    backend::{DeleteAlbumOutcome, ScanOutcome},
    surfaces::Destination,
};

pub(super) fn filter_artists(artists: &[(String, u64)], search: &str) -> Vec<(String, u64)> {
    let needle = search.trim().to_lowercase();
    artists
        .iter()
        .filter(|(artist, _)| needle.is_empty() || artist.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

pub(super) fn filter_genres(genres: &[(String, u64)], search: &str) -> Vec<(String, u64)> {
    let needle = search.trim().to_lowercase();
    genres
        .iter()
        .filter(|(genre, _)| needle.is_empty() || genre.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

pub(super) fn selected_genre<'a>(
    destination: Destination,
    album_filter: &'a FilterChip,
    track_filter: &'a FilterChip,
) -> Option<&'a str> {
    let filter = match destination {
        Destination::Albums => album_filter,
        Destination::Tracks => track_filter,
        _ => return None,
    };
    match filter {
        FilterChip::Genre(genre) => Some(genre),
        _ => None,
    }
}

pub(super) fn delete_album_notice(
    outcome: &DeleteAlbumOutcome,
    reload_error: Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(db_error) = &outcome.db_error {
        parts.push(format!(
            "Deleted {} of {} audio files, but updating the library failed: {db_error}. Run \
             Delete Album again to finish the cleanup.",
            outcome.deleted_files, outcome.total_files
        ));
        if !outcome.failures.is_empty() {
            parts.push(format!("Could not delete: {}", outcome.failures.join("; ")));
        }
    } else if !outcome.failures.is_empty() {
        parts.push(format!(
            "Deleted {} of {} audio files. Could not delete: {}",
            outcome.deleted_files,
            outcome.total_files,
            outcome.failures.join("; ")
        ));
    }
    if let Some(reload) = reload_error {
        if parts.is_empty() {
            return Some(reload);
        }
        parts.push(format!("Reloading the library also failed: {reload}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

pub(super) fn scan_verified_presence(completion: &ScanCompletion) -> bool {
    matches!(
        completion,
        ScanCompletion::Completed {
            outcome: ScanOutcome::Completed | ScanOutcome::CompletedWithErrors,
            removals_suppressed: false,
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        deleted_files: usize,
        total_files: usize,
        failures: Vec<String>,
        db_error: Option<String>,
    ) -> DeleteAlbumOutcome {
        DeleteAlbumOutcome {
            deleted_ids: Vec::new(),
            deleted_files,
            total_files,
            failures,
            db_error,
        }
    }

    #[test]
    fn delete_album_notice_preserves_partial_state_when_the_reload_also_fails() {
        let notice = delete_album_notice(
            &outcome(3, 5, Vec::new(), Some("no such table".into())),
            Some("no such table: playlist_tracks".into()),
        )
        .unwrap();
        assert!(notice.contains("Deleted 3 of 5 audio files"));
        assert!(notice.contains("Run Delete Album again"));
        assert!(notice.contains("Reloading the library also failed"));
    }

    #[test]
    fn delete_album_notice_keeps_file_failures_alongside_a_db_failure() {
        let notice = delete_album_notice(
            &outcome(
                1,
                3,
                vec!["b.wav: permission denied".into()],
                Some("database is locked".into()),
            ),
            None,
        )
        .unwrap();
        assert!(notice.contains("Deleted 1 of 3 audio files"));
        assert!(notice.contains("database is locked"));
        assert!(notice.contains("b.wav: permission denied"));
    }

    #[test]
    fn delete_album_notice_reports_file_failures_and_clean_runs() {
        let notice =
            delete_album_notice(&outcome(1, 2, vec!["a.wav: offline".into()], None), None).unwrap();
        assert!(notice.contains("Deleted 1 of 2 audio files"));
        assert!(notice.contains("a.wav: offline"));

        assert!(delete_album_notice(&outcome(2, 2, Vec::new(), None), None).is_none());
        assert_eq!(
            delete_album_notice(
                &outcome(2, 2, Vec::new(), None),
                Some("reload broke".into())
            ),
            Some("reload broke".into())
        );
    }

    #[test]
    fn missing_marks_clear_only_after_presence_verifying_scans() {
        let completed = |outcome, removals_suppressed| ScanCompletion::Completed {
            outcome,
            removals_suppressed,
        };
        assert!(scan_verified_presence(&completed(
            ScanOutcome::Completed,
            false
        )));
        assert!(scan_verified_presence(&completed(
            ScanOutcome::CompletedWithErrors,
            false
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Completed,
            true
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Offline,
            false
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Failed,
            false
        )));
        assert!(!scan_verified_presence(&ScanCompletion::Cancelled));
    }

    #[test]
    fn artist_search_filters_case_insensitively_and_empty_matches_all() {
        let artists = vec![
            ("Daft Punk".to_string(), 22_u64),
            ("宇多田ヒカル".to_string(), 14),
            ("Kamasi Washington".to_string(), 17),
        ];
        let names = |search: &str| {
            filter_artists(&artists, search)
                .into_iter()
                .map(|(artist, _)| artist)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("").len(), 3);
        assert_eq!(names("daft"), ["Daft Punk"]);
        assert_eq!(names("  WASH  "), ["Kamasi Washington"]);
        assert_eq!(names("ヒカル"), ["宇多田ヒカル"]);
        assert!(names("zzz").is_empty());
        assert!(filter_artists(&[], "").is_empty());
    }

    #[test]
    fn genre_search_filters_case_insensitively_and_empty_matches_all() {
        let genres = vec![
            ("Asie".to_string(), 2_u64),
            ("J-pop".to_string(), 3),
            ("Musiques du monde".to_string(), 1),
        ];
        let names = |search: &str| {
            filter_genres(&genres, search)
                .into_iter()
                .map(|(genre, _)| genre)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("").len(), 3);
        assert_eq!(names("  J-POP "), ["J-pop"]);
        assert_eq!(names("monde"), ["Musiques du monde"]);
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn selected_genre_uses_the_visible_library_surface() {
        let albums = FilterChip::Genre("Jazz".to_string());
        let tracks = FilterChip::Genre("Rock".to_string());
        assert_eq!(
            selected_genre(Destination::Albums, &albums, &tracks),
            Some("Jazz")
        );
        assert_eq!(
            selected_genre(Destination::Tracks, &albums, &tracks),
            Some("Rock")
        );
        assert_eq!(selected_genre(Destination::Storage, &albums, &tracks), None);
        assert_eq!(
            selected_genre(Destination::Albums, &FilterChip::All, &tracks),
            None
        );
    }
}
