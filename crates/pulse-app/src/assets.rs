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
    "icons/shuffle.svg",
    "icons/skip-back.svg",
    "icons/play.svg",
    "icons/pause.svg",
    "icons/skip-forward.svg",
    "icons/repeat-2.svg",
    "icons/repeat-1.svg",
    "icons/volume-2.svg",
    "icons/volume-1.svg",
    "icons/volume-x.svg",
    "icons/speaker.svg",
    "icons/bluetooth.svg",
    "icons/list-music.svg",
    "icons/activity.svg",
    "icons/library.svg",
    "icons/music.svg",
    "icons/mic-vocal.svg",
    "icons/database.svg",
    "icons/search.svg",
    "icons/settings.svg",
    "icons/check.svg",
    "icons/log-in.svg",
    "icons/arrow-left.svg",
    "icons/arrow-up-down.svg",
    "icons/ellipsis.svg",
    "icons/audio-lines.svg",
    "icons/chevron-down.svg",
    "icons/disc-3.svg",
    "icons/folder.svg",
    "icons/folder-plus.svg",
    "icons/plus.svg",
    "icons/refresh-cw.svg",
    "icons/circle-arrow-down.svg",
    "icons/loader.svg",
    "icons/user-round-search.svg",
    "icons/user.svg",
    "icons/x.svg",
    "icons/chevron-left.svg",
    "icons/chevron-right.svg",
    "icons/external-link.svg",
    "icons/info.svg",
    "icons/sliders-horizontal.svg",
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
