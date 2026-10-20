//! The zoom: one scale everything of the interface is drawn at.
//!
//! The whole window grows with it: the bars, the rows, the panels, the
//! icons, the text. The toolkit sizes its own parts by its em
//! (`Window::set_rem_size`, which the window sets from the zoom); every
//! measure of ours is a number of points, and [`px`] turns it into the
//! toolkit's pixels at the zoom in force. A module that draws imports this
//! `px` in place of the toolkit's, and is then drawn at the zoom without
//! anything else of it knowing.
//!
//! What comes back from the toolkit in its pixels (where the pointer is,
//! how large something was laid out, how far a list is scrolled) is turned
//! back into points with [`unpx`] before it is held against measures of
//! ours. At 100 % both are the toolkit's own, to the bit.
//!
//! Sources: gpui's `Pixels` and `Window::set_rem_size` documentation.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::{Pixels, Size, size};

/// The smallest and largest zoom there is.
pub const LEAST: f32 = 0.5;
pub const MOST: f32 = 3.0;

thread_local! {
    /// The zoom in force: the window sets it before it is drawn. Views are
    /// drawn on the one thread the window has.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}

/// The zoom a number is read at: what was set, kept within its bounds; one
/// that is no number is 100 %.
fn bounded(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(LEAST, MOST)
    } else {
        1.0
    }
}

/// Set the zoom everything is drawn at from now on.
pub fn set(zoom: f32) {
    SCALE.with(|scale| scale.set(bounded(zoom)));
}

/// The zoom in force.
pub fn scale() -> f32 {
    SCALE.with(Cell::get)
}

/// A measure of ours, in points, as the toolkit's pixels at the zoom.
pub fn px(points: f32) -> Pixels {
    gpui_kit::px(points * scale())
}

/// The toolkit's pixels as a measure of ours, in points.
pub fn unpx(pixels: Pixels) -> f32 {
    f32::from(pixels) / scale()
}

/// Sizes of a list's rows, in the toolkit's pixels.
type Rows = Rc<Vec<Size<Pixels>>>;

/// How high and wide each row of a list is, in the toolkit's pixels, as a
/// virtual list is given them: made at the zoom in force when they were
/// made, and given at the zoom in force whenever they are asked for. The
/// rows of every list grow and shrink with the zoom at once, however it
/// was changed. Made over from the sizes as they were made, never from
/// sizes made over before, so that zooming back and forth leaves nothing
/// off.
#[derive(Debug, Clone)]
pub struct Sizes {
    /// As they were made, and the zoom they were made at.
    made: Rows,
    at: f32,
    /// The last made over, and the zoom it is for.
    over: RefCell<Option<(f32, Rows)>>,
}

/// The same sizes, made at the same zoom.
impl PartialEq for Sizes {
    fn eq(&self, other: &Self) -> bool {
        self.made == other.made && self.at.to_bits() == other.at.to_bits()
    }
}

impl Default for Sizes {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Sizes {
    /// Sizes made at the zoom in force.
    pub fn new(sizes: Vec<Size<Pixels>>) -> Self {
        Self {
            made: Rc::new(sizes),
            at: scale(),
            over: RefCell::new(None),
        }
    }

    pub fn len(&self) -> usize {
        self.made.len()
    }

    pub fn is_empty(&self) -> bool {
        self.made.is_empty()
    }

    /// The sizes at the zoom in force.
    pub fn get(&self) -> Rows {
        let now = scale();
        let same = |a: f32, b: f32| (a - b).abs() < f32::EPSILON;
        if same(now, self.at) || !self.at.is_finite() || self.at <= 0.0 {
            return self.made.clone();
        }
        if let Ok(over) = self.over.try_borrow()
            && let Some((zoom, sizes)) = over.as_ref()
            && same(*zoom, now)
        {
            return sizes.clone();
        }
        let by = now / self.at;
        let sizes: Rows = Rc::new(
            self.made
                .iter()
                .map(|made| size(made.width * by, made.height * by))
                .collect(),
        );
        if let Ok(mut over) = self.over.try_borrow_mut() {
            *over = Some((now, sizes.clone()));
        }
        sizes
    }
}

#[cfg(test)]
mod tests {
    use super::{px, scale, set, unpx};

    #[test]
    fn a_measure_grows_with_the_zoom_and_comes_back_as_it_was() {
        set(1.0);
        assert_eq!(px(28.0), gpui_kit::px(28.0), "100 % is the toolkit's own");
        set(1.2);
        assert!((f32::from(px(28.0)) - 33.6).abs() < 1e-4);
        assert!((unpx(px(28.0)) - 28.0).abs() < 1e-4);
        // Kept within its bounds, and a zoom that is no number is 100 %.
        set(10.0);
        assert_eq!(scale(), 3.0);
        set(0.0);
        assert_eq!(scale(), 0.5);
        set(f32::NAN);
        assert_eq!(scale(), 1.0);
        set(1.0);
    }

    #[test]
    fn a_lists_sizes_follow_the_zoom_and_come_back_as_they_were() {
        use super::Sizes;
        use gpui_kit::size;
        set(1.0);
        let sizes = Sizes::new(vec![size(px(0.0), px(28.0)); 3]);
        assert_eq!(sizes.get()[2].height, gpui_kit::px(28.0));
        set(1.5);
        assert_eq!(sizes.get()[2].height, gpui_kit::px(42.0));
        assert_eq!(sizes.len(), 3);
        // Back and forth many times: nothing left off.
        for _ in 0..100 {
            set(1.1);
            let _ = sizes.get();
            set(0.7);
            let _ = sizes.get();
        }
        set(1.0);
        assert_eq!(sizes.get()[0].height, gpui_kit::px(28.0));
        // Made at 120 %: as many pixels as 28 points are there.
        set(1.2);
        let made = Sizes::new(vec![size(px(0.0), px(28.0))]);
        set(1.0);
        assert!((f32::from(made.get()[0].height) - 28.0).abs() < 1e-4);
        assert!(Sizes::default().is_empty());
    }
}
