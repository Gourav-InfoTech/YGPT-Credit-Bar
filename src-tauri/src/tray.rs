use crate::models::Severity;
use once_cell::sync::Lazy;
use tauri::image::Image;

/// Embedded YourGPT logo (white silhouette). We rasterize this once at startup, then for each
/// severity we recolor the RGB channels while keeping the original alpha to get tinted versions.
const LOGO_SVG: &[u8] = include_bytes!("../icons/logo.svg");

/// Final tray icon size in physical pixels. macOS menu bar is 22 logical pixels tall; rendering
/// at 44 gives us a crisp result on Retina displays.
const ICON_PX: u32 = 44;
/// How much padding around the logo inside the icon. macOS HIG suggests ~2px of breathing room.
const PADDING_PX: u32 = 4;

/// Rasterized logo: RGBA bytes (premultiplied) of the logo at ICON_PX x ICON_PX, white on
/// transparent. We extract just the alpha channel to use as a stencil for severity colors.
static LOGO_ALPHA: Lazy<Vec<u8>> = Lazy::new(rasterize_logo_alpha);

fn rasterize_logo_alpha() -> Vec<u8> {
    let tree = match usvg::Tree::from_data(LOGO_SVG, &usvg::Options::default()) {
        Ok(t) => t,
        Err(e) => {
            log::error!("failed to parse logo SVG: {e}");
            return vec![0u8; (ICON_PX * ICON_PX) as usize];
        }
    };

    let mut pixmap = match tiny_skia::Pixmap::new(ICON_PX, ICON_PX) {
        Some(p) => p,
        None => return vec![0u8; (ICON_PX * ICON_PX) as usize],
    };

    let svg_size = tree.size();
    let target = (ICON_PX - PADDING_PX * 2) as f32;
    let scale = (target / svg_size.width()).min(target / svg_size.height());

    let scaled_w = svg_size.width() * scale;
    let scaled_h = svg_size.height() * scale;
    let offset_x = (ICON_PX as f32 - scaled_w) / 2.0;
    let offset_y = (ICON_PX as f32 - scaled_h) / 2.0;

    let transform = tiny_skia::Transform::from_translate(offset_x, offset_y).pre_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Extract just the alpha channel — that's our stencil.
    pixmap
        .data()
        .chunks_exact(4)
        .map(|px| px[3])
        .collect()
}

pub fn icon_for(severity: Severity) -> Image<'static> {
    let alpha = &*LOGO_ALPHA;
    let color = severity_color(severity);
    let mut rgba = Vec::with_capacity((ICON_PX * ICON_PX * 4) as usize);
    for &a in alpha.iter() {
        rgba.push(color[0]);
        rgba.push(color[1]);
        rgba.push(color[2]);
        rgba.push(a);
    }
    let leaked: &'static [u8] = Box::leak(rgba.into_boxed_slice());
    Image::new(leaked, ICON_PX, ICON_PX)
}

fn severity_color(s: Severity) -> [u8; 3] {
    match s {
        Severity::Idle => [200, 200, 205], // soft gray (visible in both light and dark menu bars)
        Severity::Ok => [52, 199, 89],     // system green
        Severity::Warn => [255, 204, 0],   // system yellow
        Severity::Alert => [255, 69, 58],  // system red
    }
}
