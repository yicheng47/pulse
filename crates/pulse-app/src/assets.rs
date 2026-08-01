use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! assets {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../assets/", $path)))),*]
    };
}

const ASSETS: &[(&str, &[u8])] = assets![
    "fonts/Rajdhani-Medium.ttf",
    "fonts/Rajdhani-Bold.ttf",
    "fonts/Inter-Variable.ttf",
    "fonts/GeistMono-Variable.ttf",
    "app-icon/v06cM3.png",
    "icons/shuffle.svg",
    "icons/skip-back.svg",
    "icons/play.svg",
    "icons/pause.svg",
    "icons/skip-forward.svg",
    "icons/repeat-2.svg",
    "icons/speaker.svg",
    "icons/list-music.svg",
    "icons/pulse-mark.svg",
    "icons/library.svg",
    "icons/music.svg",
    "icons/database.svg",
    "icons/search.svg",
    "icons/settings.svg",
    "icons/chevrons-left.svg",
    "icons/check.svg",
    "icons/log-in.svg",
    "icons/arrow-left.svg",
    "icons/ellipsis.svg",
    "icons/audio-lines.svg",
    "icons/chevron-left.svg",
    "icons/chevron-right.svg",
    "icons/chevron-down.svg",
    "icons/disc-3.svg",
    "icons/folder.svg",
    "icons/folder-plus.svg",
    "icons/plus.svg",
    "icons/refresh-cw.svg",
    "icons/user-round-search.svg",
    "icons/x.svg",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(asset_path, _)| *asset_path == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(asset_path, _)| asset_path.starts_with(path))
            .map(|(asset_path, _)| SharedString::from(*asset_path))
            .collect())
    }
}

pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    ASSETS
        .iter()
        .filter(|(path, _)| path.ends_with(".ttf"))
        .map(|(_, bytes)| Cow::Borrowed(*bytes))
        .collect()
}
