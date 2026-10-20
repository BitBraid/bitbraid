//! A branch or tag label whose name is cut where it stands, widened to
//! its whole name while the pointer rests on it: drawn at its own place
//! over the graph and the columns right of it, so that nothing else
//! moves. It widens once the pointer has rested on the label as long as
//! the highlight of a branch waits ([`super::light::DELAY`]), and goes
//! back as soon as the pointer is on neither the label nor the widened
//! label. A label whose whole name fits does not change.
//!
//! The labels say when the pointer comes onto them and leaves them; a
//! label in its row and the same label in the column a row's labels
//! unfold into are one label (the row of the list and the label's place
//! among the row's). Whether a name is cut is measured as the label is
//! laid out: its whole name, shaped in the text style it is drawn in, is
//! wider than the room it was given.
//!
//! Sources: the toolkit's `WindowTextSystem::shape_line`, `canvas` and
//! `Context::defer_in` documentation.

use std::{cell::Cell, rc::Rc};

/// A label: the row of the list it is on, and its place among the row's
/// labels.
pub(super) type Key = (usize, usize);

/// Where a label is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// In its row: the first of the row's labels.
    Row,
    /// In the column of a row's labels, unfolded over the rows below.
    Column,
    /// Widened, over what is right of it.
    Wide,
}

/// Whether the name of the label measured last is cut, and which label
/// that was.
pub(super) type Measured = Rc<Cell<Option<(Key, bool)>>>;

/// The label the pointer rests on, and whether it is widened.
#[derive(Debug, Default)]
pub(super) struct Widen {
    /// The labels the pointer is on, each where it is drawn, as they said.
    under: Vec<(Key, Place)>,
    /// The pointer is on the widened label.
    on_wide: bool,
    /// The label asked to be widened: the one the pointer came onto last.
    asked: Option<Key>,
    /// How many times what is asked changed: a delay that ends after
    /// another change widens nothing.
    turn: u64,
    /// The widened label.
    shown: Option<Key>,
    /// Whether the asked label's name is cut, as it was last laid out.
    measured: Measured,
}

/// What a change of where the pointer is asks of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Then {
    /// Nothing more.
    Nothing,
    /// A frame, to measure the label asked for (and to draw the label
    /// widened before as it was), and once the delay is over
    /// [`Widen::due`] with this turn.
    Wait(u64),
    /// Once every change of what the pointer is on has been heard (the
    /// label it left may be under the one it came onto, which says so
    /// after it): [`Widen::settle`].
    Settle,
}

impl Widen {
    /// The widened label.
    pub(super) fn shown(&self) -> Option<Key> {
        self.shown
    }

    /// The label whose name is measured as it is laid out, and where it
    /// says whether the name is cut.
    pub(super) fn measuring(&self) -> Option<(Key, Measured)> {
        self.asked.map(|key| (key, self.measured.clone()))
    }

    /// The pointer came onto the label `key` drawn at `place`, or left it.
    /// Onto another label than the one asked: that one is asked, and the
    /// label widened before goes back at once.
    pub(super) fn point(&mut self, key: Key, place: Place, on: bool) -> Then {
        if !on {
            self.under.retain(|under| *under != (key, place));
            return self.left();
        }
        if !self.under.contains(&(key, place)) {
            self.under.push((key, place));
        }
        if self.asked == Some(key) {
            return Then::Nothing;
        }
        self.asked = Some(key);
        self.turn = self.turn.wrapping_add(1);
        if self.shown.is_some_and(|shown| shown != key) {
            self.shown = None;
            self.on_wide = false;
        }
        Then::Wait(self.turn)
    }

    /// The pointer came onto the widened label, or left it.
    pub(super) fn point_at_wide(&mut self, on: bool) -> Then {
        self.on_wide = on && self.shown.is_some();
        if on { Then::Nothing } else { self.left() }
    }

    fn left(&self) -> Then {
        if self.under.is_empty() && !self.on_wide {
            Then::Settle
        } else {
            Then::Nothing
        }
    }

    /// Every change of what the pointer is on has been heard: on no label
    /// and not on the widened one, nothing is asked or widened from now
    /// on. Whether the widened label went back.
    pub(super) fn settle(&mut self) -> bool {
        if !self.under.is_empty() || self.on_wide || self.asked.is_none() {
            return false;
        }
        self.asked = None;
        self.turn = self.turn.wrapping_add(1);
        self.shown.take().is_some()
    }

    /// The delay of `turn` is over: the label asked is widened when
    /// nothing else was asked since, the pointer is still on it and its
    /// name was cut where it was drawn last. Whether it was widened.
    pub(super) fn due(&mut self, turn: u64) -> bool {
        let Some(key) = self.asked.filter(|_| turn == self.turn) else {
            return false;
        };
        let still = self.under.iter().any(|(under, _)| *under == key);
        let cut = self.measured.get() == Some((key, true));
        if still && cut && self.shown != Some(key) {
            self.shown = Some(key);
            return true;
        }
        false
    }

    /// Nothing is asked or widened, at once: the rows are other rows now.
    pub(super) fn reset(&mut self) {
        self.under.clear();
        self.on_wide = false;
        self.asked = None;
        self.shown = None;
        self.measured.set(None);
        self.turn = self.turn.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{Place, Then, Widen};

    fn measured(widen: &Widen, cut: bool) {
        if let Some((key, said)) = widen.measuring() {
            said.set(Some((key, cut)));
        }
    }

    #[test]
    fn a_cut_label_is_widened_once_the_delay_is_over_and_the_pointer_stayed() {
        let mut widen = Widen::default();
        let Then::Wait(turn) = widen.point((3, 0), Place::Row, true) else {
            panic!("a label asked");
        };
        measured(&widen, true);
        assert_eq!(widen.shown(), None, "not before the delay");
        assert!(widen.due(turn));
        assert_eq!(widen.shown(), Some((3, 0)));
        // The pointer goes over to the widened label, which lies over the
        // label: the label says it left first; nothing goes back.
        assert_eq!(widen.point((3, 0), Place::Row, false), Then::Settle);
        assert_eq!(widen.point_at_wide(true), Then::Nothing);
        assert!(!widen.settle());
        assert_eq!(widen.shown(), Some((3, 0)));
        // Off it: back, once that is heard.
        assert_eq!(widen.point_at_wide(false), Then::Settle);
        assert!(widen.settle());
        assert_eq!(widen.shown(), None);
        assert!(widen.measuring().is_none());
    }

    #[test]
    fn a_label_that_fits_or_was_left_is_not_widened() {
        let mut widen = Widen::default();
        let Then::Wait(turn) = widen.point((1, 0), Place::Row, true) else {
            panic!("a label asked");
        };
        measured(&widen, false);
        assert!(!widen.due(turn), "its whole name fits");

        // Left before the delay is over.
        let mut widen = Widen::default();
        let Then::Wait(turn) = widen.point((1, 0), Place::Row, true) else {
            panic!("a label asked");
        };
        measured(&widen, true);
        widen.point((1, 0), Place::Row, false);
        assert!(widen.settle() || widen.shown().is_none());
        assert!(!widen.due(turn));

        // Another label asked meanwhile: the first one's delay widens
        // nothing, and the widened one goes back at once.
        let mut widen = Widen::default();
        let Then::Wait(first) = widen.point((1, 0), Place::Column, true) else {
            panic!("a label asked");
        };
        measured(&widen, true);
        assert!(widen.due(first));
        let Then::Wait(second) = widen.point((1, 1), Place::Column, true) else {
            panic!("another label asked");
        };
        assert_eq!(widen.shown(), None, "the first went back at once");
        assert!(!widen.due(first));
        measured(&widen, true);
        assert!(widen.due(second));
        assert_eq!(widen.shown(), Some((1, 1)));

        // The same label in its row and in the column over it is one: the
        // row's saying it left keeps the column's.
        let mut widen = Widen::default();
        widen.point((2, 0), Place::Row, true);
        assert_eq!(widen.point((2, 0), Place::Column, true), Then::Nothing);
        assert_eq!(widen.point((2, 0), Place::Row, false), Then::Nothing);
        widen.reset();
        assert!(widen.measuring().is_none() && widen.shown().is_none());
    }
}
