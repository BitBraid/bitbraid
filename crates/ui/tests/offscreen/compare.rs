//! What is stored of a frame, and how a frame is compared with it.
//!
//! A frame is drawn at two device pixels to the point. What is kept in the
//! repository is that frame averaged down four to one in each direction: a
//! quarter of the bytes of a picture at one pixel to the point, still
//! readable, and each stored pixel is the mean of sixteen drawn ones, which
//! takes the last bit of a rasteriser's rounding out of the comparison
//! (`AGENTS.md` section 3).

use image::{Rgba, RgbaImage};

/// Device pixels to one stored pixel, in each direction.
pub const REDUCTION: u32 = 4;

/// How far a channel of a stored pixel may be from the stored picture's.
/// Sixteen drawn pixels are averaged into one, so a renderer that rounds
/// the other way moves it by less than one step; a line one point wide
/// that goes missing moves it by half the line's contrast.
pub const CHANNEL_TOLERANCE: u8 = 3;

/// The frame as it is stored: averaged down [`REDUCTION`] to one, opaque.
pub fn reduce(frame: &RgbaImage) -> RgbaImage {
    let (width, height) = (frame.width() / REDUCTION, frame.height() / REDUCTION);
    let mut out = RgbaImage::new(width, height);
    let cell = u64::from(REDUCTION * REDUCTION);
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let mut sum = [0u64; 3];
        for dy in 0..REDUCTION {
            for dx in 0..REDUCTION {
                let source = frame.get_pixel(x * REDUCTION + dx, y * REDUCTION + dy).0;
                for (total, channel) in sum.iter_mut().zip(source) {
                    *total += u64::from(channel);
                }
            }
        }
        let mean = |total: u64| ((total + cell / 2) / cell).min(255) as u8;
        *pixel = Rgba([mean(sum[0]), mean(sum[1]), mean(sum[2]), 255]);
    }
    out
}

/// What comparing two stored-size pictures found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The two are not the same size: nothing else was compared.
    pub sizes: Option<((u32, u32), (u32, u32))>,
    /// Pixels further off than the tolerance.
    pub differing: usize,
    /// The largest distance of a channel.
    pub worst: u8,
}

impl Verdict {
    /// Whether the picture counts as the stored one: no pixel is further
    /// off than the tolerance. Not one is let through, since one glyph
    /// that changed is only a few dozen.
    pub fn same(&self) -> bool {
        self.sizes.is_none() && self.differing == 0
    }

    /// What is wrong, in a few words.
    pub fn said(&self) -> String {
        match self.sizes {
            Some((stored, drawn)) => format!(
                "the stored picture is {}x{} and the drawn one {}x{}",
                stored.0, stored.1, drawn.0, drawn.1
            ),
            None => format!(
                "{} pixels differ by more than {CHANNEL_TOLERANCE} (the furthest by {})",
                self.differing, self.worst
            ),
        }
    }
}

fn distance(a: Rgba<u8>, b: Rgba<u8>) -> u8 {
    a.0.iter()
        .zip(b.0)
        .take(3)
        .map(|(a, b)| a.abs_diff(b))
        .max()
        .unwrap_or(0)
}

/// Compare the stored picture with one drawn now, both at stored size.
pub fn compare(stored: &RgbaImage, drawn: &RgbaImage) -> Verdict {
    if stored.dimensions() != drawn.dimensions() {
        return Verdict {
            sizes: Some((stored.dimensions(), drawn.dimensions())),
            differing: 0,
            worst: 0,
        };
    }
    let (mut differing, mut worst) = (0usize, 0u8);
    for (a, b) in stored.pixels().zip(drawn.pixels()) {
        let far = distance(*a, *b);
        worst = worst.max(far);
        if far > CHANNEL_TOLERANCE {
            differing += 1;
        }
    }
    Verdict {
        sizes: None,
        differing,
        worst,
    }
}

/// A picture to look at when the two differ: the stored one, the drawn
/// one, and the drawn one dimmed with every pixel that differs marked,
/// side by side. Each of a different size is put at the top left of its
/// third.
pub fn difference(stored: &RgbaImage, drawn: &RgbaImage) -> RgbaImage {
    const GAP: u32 = 8;
    const MARK: Rgba<u8> = Rgba([255, 0, 200, 255]);
    let width = stored.width().max(drawn.width());
    let height = stored.height().max(drawn.height());
    let mut out = RgbaImage::from_pixel(width * 3 + GAP * 2, height, Rgba([90, 90, 90, 255]));
    for (x, y, pixel) in stored.enumerate_pixels() {
        out.put_pixel(x, y, *pixel);
    }
    for (x, y, pixel) in drawn.enumerate_pixels() {
        out.put_pixel(width + GAP + x, y, *pixel);
        let other = (x < stored.width() && y < stored.height()).then(|| *stored.get_pixel(x, y));
        let marked = match other {
            Some(other) if distance(other, *pixel) <= CHANNEL_TOLERANCE => {
                // Dimmed towards grey, so that the marks stand out in
                // either theme.
                let [r, g, b, _] = pixel.0;
                let dim = |c: u8| ((u16::from(c) + 2 * 128) / 3) as u8;
                Rgba([dim(r), dim(g), dim(b), 255])
            }
            _ => MARK,
        };
        out.put_pixel((width + GAP) * 2 + x, y, marked);
    }
    out
}

/// The comparison is held to what it is for: it must not see a renderer
/// rounding the other way, and it must see an icon that went blank and a
/// thin line that went missing.
pub fn self_test() {
    let background = Rgba([24, 27, 34, 255]);
    let mut frame = RgbaImage::from_pixel(64 * REDUCTION, 32 * REDUCTION, background);
    // A line one point wide (two device pixels) of low contrast, as the
    // border between two panels is.
    for y in 0..frame.height() {
        for x in 100..102 {
            frame.put_pixel(x, y, Rgba([44, 48, 58, 255]));
        }
    }
    let stored = reduce(&frame);
    assert_eq!(stored.dimensions(), (64, 32));
    assert!(compare(&stored, &stored).same());

    // Every drawn pixel one step off, as another rasteriser may have it.
    let mut rounded = frame.clone();
    for pixel in rounded.pixels_mut() {
        pixel.0[0] = pixel.0[0].saturating_add(1);
        pixel.0[2] = pixel.0[2].saturating_sub(1);
    }
    let verdict = compare(&stored, &reduce(&rounded));
    assert!(
        verdict.same(),
        "one step of rounding is no difference: {verdict:?}"
    );

    // The line gone.
    let mut without = frame.clone();
    for y in 0..without.height() {
        for x in 100..102 {
            without.put_pixel(x, y, background);
        }
    }
    let verdict = compare(&stored, &reduce(&without));
    assert!(!verdict.same(), "a border that went missing is seen");
    assert_eq!(verdict.differing, 32, "one stored pixel in each row");

    // An icon of sixteen points whose strokes went blank.
    let mut with_icon = frame.clone();
    for step in 0..32 {
        for wide in 0..3 {
            with_icon.put_pixel(20 + step, 20 + step + wide, Rgba([200, 205, 215, 255]));
        }
    }
    let verdict = compare(&reduce(&with_icon), &stored);
    assert!(!verdict.same() && verdict.differing >= 8, "{verdict:?}");

    // Another size is said as such, and the picture to look at has room
    // for both.
    let smaller = RgbaImage::from_pixel(60, 30, background);
    let verdict = compare(&stored, &smaller);
    assert!(!verdict.same() && verdict.sizes.is_some());
    assert!(verdict.said().contains("64x32") && verdict.said().contains("60x30"));
    let shown = difference(&stored, &smaller);
    assert_eq!(shown.dimensions(), (64 * 3 + 16, 32));
    // The third panel marks the pixels that differ, and only those.
    let shown = difference(&stored, &reduce(&without));
    let marked = shown
        .pixels()
        .filter(|pixel| pixel.0 == [255, 0, 200, 255])
        .count();
    assert_eq!(marked, 32);
}
