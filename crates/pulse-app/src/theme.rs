#![allow(dead_code)]

// Tokens mirror the variables in design/pulse-desktop.pen. Amber is
// interactive/action, bright gold is signal/quality readouts, warm brown is
// selected-state only, greys are everything else.

use gpui::{Rgba, rgb};

pub fn bg_inset() -> Rgba {
    rgb(0x0a0a0a)
}

pub fn bg_page() -> Rgba {
    rgb(0x0f0f0f)
}

pub fn bg_surface() -> Rgba {
    rgb(0x161615)
}

pub fn bg_muted() -> Rgba {
    rgb(0x1e1e1c)
}

pub fn bg_elevated() -> Rgba {
    rgb(0x282825)
}

pub fn accent() -> Rgba {
    rgb(0xf5a624)
}

pub fn accent_bright() -> Rgba {
    rgb(0xffcb74)
}

pub fn accent_soft() -> Rgba {
    rgb(0x373127)
}

pub fn text_primary() -> Rgba {
    rgb(0xf0f0ee)
}
