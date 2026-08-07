#![allow(dead_code)]

// Mirrors the variables in design/pulse-desktop.pen ("Design System · Cyberpunk
// Neon"). Magenta is interactive/action, purple is navigation/informational,
// lime is signal/quality readouts, greys are everything else.

use gpui::{Rgba, rgb, rgba};

pub fn bg_inset() -> Rgba {
    rgb(0x0a0a0a)
}

pub fn bg_page() -> Rgba {
    rgb(0x0f0f0f)
}

pub fn bg_surface() -> Rgba {
    rgb(0x161615)
}

pub fn bg_surface_alt() -> Rgba {
    rgb(0x111110)
}

pub fn bg_muted() -> Rgba {
    rgb(0x1e1e1c)
}

pub fn bg_elevated() -> Rgba {
    rgb(0x282825)
}

pub fn border() -> Rgba {
    rgb(0x312f2c)
}

pub fn border_strong() -> Rgba {
    rgb(0x403d39)
}

pub fn accent() -> Rgba {
    rgb(0xff2d7e)
}

pub fn accent_bright() -> Rgba {
    rgb(0xff74b8)
}

pub fn accent_soft() -> Rgba {
    rgb(0x2a0e1c)
}

pub fn bg_selected() -> Rgba {
    rgb(0x241019)
}

pub fn primary() -> Rgba {
    rgb(0xb15cff)
}

pub fn primary_soft() -> Rgba {
    rgb(0x241334)
}

pub fn signature() -> Rgba {
    rgb(0xf3b8c6)
}

pub fn signature_soft() -> Rgba {
    rgb(0x2a1d22)
}

pub fn quality() -> Rgba {
    rgb(0x9de23a)
}

pub fn quality_soft() -> Rgba {
    rgb(0x111a0b)
}

pub fn quality_border() -> Rgba {
    rgb(0x4f7b22)
}

pub fn meter() -> Rgba {
    rgb(0x9de23a)
}

pub fn info() -> Rgba {
    rgb(0xb15cff)
}

pub fn danger() -> Rgba {
    rgb(0xff4d6d)
}

pub fn danger_soft() -> Rgba {
    rgb(0x2a1016)
}

pub fn scrim() -> Rgba {
    rgba(0x0a0a0acc)
}

pub fn warning() -> Rgba {
    rgb(0xff8a2a)
}

pub fn text_primary() -> Rgba {
    rgb(0xf0f0ee)
}

pub fn text_secondary() -> Rgba {
    rgb(0x9d9d99)
}

pub fn text_muted() -> Rgba {
    rgb(0x6a6a66)
}

pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 8.0;

pub const FONT_DISPLAY: &str = "Rajdhani";
pub const FONT_SANS: &str = "Inter";
pub const FONT_MONO: &str = "Geist Mono";
