//! Two versions of an image: the new one above the old one, each at its
//! own size, or one laid over the other: blended, swiped, or reduced to
//! the pixels that differ.
//!
//! The first three are layout only. The fourth compares the decoded
//! pixels and hands the result back as an uncompressed BMP, which needs
//! no encoder beyond the few lines here.
//!
//! A picture is drawn from a copy with a border of one pixel that repeats
//! its edge ([`bordered`]), laid out one of its pixels larger on every side
//! and cut to its place ([`own_size`]). The renderer keeps every picture as
//! a tile among others in one texture and reads it smoothly; a small
//! picture shown large is read half a pixel past the tile's edge along its
//! outline, which without the border would take the colour of whatever
//! lies beside the tile. With the border what is read there is the
//! picture's own edge, wherever the tile is.
//!
//! Sources: Microsoft's description of the bitmap file format
//! (BITMAPFILEHEADER, BITMAPINFOHEADER); gpui documentation (`img`,
//! `Image`, `RenderImage`).

use std::sync::Arc;

use gpui_kit::{Image, ImageFormat, RenderImage};

/// How the two versions are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The new one above the old one, each by itself.
    Apart,
    /// The old one laid over the new one, more or less see-through.
    Onion,
    /// One over the other with a divider that is dragged.
    Swipe,
    /// Only the pixels that are not the same.
    Difference,
}

impl Mode {
    /// The ways of laying one version over the other, each with the word
    /// on its switch, in the order of the switches.
    pub const OVER: [(Mode, bitbraid_strings::Message); 3] = [
        (Mode::Onion, message!("diff-image-overlay")),
        (Mode::Swipe, message!("diff-image-swipe")),
        (Mode::Difference, message!("diff-image-difference")),
    ];

    /// The mode after the switch of `which` was pressed: that way of
    /// laying them over each other, or apart again when it was on.
    pub fn switched(self, which: Mode) -> Mode {
        if self == which { Mode::Apart } else { which }
    }
}

/// One version of the image.
#[derive(Debug, Clone)]
pub struct Side {
    pub image: Arc<Image>,
    /// Size of the file in bytes.
    pub bytes: usize,
    /// The picture as it is drawn, once it has been decoded. Stays absent
    /// for what cannot be read as a picture.
    pub shown: Option<Shown>,
}

/// A decoded picture ready to be drawn: with a border of one pixel around
/// it that repeats its edge.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The picture with its border, two pixels wider and higher.
    pub picture: Arc<RenderImage>,
    /// Width of the picture itself, in its own pixels.
    pub width: u32,
    /// Height of the picture itself.
    pub height: u32,
}

/// `data` (four bytes a pixel, `width` in a row, `height` rows) with a
/// border of one pixel that repeats the pixel it lies against. None for
/// nothing, for bytes that are not that many, and for what is too large to
/// count.
pub fn with_border(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let (wide, tall) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let row_bytes = wide.checked_mul(4)?;
    if wide == 0 || tall == 0 || data.len() != row_bytes.checked_mul(tall)? {
        return None;
    }
    let bordered_row = row_bytes.checked_add(8)?;
    let mut out = Vec::with_capacity(bordered_row.checked_mul(tall.checked_add(2)?)?);
    let mut push_row = |row: &[u8]| -> Option<()> {
        out.extend_from_slice(row.get(..4)?);
        out.extend_from_slice(row);
        out.extend_from_slice(row.get(row_bytes.checked_sub(4)?..)?);
        Some(())
    };
    let mut rows = data.chunks_exact(row_bytes);
    let first = rows.clone().next()?;
    let last = rows.clone().next_back()?;
    push_row(first)?;
    rows.try_for_each(&mut push_row)?;
    push_row(last)?;
    Some(out)
}

/// Every frame of a decoded picture with such a border, for drawing.
/// None for a picture without a frame or with one that cannot be copied.
pub fn bordered(decoded: &RenderImage) -> Option<Shown> {
    let mut frames: Vec<image::Frame> = Vec::new();
    let mut first: Option<(u32, u32)> = None;
    for index in 0..decoded.frame_count() {
        let size = decoded.size(index);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let bytes = with_border(decoded.as_bytes(index)?, width, height)?;
        let buffer =
            image::RgbaImage::from_raw(width.checked_add(2)?, height.checked_add(2)?, bytes)?;
        frames.push(image::Frame::from_parts(buffer, 0, 0, decoded.delay(index)));
        first.get_or_insert((width, height));
    }
    let (width, height) = first?;
    Some(Shown {
        picture: Arc::new(RenderImage::new(frames)),
        width,
        height,
    })
}

/// What a picture of `width` by `height` pixels is shown as at its own
/// size: a pixel for a pixel, and smaller only where the place it has is
/// not `room` wide. The width and height it is shown at, and how large
/// one of its pixels is then. None for no picture, no room, and numbers
/// that are none.
pub fn own_size(width: u32, height: u32, room: f32) -> Option<(f32, f32, f32)> {
    if width == 0 || height == 0 || !room.is_finite() || room <= 0.0 {
        return None;
    }
    let (wide, tall) = (width as f32, height as f32);
    let scale = (room / wide).min(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some((wide * scale, tall * scale, scale))
}

/// The format of an image file, by its name. None for what is not shown
/// as a picture.
pub fn format_of(path: &str) -> Option<ImageFormat> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "ico" => ImageFormat::Ico,
        "svg" => ImageFormat::Svg,
        _ => return None,
    })
}

/// A size in bytes for a caption.
pub fn human_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let bytes_f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes_f < KIB * KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{:.1} MiB", bytes_f / (KIB * KIB))
    }
}

/// Decoded pixels: four bytes each, row after row, and the width and
/// height they make.
#[derive(Debug, Clone, Copy)]
pub struct Pixels<'a> {
    /// The bytes, four per pixel.
    pub data: &'a [u8],
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

/// What comparing two images gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compared {
    /// The picture of the difference, as a BMP file.
    pub bmp: Vec<u8>,
    /// Pixels that are not the same in both.
    pub differing: usize,
    /// Pixels compared: the area that holds both images.
    pub total: usize,
    /// Width and height of that area.
    pub size: (u32, u32),
}

/// Most pixels compared; larger images are not.
pub const MAX_PIXELS: usize = 24_000_000;

/// Compare two decoded images pixel by pixel over the area that holds
/// both. A pixel only one of them has counts as different. What is the
/// same is drawn pale and grey, what differs in a colour that stands out,
/// stronger the more it differs. None when there is nothing to compare or
/// too much.
pub fn compare(old: Pixels<'_>, new: Pixels<'_>) -> Option<Compared> {
    let width = old.width.max(new.width);
    let height = old.height.max(new.height);
    let total = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    if total == 0 || total > MAX_PIXELS {
        return None;
    }
    let pixel = |image: &Pixels<'_>, x: u32, y: u32| -> Option<[u8; 4]> {
        if x >= image.width || y >= image.height {
            return None;
        }
        let at = (usize::try_from(y).ok()?)
            .checked_mul(usize::try_from(image.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?
            .checked_mul(4)?;
        let bytes = image.data.get(at..at.checked_add(4)?)?;
        Some([
            *bytes.first()?,
            *bytes.get(1)?,
            *bytes.get(2)?,
            *bytes.get(3)?,
        ])
    };
    let mut differing = 0usize;
    // Three bytes a pixel, blue first, as a bitmap stores them.
    let mut bgr = Vec::with_capacity(total.checked_mul(3)?);
    for y in 0..height {
        for x in 0..width {
            let (a, b) = (pixel(&old, x, y), pixel(&new, x, y));
            let delta = match (a, b) {
                (Some(a), Some(b)) => a
                    .iter()
                    .zip(b.iter())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0),
                (None, None) => 0,
                _ => 255,
            };
            if delta == 0 {
                // The same in both: a pale grey of what is there.
                let shade = a.map_or(255u16, |p| {
                    let grey = (u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3;
                    // Mostly white, so that differences are what is seen.
                    208 + grey * 47 / 255
                });
                let shade = u8::try_from(shade).unwrap_or(255);
                bgr.extend([shade, shade, shade]);
            } else {
                differing = differing.saturating_add(1);
                // Magenta, never fainter than can be seen.
                let strength = 96u16 + u16::from(delta) * 159 / 255;
                let fade = u8::try_from(255 - strength).unwrap_or(0);
                bgr.extend([255, fade, 255]);
            }
        }
    }
    Some(Compared {
        bmp: bmp(width, height, &bgr)?,
        differing,
        total,
        size: (width, height),
    })
}

/// A 24-bit bitmap file from rows of blue, green, red bytes, top row
/// first.
fn bmp(width: u32, height: u32, bgr: &[u8]) -> Option<Vec<u8>> {
    let row = usize::try_from(width).ok()?.checked_mul(3)?;
    // Rows are padded to four bytes.
    let padded = row.checked_add(3)? / 4 * 4;
    let pixels = padded.checked_mul(usize::try_from(height).ok()?)?;
    let size = u32::try_from(pixels.checked_add(54)?).ok()?;
    let mut out = Vec::with_capacity(pixels.checked_add(54)?);
    // File header: "BM", file size, two reserved words, offset of pixels.
    out.extend(b"BM");
    out.extend(size.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(54u32.to_le_bytes());
    // Info header: its size, width, height (positive: bottom row first),
    // one plane, 24 bits, no compression, then sizes and resolutions that
    // may be zero.
    out.extend(40u32.to_le_bytes());
    out.extend(i32::try_from(width).ok()?.to_le_bytes());
    out.extend(i32::try_from(height).ok()?.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend([0u8; 24]);
    for line in bgr.chunks(row.max(1)).rev() {
        out.extend(line);
        out.extend(std::iter::repeat_n(0u8, padded.saturating_sub(line.len())));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{
        Compared, Mode, Pixels, bmp, bordered, compare, format_of, human_size, own_size,
        with_border,
    };
    use gpui_kit::{ImageFormat, RenderImage};

    #[test]
    fn a_border_repeats_the_edge_it_lies_against() {
        // Two by two, each pixel a number of its own.
        let pixel = |n: u8| [n, n, n, 255];
        let data: Vec<u8> = [1, 2, 3, 4].into_iter().flat_map(pixel).collect();
        let out = with_border(&data, 2, 2).unwrap();
        let rows: Vec<Vec<u8>> = out
            .as_chunks::<16>()
            .0
            .iter()
            .map(|row| row.as_chunks::<4>().0.iter().map(|p| p[0]).collect())
            .collect();
        assert_eq!(
            rows,
            [[1, 1, 2, 2], [1, 1, 2, 2], [3, 3, 4, 4], [3, 3, 4, 4]]
        );
        assert!(out.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // One pixel is its own border all round.
        assert_eq!(with_border(&pixel(9), 1, 1).unwrap(), pixel(9).repeat(9));
        // Nothing, too few bytes, too many, and sizes beyond counting.
        assert_eq!(with_border(&[], 0, 0), None);
        assert_eq!(with_border(&data, 0, 2), None);
        assert_eq!(with_border(&data, 2, 3), None);
        assert_eq!(with_border(&data, 3, 1), None);
        assert_eq!(with_border(&data, u32::MAX, u32::MAX), None);
    }

    #[test]
    fn a_decoded_picture_is_drawn_from_a_copy_with_a_border() {
        let frame = |fill: u8, width: u32, height: u32| {
            let buffer =
                image::RgbaImage::from_pixel(width, height, image::Rgba([fill, fill, fill, 255]));
            image::Frame::new(buffer)
        };
        let shown = bordered(&RenderImage::new(vec![frame(7, 3, 2), frame(8, 3, 2)])).unwrap();
        assert_eq!((shown.width, shown.height), (3, 2));
        assert_eq!(shown.picture.frame_count(), 2);
        let size = shown.picture.size(1);
        assert_eq!((size.width.0, size.height.0), (5, 4));
        assert!(
            shown
                .picture
                .as_bytes(1)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] == 8)
        );
        // Without a frame there is nothing to draw.
        assert!(bordered(&RenderImage::new(Vec::<image::Frame>::new())).is_none());
    }

    #[test]
    fn a_picture_is_shown_at_its_own_size_and_smaller_only_to_fit() {
        // Small: a pixel for a pixel, however much room there is.
        assert_eq!(own_size(64, 64, 1400.0), Some((64.0, 64.0, 1.0)));
        assert_eq!(own_size(400, 240, 400.0), Some((400.0, 240.0, 1.0)));
        // Wider than its place: as wide as the place, proportions kept.
        assert_eq!(own_size(800, 200, 400.0), Some((400.0, 100.0, 0.5)));
        // No picture, no room, and numbers that are none.
        assert_eq!(own_size(0, 10, 100.0), None);
        assert_eq!(own_size(10, 0, 100.0), None);
        assert_eq!(own_size(10, 10, 0.0), None);
        assert_eq!(own_size(10, 10, -5.0), None);
        assert_eq!(own_size(10, 10, f32::NAN), None);
        assert_eq!(own_size(10, 10, f32::INFINITY), None);
        assert!(own_size(u32::MAX, 1, 1.0).is_some());
    }

    #[test]
    fn a_switch_lays_the_two_over_each_other_and_apart_again() {
        assert_eq!(Mode::Apart.switched(Mode::Onion), Mode::Onion);
        assert_eq!(Mode::Onion.switched(Mode::Onion), Mode::Apart);
        assert_eq!(Mode::Onion.switched(Mode::Swipe), Mode::Swipe);
        assert_eq!(Mode::Swipe.switched(Mode::Difference), Mode::Difference);
        assert_eq!(Mode::Difference.switched(Mode::Difference), Mode::Apart);
        // Overlay first; a word each.
        let words: Vec<&str> = Mode::OVER.iter().map(|(_, word)| word.text()).collect();
        assert_eq!(words, ["Overlay", "Swipe", "Difference"]);
    }

    #[test]
    fn pictures_are_told_by_their_extension() {
        assert_eq!(format_of("a/b/logo.PNG"), Some(ImageFormat::Png));
        assert_eq!(format_of("photo.jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(format_of("brand/mark.svg"), Some(ImageFormat::Svg));
        assert_eq!(format_of("archive.tar.gz"), None);
        assert_eq!(format_of("png"), None);
        assert_eq!(format_of(""), None);
        assert_eq!(format_of("trailing."), None);
    }

    #[test]
    fn sizes_read_like_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
    }

    fn image(width: u32, height: u32, fill: [u8; 4]) -> Vec<u8> {
        fill.iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect()
    }

    fn px(data: &[u8], width: u32, height: u32) -> Pixels<'_> {
        Pixels {
            data,
            width,
            height,
        }
    }

    #[test]
    fn the_same_picture_differs_nowhere() {
        let a = image(4, 3, [10, 20, 30, 255]);
        let Compared {
            differing,
            total,
            size,
            bmp,
        } = compare(px(&a, 4, 3), px(&a, 4, 3)).unwrap();
        assert_eq!((differing, total, size), (0, 12, (4, 3)));
        // A header and three rows of four pixels, each already a multiple
        // of four bytes long.
        assert_eq!(bmp.len(), 54 + 3 * 12);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize,
            bmp.len()
        );
    }

    #[test]
    fn changed_and_missing_pixels_are_counted() {
        let a = image(2, 2, [0, 0, 0, 255]);
        let mut b = a.clone();
        // One pixel a little different, one very.
        b[0] = 1;
        b[12..16].copy_from_slice(&[255, 255, 255, 255]);
        let got = compare(px(&a, 2, 2), px(&b, 2, 2)).unwrap();
        assert_eq!((got.differing, got.total), (2, 4));
        // Rows are stored bottom first and padded to four bytes: the last
        // pixel of the picture is the second of the first stored row.
        let rows = &got.bmp[54..];
        assert_eq!(rows.len(), 2 * 8);
        assert_eq!(&rows[3..6], &[255, 0, 255], "the strong difference");
        let faint = &rows[8..11];
        assert_eq!((faint[0], faint[2]), (255, 255));
        assert!(
            faint[1] > 100 && faint[1] < 255,
            "the faint one is still seen"
        );

        // Another size: what only one has is different.
        let wide = image(3, 2, [0, 0, 0, 255]);
        let got = compare(px(&a, 2, 2), px(&wide, 3, 2)).unwrap();
        assert_eq!((got.differing, got.total, got.size), (2, 6, (3, 2)));
    }

    #[test]
    fn nothing_and_too_much_are_not_compared() {
        let a = image(1, 1, [0, 0, 0, 255]);
        assert!(compare(px(&[], 0, 0), px(&[], 0, 0)).is_none());
        // A size that says more than the data holds: nothing is read past
        // the end, and pixels neither really has are not differences.
        let got = compare(px(&a, 1, 1), px(&a, 2, 2)).unwrap();
        assert_eq!((got.differing, got.total), (0, 4));
        assert!(compare(px(&a, 1, 1), px(&a, 100_000, 100_000)).is_none());
        assert!(compare(px(&a, u32::MAX, u32::MAX), px(&a, 1, 1)).is_none());
        assert!(bmp(0, 0, &[]).is_some());
        assert!(bmp(u32::MAX, u32::MAX, &[]).is_none());
    }
}
