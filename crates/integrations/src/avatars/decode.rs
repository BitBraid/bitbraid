//! A picture out of bytes that came over the network.
//!
//! The bytes are not trusted: what they are is told from their first few
//! (never from what the sender called them), only four kinds are read, the
//! size the file claims is looked at before anything is decoded, and the
//! decoder works within a limit on memory. Whatever goes wrong, a panic in
//! the decoder included, the answer is "no picture".
//!
//! Sources: the documentation of the `image` crate (`ImageReader`,
//! `Limits`, `DynamicImage::resize_to_fill`); the first bytes of each kind
//! as their specifications give them (PNG §5.2, JPEG's start-of-image
//! marker, the GIF87a and GIF89a headers, the RIFF header of WebP).

use std::io::Cursor;

use image::{ImageFormat, ImageReader, imageops::FilterType};

/// Largest file taken for a picture, in bytes.
pub const MAX_BYTES: usize = 512 * 1024;
/// Largest width or height a picture may claim, in pixels.
pub const MAX_SIDE: u32 = 1024;
/// Most memory the decoder may ask for.
const MAX_ALLOC: u64 = 16 * 1024 * 1024;
/// Largest side a picture is handed out with.
pub const MAX_OUT: u32 = 256;
/// Most sizes handed out of one picture.
const MAX_SIZES: usize = 4;

/// A square picture: `side` pixels each way, four bytes a pixel (red,
/// green, blue, alpha), row after row.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    /// Pixels each way.
    pub side: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bitmap")
            .field("side", &self.side)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// What kind of picture the bytes begin as, of the kinds that are read.
fn kind(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

fn decode_within_limits(bytes: &[u8], sides: &[u32]) -> Option<Vec<Bitmap>> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES || sides.is_empty() || sides.len() > MAX_SIZES {
        return None;
    }
    if sides.iter().any(|side| *side == 0 || *side > MAX_OUT) {
        return None;
    }
    let kind = kind(bytes)?;
    // What the file says its size is, before any pixel is decoded.
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), kind)
        .into_dimensions()
        .ok()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), kind);
    reader.limits(limits());
    let picture = reader.decode().ok()?;
    let mut bitmaps = Vec::with_capacity(sides.len());
    for side in sides {
        // The middle square, when it is not one already.
        let scaled = picture
            .resize_to_fill(*side, *side, FilterType::CatmullRom)
            .into_rgba8();
        if scaled.dimensions() != (*side, *side) {
            return None;
        }
        bitmaps.push(Bitmap {
            side: *side,
            rgba: scaled.into_raw(),
        });
    }
    Some(bitmaps)
}

/// Decode a picture and scale it to each of `sides`. None for anything
/// that is not a picture of a kind and a size that is taken.
pub fn decode(bytes: &[u8], sides: &[u32]) -> Option<Vec<Bitmap>> {
    // The decoders are other people's code fed with other people's bytes:
    // should one of them give up in the middle, that is no picture, not
    // the end of the thread that asked.
    std::panic::catch_unwind(|| decode_within_limits(bytes, sides))
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let picture = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x * 255 / width.max(1)) as u8,
                (y * 255 / height.max(1)) as u8,
                90,
                255,
            ])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let picture = image::RgbImage::from_fn(40, 40, |x, _| image::Rgb([x as u8 * 6, 80, 160]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(picture)
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn a_picture_is_scaled_to_each_size_asked_for() {
        let bitmaps = decode(&png(64, 64), &[36, 64]).unwrap();
        assert_eq!(bitmaps.len(), 2);
        assert_eq!((bitmaps[0].side, bitmaps[0].rgba.len()), (36, 36 * 36 * 4));
        assert_eq!((bitmaps[1].side, bitmaps[1].rgba.len()), (64, 64 * 64 * 4));
        // Left to right it goes from no red to red: still the picture.
        let row = &bitmaps[1].rgba[..64 * 4];
        assert!(
            row[0] < 20 && row[63 * 4] > 230,
            "{} {}",
            row[0],
            row[63 * 4]
        );
        assert!(
            bitmaps[1]
                .rgba
                .iter()
                .skip(3)
                .step_by(4)
                .all(|alpha| *alpha == 255)
        );
    }

    #[test]
    fn each_kind_that_is_read_is_read() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::WebP,
        ] {
            let bitmaps = decode(&encoded(format), &[32]);
            assert_eq!(bitmaps.map(|b| b.len()), Some(1), "{format:?}");
        }
    }

    #[test]
    fn a_picture_that_is_not_square_gives_its_middle() {
        let bitmaps = decode(&png(120, 40), &[40]).unwrap();
        // The middle third of the red ramp: neither its start nor its end.
        let row = &bitmaps[0].rgba[..40 * 4];
        assert!(
            row[0] > 60 && row[39 * 4] < 200,
            "{} {}",
            row[0],
            row[39 * 4]
        );
    }

    #[test]
    fn what_is_no_picture_gives_none() {
        let good = png(64, 64);
        assert!(decode(&[], &[32]).is_none());
        assert!(decode(b"<html>not found</html>", &[32]).is_none());
        assert!(decode(&[0u8; 4096], &[32]).is_none());
        // A kind the decoder would know, and that is not taken.
        let mut bitmap = b"BM".to_vec();
        bitmap.extend_from_slice(&[0u8; 60]);
        assert!(decode(&bitmap, &[32]).is_none());
        assert!(decode(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", &[32]).is_none());
        // The right start and then nothing, or nonsense.
        assert!(decode(b"\x89PNG\r\n\x1a\n", &[32]).is_none());
        assert!(decode(&good[..good.len() / 2], &[32]).is_none());
        let mut broken = good.clone();
        for byte in broken.iter_mut().skip(40) {
            *byte = byte.wrapping_mul(31).wrapping_add(7);
        }
        assert!(decode(&broken, &[32]).is_none());
        assert!(decode(b"\xff\xd8\xff\xe0garbage after the marker", &[32]).is_none());
        assert!(decode(b"GIF89a\xff\xff\xff\xff", &[32]).is_none());
        assert!(decode(b"RIFF\x00\x00\x00\x00WEBPVP8 ", &[32]).is_none());
        // Sizes that are not handed out.
        assert!(decode(&good, &[]).is_none());
        assert!(decode(&good, &[0]).is_none());
        assert!(decode(&good, &[MAX_OUT + 1]).is_none());
        assert!(decode(&good, &[8, 16, 32, 64, 128]).is_none());
    }

    #[test]
    fn a_picture_too_large_is_not_decoded() {
        // One side larger than is taken.
        let wide = png(MAX_SIDE + 1, 8);
        assert!(wide.len() < MAX_BYTES);
        assert!(decode(&wide, &[32]).is_none());
        assert!(decode(&png(8, MAX_SIDE + 1), &[32]).is_none());
        // A header that claims a size the data does not have: the claim
        // is what is judged, before anything is decoded.
        let mut claims = png(16, 16);
        // IHDR's width is at bytes 16..20, big-endian.
        claims[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        assert!(decode(&claims, &[32]).is_none());
        // A file of more bytes than a picture may have.
        let mut long = png(16, 16);
        long.resize(MAX_BYTES + 1, 0);
        assert!(decode(&long, &[32]).is_none());
        // The largest that is taken is taken.
        assert!(decode(&png(MAX_SIDE, 4), &[16]).is_some());
    }

    #[test]
    fn every_cut_of_a_picture_is_survived() {
        // Not one length of a good file, and no byte of it turned over,
        // brings the decoder down.
        let good = png(24, 24);
        for end in 0..good.len() {
            let _ = decode(&good[..end], &[16]);
        }
        for at in 0..good.len() {
            let mut bent = good.clone();
            bent[at] ^= 0xff;
            let _ = decode(&bent, &[16]);
        }
        let jpeg = encoded(ImageFormat::Jpeg);
        for end in (0..jpeg.len()).step_by(7) {
            let _ = decode(&jpeg[..end], &[16]);
        }
    }
}
