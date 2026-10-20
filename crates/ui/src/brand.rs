//! The BitBraid name and logo in the application.
//!
//! The drawings are the ones in `brand/`, built into the binary as
//! they are; nothing is redrawn or recoloured here. Which of the two
//! variants is shown follows the theme, as the terms at the end of
//! `LICENSE` ask.
//!
//! Where it is shown, a drawing is rasterized from its outlines at the
//! device pixels it takes on the screen, with smooth edges, and drawn one
//! to one: the toolkit's own picture of an SVG is made once at twice its
//! view box and shrunk on the graphics card, a pixel of the screen taking
//! one sample of several, and its edges come out stepped.
//!
//! Sources: the drawings in `brand/` and the terms for them in `LICENSE`;
//! gpui's `SvgRenderer` (`parse_svg`, `render_parsed`, `SvgSize`) and
//! `Window::scale_factor` documentation.

use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};

use gpui_kit::{
    App, DevicePixels, Image, ImageFormat, Img, Pixels, RenderImage, Styled, SvgRenderer, SvgSize,
    Window, img,
};

/// The product's name as people read it.
pub const NAME: &str = "BitBraid";

/// What desktops know the application by: the name of its desktop entry and
/// of its icon.
pub const APP_ID: &str = "bitbraid";

/// Width over height of the logo (mark and wordmark), from its view box.
pub const LOGO_ASPECT: f32 = 1161.7 / 502.0;

/// Width over height of the mark alone.
pub const MARK_ASPECT: f32 = 318.0 / 502.0;

/// The strands' colours on a dark background, in the order they enter
/// the mark at its top: amber on the left, sky in the middle, orchid on
/// the right, as `brand/mark.svg` draws them.
pub const STRANDS_ON_DARK: [u32; 3] = [0xffb83d, 0x62a8f5, 0xa98bf7];

/// The same on a light background.
pub const STRANDS_ON_LIGHT: [u32; 3] = [0xf2a318, 0x2f7bc8, 0x7c5fd6];

/// The icon's tile, from its top to its bottom.
pub const TILE: [u32; 2] = [0x1b2633, 0x0c1016];

const LOGO: &[u8] = include_bytes!("../../../brand/logo.svg");
const LOGO_ON_LIGHT: &[u8] = include_bytes!("../../../brand/logo-on-light.svg");
const MARK: &[u8] = include_bytes!("../../../brand/mark.svg");
const MARK_ON_LIGHT: &[u8] = include_bytes!("../../../brand/mark-on-light.svg");

fn cached(slot: &'static OnceLock<Arc<Image>>, bytes: &'static [u8]) -> Arc<Image> {
    slot.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Svg, bytes.to_vec())))
        .clone()
}

/// Mark and wordmark, for a dark or a light background.
pub fn logo(dark: bool) -> Arc<Image> {
    static DARK: OnceLock<Arc<Image>> = OnceLock::new();
    static LIGHT: OnceLock<Arc<Image>> = OnceLock::new();
    if dark {
        cached(&DARK, LOGO)
    } else {
        cached(&LIGHT, LOGO_ON_LIGHT)
    }
}

/// The mark alone, for a dark or a light background.
pub fn mark(dark: bool) -> Arc<Image> {
    static DARK: OnceLock<Arc<Image>> = OnceLock::new();
    static LIGHT: OnceLock<Arc<Image>> = OnceLock::new();
    if dark {
        cached(&DARK, MARK)
    } else {
        cached(&LIGHT, MARK_ON_LIGHT)
    }
}

/// The logo `width` wide (the toolkit's pixels, the zoom in them), for a
/// dark or a light background, rasterized at the device pixels it takes.
pub fn logo_image(dark: bool, width: Pixels, window: &mut Window, cx: &mut App) -> Img {
    let bytes = if dark { LOGO } else { LOGO_ON_LIGHT };
    drawn(
        Drawing::Logo,
        dark,
        bytes,
        f32::from(width),
        LOGO_ASPECT,
        window,
        cx,
    )
    .unwrap_or_else(|| img(logo(dark)).w(width).h(width / LOGO_ASPECT))
}

/// The mark alone `height` high (the toolkit's pixels, the zoom in
/// them), for a dark or a light background, rasterized at the device
/// pixels it takes.
pub fn mark_image(dark: bool, height: Pixels, window: &mut Window, cx: &mut App) -> Img {
    let bytes = if dark { MARK } else { MARK_ON_LIGHT };
    let width = height * MARK_ASPECT;
    drawn(
        Drawing::Mark,
        dark,
        bytes,
        f32::from(width),
        MARK_ASPECT,
        window,
        cx,
    )
    .unwrap_or_else(|| img(mark(dark)).w(width).h(height))
}

/// Which of the drawings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawing {
    Logo,
    Mark,
}

/// A drawing rasterized at one size.
struct Raster {
    drawing: Drawing,
    dark: bool,
    size: (i32, i32),
    image: Arc<RenderImage>,
}

/// How many rasters are kept: a few zooms and both variants. The one used
/// longest ago goes, and is taken out of the windows' textures.
const KEPT: usize = 8;

/// The longest side a drawing is rasterized at, in device pixels.
const LARGEST: f32 = 4096.0;

thread_local! {
    /// The rasters made, the one used last at the end. Views are drawn on
    /// the one thread the application has.
    static RASTERS: RefCell<Vec<Raster>> = const { RefCell::new(Vec::new()) };
}

/// The device pixels a drawing `width` wide (the toolkit's pixels) and
/// `aspect` times as wide as high takes in a window of this scale factor:
/// whole pixels, at least one, at most [`LARGEST`]; none for a size that
/// is no size.
fn device_size(width: f32, aspect: f32, scale: f32) -> Option<(i32, i32)> {
    let usable = |value: f32| value.is_finite() && value > 0.0;
    if !(usable(width) && usable(aspect) && usable(scale)) {
        return None;
    }
    let wide = (width * scale).round().clamp(1.0, LARGEST);
    let high = (wide / aspect).round().clamp(1.0, LARGEST);
    Some((wide as i32, high as i32))
}

/// The drawing rasterized from its outlines at exactly this many device
/// pixels, its edges smoothed.
fn rasterize(
    renderer: &SvgRenderer,
    bytes: &[u8],
    (wide, high): (i32, i32),
) -> Option<Arc<RenderImage>> {
    let parsed = renderer.parse_svg(bytes).ok()?;
    renderer
        .render_parsed(
            &parsed,
            SvgSize::ExactSize(gpui_kit::size(DevicePixels(wide), DevicePixels(high))),
        )
        .ok()
}

/// A drawing `width` wide as an image of the device pixels it takes: the
/// one made before at that size, else made now. None when it cannot be
/// made, and the toolkit's own picture of it is shown.
fn drawn(
    drawing: Drawing,
    dark: bool,
    bytes: &[u8],
    width: f32,
    aspect: f32,
    window: &mut Window,
    cx: &mut App,
) -> Option<Img> {
    let scale = window.scale_factor();
    let size = device_size(width, aspect, scale)?;
    let kept = RASTERS.with(|rasters| {
        let mut rasters = rasters.borrow_mut();
        let found = rasters.iter().position(|raster| {
            raster.drawing == drawing && raster.dark == dark && raster.size == size
        })?;
        let raster = rasters.remove(found);
        let image = raster.image.clone();
        rasters.push(raster);
        Some(image)
    });
    let image = match kept {
        Some(image) => image,
        None => {
            let image = rasterize(&cx.svg_renderer(), bytes, size)?;
            let gone = RASTERS.with(|rasters| {
                let mut rasters = rasters.borrow_mut();
                rasters.push(Raster {
                    drawing,
                    dark,
                    size,
                    image: image.clone(),
                });
                let over = rasters.len().saturating_sub(KEPT);
                rasters.drain(..over).collect::<Vec<_>>()
            });
            for raster in gone {
                cx.drop_image(raster.image, Some(window));
            }
            image
        }
    };
    let (wide, high) = size;
    Some(
        img(image)
            .w(gpui_kit::px(wide as f32 / scale))
            .h(gpui_kit::px(high as f32 / scale)),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::SvgRenderer;

    use super::{
        LARGEST, LOGO, LOGO_ASPECT, LOGO_ON_LIGHT, MARK, MARK_ASPECT, MARK_ON_LIGHT, device_size,
        rasterize,
    };

    /// The view box of an SVG file as (width, height).
    fn view_box(svg: &[u8]) -> (f32, f32) {
        let text = std::str::from_utf8(svg).expect("the logo is text");
        let start = text.find("viewBox=\"").expect("a view box") + "viewBox=\"".len();
        let end = start + text[start..].find('"').expect("a closing quote");
        let numbers: Vec<f32> = text[start..end]
            .split_whitespace()
            .map(|n| n.parse().expect("a number"))
            .collect();
        (numbers[2], numbers[3])
    }

    #[test]
    fn the_built_in_drawings_are_the_brand_files() {
        for svg in [LOGO, LOGO_ON_LIGHT, MARK, MARK_ON_LIGHT] {
            let text = std::str::from_utf8(svg).unwrap();
            assert!(text.starts_with("<svg"), "an SVG document");
            assert!(text.contains("<title"), "with a title for screen readers");
        }
        // The space the start page gives the logo matches its shape, and
        // both variants have the same shape.
        for svg in [LOGO, LOGO_ON_LIGHT] {
            let (width, height) = view_box(svg);
            assert!((width / height - LOGO_ASPECT).abs() < 0.001);
        }
        for svg in [MARK, MARK_ON_LIGHT] {
            let (width, height) = view_box(svg);
            assert!((width / height - MARK_ASPECT).abs() < 0.001);
        }
    }

    #[test]
    fn a_drawing_takes_whole_device_pixels_at_every_zoom_and_scale() {
        // 260 points at 100 % on a plain screen, at 150 %, and on a screen
        // of twice the pixels.
        assert_eq!(device_size(260.0, LOGO_ASPECT, 1.0), Some((260, 112)));
        assert_eq!(device_size(390.0, LOGO_ASPECT, 1.0), Some((390, 169)));
        assert_eq!(device_size(260.0, LOGO_ASPECT, 2.0), Some((520, 225)));
        assert_eq!(
            device_size(40.0 * MARK_ASPECT, MARK_ASPECT, 1.5),
            Some((38, 60))
        );
        // Sizes that are none are not drawn; huge ones are bounded.
        assert_eq!(device_size(0.0, LOGO_ASPECT, 1.0), None);
        assert_eq!(device_size(f32::NAN, LOGO_ASPECT, 1.0), None);
        assert_eq!(device_size(260.0, LOGO_ASPECT, f32::INFINITY), None);
        assert_eq!(device_size(260.0, 0.0, 1.0), None);
        let (wide, high) = device_size(1.0e9, LOGO_ASPECT, 1.0).unwrap();
        assert_eq!(wide as f32, LARGEST);
        assert!(high >= 1);
    }

    #[test]
    fn the_logo_is_rasterized_at_the_size_asked_with_soft_edges_and_its_colours() {
        let renderer = SvgRenderer::new(Arc::new(()));
        for (svg, sky) in [
            (LOGO, [0x62, 0xa8, 0xf5]),
            (LOGO_ON_LIGHT, [0x2f, 0x7b, 0xc8]),
        ] {
            let image = rasterize(&renderer, svg, (390, 169)).expect("the logo is drawn");
            let size = image.size(0);
            assert_eq!((size.width.0, size.height.0), (390, 169));
            // Blue, green, red and alpha, a pixel after the other.
            let bytes = image.as_bytes(0).unwrap();
            let (pixels, _) = bytes.as_chunks::<4>();
            let soft = pixels.iter().filter(|p| p[3] > 0 && p[3] < 255).count();
            let solid = pixels.iter().filter(|p| p[3] == 255).count();
            assert!(solid > 1000, "the drawing is there: {solid}");
            assert!(
                soft > 300,
                "the edges are smoothed, not stepped: {soft} pixels partly covered"
            );
            // Inside a strand the colour is the brand's own.
            assert!(
                pixels
                    .iter()
                    .any(|p| p[3] == 255 && [p[2], p[1], p[0]] == sky),
                "the sky strand keeps its colour"
            );
        }
        let mark = rasterize(&renderer, MARK, (25, 40)).expect("the mark is drawn");
        assert_eq!((mark.size(0).width.0, mark.size(0).height.0), (25, 40));
    }
}
