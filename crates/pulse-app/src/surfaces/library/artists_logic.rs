use std::path::Path;

use crate::{backend::Artist, ui::SIDEBAR_SLOT_WIDTH};

pub(super) const ARTIST_BODY_HORIZONTAL_PADDING: f32 = 28.;
pub(super) const ARTIST_GRID_GAP: f32 = 32.;
pub(super) const ARTIST_CARD_MIN_WIDTH: f32 = 200.;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ArtistRoute {
    #[default]
    Index,
    Detail {
        artist: String,
    },
    Album {
        artist: String,
        album: String,
    },
}

impl ArtistRoute {
    pub(super) fn open_artist(&mut self, artist: String) {
        *self = Self::Detail { artist };
    }

    pub(super) fn open_album(&mut self, album: String) {
        if let Self::Detail { artist } = self {
            *self = Self::Album {
                artist: artist.clone(),
                album,
            };
        }
    }

    pub(super) fn back(&mut self) {
        *self = match self {
            Self::Album { artist, .. } => Self::Detail {
                artist: artist.clone(),
            },
            Self::Detail { .. } | Self::Index => Self::Index,
        };
    }

    pub(super) fn artist(&self) -> Option<&str> {
        match self {
            Self::Index => None,
            Self::Detail { artist } | Self::Album { artist, .. } => Some(artist),
        }
    }
}

pub(super) struct ArtistArtwork<'a> {
    pub photo: Option<&'a Path>,
    pub album_cover: Option<&'a Path>,
}

impl ArtistArtwork<'_> {
    pub(super) fn path(&self) -> Option<&Path> {
        self.photo.or(self.album_cover)
    }
}

pub(super) fn filter_artist_index<'a>(artists: &'a [Artist], search: &str) -> Vec<&'a Artist> {
    let needle = search.trim().to_lowercase();
    artists
        .iter()
        .filter(|artist| needle.is_empty() || artist.name.to_lowercase().contains(&needle))
        .collect()
}

pub(super) fn artist_grid_columns(viewport_width: f32) -> u16 {
    let content_width =
        (viewport_width - SIDEBAR_SLOT_WIDTH - ARTIST_BODY_HORIZONTAL_PADDING * 2.).max(0.);
    (((content_width + ARTIST_GRID_GAP) / (ARTIST_CARD_MIN_WIDTH + ARTIST_GRID_GAP)).floor() as u16)
        .max(1)
}

pub(super) fn format_artist_duration(duration_ms: u64) -> String {
    let total_minutes = duration_ms.div_ceil(60_000);
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours == 0 {
        format!("{minutes} min")
    } else if minutes == 0 {
        format!("{hours} h")
    } else {
        format!("{hours} h {minutes} min")
    }
}

pub(super) fn format_artist_count(count: u64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn artist(name: &str) -> Artist {
        Artist {
            id: 1,
            name: name.to_string(),
            name_key: name.to_string(),
            album_count: 2,
            track_count: 20,
            total_duration_ms: 60_000,
            earliest_added_ms: 1_700_000_000_000,
            earliest_added_year: Some(2024),
            cover_art_path: None,
            display_name: None,
            hidden: Some(false),
            mbid: None,
            photo_path: None,
            photo_source: None,
            enriched_at_ms: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn index_filter_round_trip_restores_the_full_artist_grid() {
        let artists = vec![artist("Frank Ocean"), artist("王菲"), artist("Fiona Apple")];
        let names = |query: &str| {
            filter_artist_index(&artists, query)
                .into_iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>()
        };

        assert_eq!(names("  fRaNk "), ["Frank Ocean"]);
        assert_eq!(names("王"), ["王菲"]);
        assert_eq!(
            names(""),
            ["Frank Ocean", "王菲", "Fiona Apple"],
            "clearing the filter restores the original index order"
        );
    }

    #[test]
    fn artists_to_artist_detail_to_album_detail_preserves_the_back_route() {
        let mut route = ArtistRoute::default();
        route.open_artist("Frank Ocean".to_string());
        assert_eq!(
            route,
            ArtistRoute::Detail {
                artist: "Frank Ocean".to_string()
            }
        );

        route.open_album("Blonde".to_string());
        assert_eq!(route.artist(), Some("Frank Ocean"));
        assert_eq!(
            route,
            ArtistRoute::Album {
                artist: "Frank Ocean".to_string(),
                album: "Blonde".to_string()
            }
        );

        route.back();
        assert_eq!(
            route,
            ArtistRoute::Detail {
                artist: "Frank Ocean".to_string()
            }
        );
        route.back();
        assert_eq!(route, ArtistRoute::Index);
    }

    #[test]
    fn artwork_prefers_a_future_photo_then_the_local_album_cover() {
        let photo = PathBuf::from("/cache/photo.jpg");
        let cover = PathBuf::from("/cache/cover.jpg");
        assert_eq!(
            ArtistArtwork {
                photo: Some(&photo),
                album_cover: Some(&cover)
            }
            .path(),
            Some(Path::new("/cache/photo.jpg"))
        );
        assert_eq!(
            ArtistArtwork {
                photo: None,
                album_cover: Some(&cover)
            }
            .path(),
            Some(Path::new("/cache/cover.jpg"))
        );
    }

    #[test]
    fn artist_grid_is_five_columns_at_the_approved_canvas_width() {
        assert_eq!(artist_grid_columns(1440.), 5);
        assert_eq!(artist_grid_columns(1663.), 5);
        assert_eq!(artist_grid_columns(1664.), 6);
        assert_eq!(artist_grid_columns(0.), 1);
    }

    #[test]
    fn formats_artist_duration_for_the_detail_hero() {
        assert_eq!(format_artist_duration(52 * 60_000), "52 min");
        assert_eq!(format_artist_duration(60 * 60_000), "1 h");
        assert_eq!(format_artist_duration(112 * 60_000), "1 h 52 min");
    }

    #[test]
    fn formats_artist_album_and_track_counts_for_zero_one_and_many() {
        for (count, albums, tracks) in [
            (0, "0 albums", "0 tracks"),
            (1, "1 album", "1 track"),
            (22, "22 albums", "22 tracks"),
        ] {
            assert_eq!(format_artist_count(count, "album"), albums);
            assert_eq!(format_artist_count(count, "track"), tracks);
        }
    }
}
