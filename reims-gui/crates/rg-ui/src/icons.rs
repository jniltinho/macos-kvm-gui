//! SVG icons, rasterized by egui_extras at the on-screen pixel size (crisp at any scale).
//!
//! - The app's own multicolor icons (`assets/icons`, from the Go app, BSD-3-Clause) and the
//!   macOS version art (`assets/versions`).
//! - Lucide icons (ISC, `assets/lucide`), recolored to white once and tinted per use, as
//!   PhotoCraft's `icons.rs` does (MIT OR Apache-2.0).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::load::Bytes;
use egui::{Color32, Image, ImageSource, Vec2};

macro_rules! table {
    ($dir:literal: $($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/", $dir, "/", $name, ".svg")))),*]
    };
}

const APP: &[(&str, &[u8])] = table!("icons":
    "audio", "display", "export", "forcestop", "fullscreen", "general", "gpu", "import", "network", "new",
    "preview", "remove", "settings", "shutdown", "start", "state-off", "state-paused", "state-running",
    "storage", "system", "usb", "window");

const VERSIONS: &[(&str, &[u8])] = table!("versions":
    "high-sierra", "mojave", "catalina", "big-sur", "monterey", "ventura", "sonoma", "sequoia", "tahoe");

const LUCIDE: &[(&str, &[u8])] = table!("lucide":
    "check", "chevron-down", "chevron-right", "circle-x", "copy", "download", "folder-open", "house", "info",
    "list", "monitor", "pause", "play", "rotate-cw", "settings", "square", "trash", "triangle-alert", "x");

/// The GPU tab's monochrome card (the Go app's `gpu-tab.svg`), tinted like Lucide.
const GPU_TAB: &[u8] = include_bytes!("../assets/icons/gpu-tab.svg");

const LOGO: &[u8] = include_bytes!("../assets/logo.svg");

fn find(table: &'static [(&str, &'static [u8])], name: &str) -> Option<&'static [u8]> {
    table.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

/// One of the app's multicolor icons; unknown names give the logo.
pub fn app(name: &str) -> ImageSource<'static> {
    let bytes = find(APP, name).unwrap_or(LOGO);
    ImageSource::Bytes {
        uri: format!("bytes://app/{name}.svg").into(),
        bytes: Bytes::Static(bytes),
    }
}

/// The art of a macOS version (`sonoma`), or the logo.
pub fn version(short: &str) -> ImageSource<'static> {
    let bytes = find(VERSIONS, short).unwrap_or(LOGO);
    ImageSource::Bytes {
        uri: format!("bytes://version/{short}.svg").into(),
        bytes: Bytes::Static(bytes),
    }
}

fn white() -> &'static HashMap<&'static str, Arc<[u8]>> {
    static MAP: OnceLock<HashMap<&'static str, Arc<[u8]>>> = OnceLock::new();
    MAP.get_or_init(|| {
        LUCIDE
            .iter()
            .copied()
            .chain([("gpu-tab", GPU_TAB)])
            .map(|(n, b)| {
                let s = String::from_utf8_lossy(b)
                    .replace("currentColor", "#ffffff")
                    .replace("#000000", "#ffffff");
                (n, Arc::from(s.into_bytes()))
            })
            .collect()
    })
}

/// A Lucide icon (or `gpu-tab`) tinted with `tint`, `size` points square.
pub fn lucide(name: &str, size: f32, tint: Color32) -> Image<'static> {
    let bytes = white()
        .get(name)
        .or_else(|| white().get("square"))
        .cloned()
        .unwrap_or_else(|| Arc::from(&[][..]));
    let src = ImageSource::Bytes {
        uri: format!("bytes://lucide/{name}.svg").into(),
        bytes: Bytes::Shared(bytes),
    };
    Image::new(src)
        .fit_to_exact_size(Vec2::splat(size))
        .tint(tint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lucide_icon_is_recolored() {
        for (name, bytes) in white() {
            let s = String::from_utf8_lossy(bytes);
            assert!(
                !s.contains("currentColor") && !s.contains("#000000"),
                "{name}"
            );
        }
        assert_eq!(white().len(), LUCIDE.len() + 1);
    }
}
