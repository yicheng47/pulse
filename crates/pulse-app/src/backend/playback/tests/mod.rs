use super::*;
use crate::backend::settings as app_settings;

mod devices;
mod events;
mod logic;
mod queue;

fn output_device(id: device::DeviceId, uid: &str, name: &str) -> device::Device {
    device::Device {
        id,
        uid: uid.to_string(),
        name: name.to_string(),
    }
}

fn library_track(id: crate::backend::library::TrackId, path: PathBuf, title: &str) -> Track {
    Track {
        id,
        storage_root_id: 1,
        path,
        title: Some(title.to_string()),
        artist: Some("Artist".to_string()),
        album: Some("Album".to_string()),
        album_artist: None,
        year: None,
        genre: None,
        track_number: None,
        disc_number: None,
        duration_ms: Some(1_000),
        sample_rate_hz: Some(44_100),
        bit_depth: Some(16),
        channels: Some(2),
        file_size_bytes: 1,
        modified_at_ns: 1,
        cover_art_path: None,
        cover_art_mime_type: None,
        added_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn wav_tracks(directory: &Path, names: &[&str]) -> Vec<Track> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let path = directory.join(format!("{name}.wav"));
            crate::backend::library::metadata::write_test_wav(&path, name, "Artist", "Album")
                .unwrap();
            library_track(index as i64 + 1, path, name)
        })
        .collect()
}

fn truncate_wav(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    std::fs::write(path, &bytes[..20]).unwrap();
}

fn now_playing(path: &str) -> PlaybackEvent {
    PlaybackEvent::NowPlaying {
        source: pulse_engine::PlayableSource {
            path: PathBuf::from(path),
            duration_ms: Some(268_000),
        },
        format: PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        },
    }
}
