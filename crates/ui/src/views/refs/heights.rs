//! How the open sections of the references panel share its height: every
//! open list wants a height of its own (what it was dragged to, else five
//! rows and a half); the last open one takes what is left, however few
//! rows it has; none is lower than four rows and a half. Where the panel
//! is too low for that, the lists give from the bottom up, each down to
//! the least, and the sections as a whole scroll.
//!
//! Dragging the edge over a heading grows the list on one side of it and
//! takes from the lists on the other side, the nearest first, each down to
//! the least. A list that is closed gives its room to the open list under
//! it; opened again, it takes it back from there.
//!
//! Everything here works on the open lists in the order they are shown,
//! as plain numbers, so that it can be tested without a window.
//!
//! Sources: none (original design).

use super::ROW_HEIGHT;

/// The least an open list is high: four rows and a half.
pub(super) const LEAST_LIST: f32 = ROW_HEIGHT * 4.5;
/// What an open list wants that was never dragged: five rows and a half.
pub(super) const FIRST_LIST: f32 = ROW_HEIGHT * 5.5;
/// The most a list is wanted at: what is remembered comes from a file
/// anyone can edit.
const MOST_LIST: f32 = 100_000.0;

/// A height that was asked for, made one that can be laid out with.
fn sane(want: Option<f32>) -> f32 {
    match want {
        Some(want) if want.is_finite() => want.clamp(LEAST_LIST, MOST_LIST),
        _ => FIRST_LIST,
    }
}

/// How high each open list is, top to bottom, given what each wants
/// (`None`: what a list wants that was never dragged) and the room all of
/// them have together. The last takes what the others leave; where that
/// is less than the least, the others give from the bottom up, and what
/// is still missing makes the sections taller than the room: they scroll.
pub(super) fn laid_out(room: f32, wants: &[Option<f32>]) -> Vec<f32> {
    let room = if room.is_finite() { room.max(0.0) } else { 0.0 };
    let Some((_, before_last)) = wants.split_last() else {
        return Vec::new();
    };
    let mut heights: Vec<f32> = before_last.iter().map(|want| sane(*want)).collect();
    let taken: f32 = heights.iter().sum();
    let mut missing = (LEAST_LIST - (room - taken)).max(0.0);
    for height in heights.iter_mut().rev() {
        if missing <= 0.0 {
            break;
        }
        let given = missing.min(*height - LEAST_LIST);
        *height -= given;
        missing -= given;
    }
    let taken: f32 = heights.iter().sum();
    heights.push((room - taken).max(LEAST_LIST));
    heights
}

/// The heights after the edge over the heading under list `at - 1` (and
/// over list `at`, or the headings before it) was dragged `by` down (up
/// when less than nought). Down, the list above the edge grows and the
/// lists under it give, the nearest first; up, the lists above give, the
/// nearest first, and the list under the edge grows. None gives under
/// the least, so a drag goes only as far as that allows. The heights are
/// as they are when `at` is not an edge between two lists.
pub(super) fn dragged(heights: &[f32], at: usize, by: f32) -> Vec<f32> {
    let mut heights = heights.to_vec();
    if at == 0 || at >= heights.len() || !by.is_finite() {
        return heights;
    }
    // Who gives, in the order they give, and who grows.
    let (givers, grows): (Vec<usize>, usize) = if by > 0.0 {
        ((at..heights.len()).collect(), at - 1)
    } else {
        ((0..at).rev().collect(), at)
    };
    let mut wanted = by.abs();
    let mut moved = 0.0;
    for giver in givers {
        let Some(height) = heights.get_mut(giver) else {
            continue;
        };
        let given = wanted.min((*height - LEAST_LIST).max(0.0));
        *height -= given;
        wanted -= given;
        moved += given;
    }
    if let Some(height) = heights.get_mut(grows) {
        *height += moved;
    }
    heights
}

/// What a list that is closed leaves to the others: of the open lists as
/// they are laid out before (`heights`), the one at `at` closes. Its room
/// goes to the open list that was under it, when that one is not the last
/// (which takes what is left anyway): that list, by its place among the
/// lists that stay open, and what it is to want from now on.
pub(super) fn on_close(heights: &[f32], at: usize) -> Option<(usize, f32)> {
    let closing = *heights.get(at)?;
    let under = *heights.get(at + 1)?;
    // After the close, the one under it is at `at`; the last is at
    // `len - 2`.
    (at + 2 < heights.len()).then_some((at, under + closing))
}

/// What a list that is opened takes from the others: of the open lists
/// as they are laid out before (`heights`), the opened one comes in at
/// `at` and wants `want`. It takes that from the open list that will be
/// under it, down to the least, when that one is not the last (which
/// gives what it has to anyway): that list, by its place among the lists
/// before the open, and what it is to want from now on.
pub(super) fn on_open(heights: &[f32], at: usize, want: Option<f32>) -> Option<(usize, f32)> {
    let under = *heights.get(at)?;
    (at + 1 < heights.len()).then(|| (at, (under - sane(want)).max(LEAST_LIST)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{FIRST_LIST, LEAST_LIST, dragged, laid_out, on_close, on_open};

    #[test]
    fn the_measures_are_kept() {
        assert_eq!((LEAST_LIST, FIRST_LIST), (108.0, 132.0));
    }

    #[test]
    fn the_last_list_takes_what_the_others_leave() {
        assert_eq!(laid_out(561.0, &[None]), [561.0]);
        assert_eq!(laid_out(561.0, &[None, None]), [132.0, 429.0]);
        assert_eq!(
            laid_out(561.0, &[Some(200.0), None, None]),
            [200.0, 132.0, 229.0]
        );
        // Where the others would leave it less than the least, they give.
        assert_eq!(laid_out(561.0, &[Some(500.0), None]), [453.0, 108.0]);
        assert!(laid_out(561.0, &[]).is_empty());
    }

    #[test]
    fn too_little_room_takes_from_the_bottom_up_and_then_scrolls() {
        // The lists before the last give from the bottom up, each down to
        // the least.
        assert_eq!(
            laid_out(561.0, &[Some(200.0), Some(253.0), None, None]),
            [200.0, 145.0, 108.0, 108.0]
        );
        // With nothing left to give, every list has the least and the
        // sections are taller than the room.
        let squeezed = laid_out(201.0, &[Some(200.0), Some(253.0), None, None]);
        assert_eq!(squeezed, [108.0; 4]);
        assert!(squeezed.iter().sum::<f32>() > 201.0);
    }

    #[test]
    fn what_was_remembered_is_held_to_sane_heights() {
        let hostile = [
            Some(f32::NAN),
            Some(-5.0),
            Some(f32::INFINITY),
            Some(1e30),
            None,
        ];
        let heights = laid_out(f32::NAN, &hostile);
        assert_eq!(heights.len(), 5);
        assert!(heights.iter().all(|h| h.is_finite() && *h >= LEAST_LIST));
        assert_eq!(laid_out(-1.0, &[None]), [LEAST_LIST]);
    }

    #[test]
    fn a_drag_takes_from_the_far_side_nearest_first() {
        // Down: the list above grows, those under it give in turn.
        assert_eq!(
            dragged(&[132.0, 132.0, 297.0], 1, 100.0),
            [232.0, 108.0, 221.0]
        );
        // Up: the lists above give in turn, the list under the edge grows.
        assert_eq!(
            dragged(&[148.0, 192.0, 221.0], 2, -500.0),
            [108.0, 108.0, 345.0]
        );
        assert_eq!(
            dragged(&[232.0, 108.0, 221.0], 1, -500.0),
            [108.0, 232.0, 221.0]
        );
        // As far as the least allows, and no further.
        assert_eq!(
            dragged(&[268.0, 185.0, 108.0], 1, 500.0),
            [345.0, 108.0, 108.0]
        );
        // No edge between two lists there: nothing moves.
        let heights = [132.0, 429.0];
        assert_eq!(dragged(&heights, 0, 50.0), heights);
        assert_eq!(dragged(&heights, 2, 50.0), heights);
        assert_eq!(dragged(&heights, 1, f32::NAN), heights);
    }

    /// The open lists, their wants, and what is done to them, as the panel
    /// does it, for checking whole sequences of steps.
    struct Panel {
        order: Vec<&'static str>,
        open: Vec<&'static str>,
        wants: HashMap<&'static str, f32>,
        room: f32,
    }

    impl Panel {
        fn new(room: f32) -> Self {
            Self {
                order: vec!["local", "remote", "worktrees", "stashes", "tags"],
                open: vec!["local"],
                wants: HashMap::new(),
                room,
            }
        }
        fn heights(&self) -> Vec<f32> {
            let wants: Vec<Option<f32>> = self
                .open
                .iter()
                .map(|key| self.wants.get(key).copied())
                .collect();
            laid_out(self.room, &wants)
        }
        fn shown(&self) -> Vec<(&'static str, f32)> {
            self.open.iter().copied().zip(self.heights()).collect()
        }
        fn toggle(&mut self, key: &'static str) {
            let heights = self.heights();
            if let Some(at) = self.open.iter().position(|open| *open == key) {
                let under = on_close(&heights, at);
                self.open.remove(at);
                if let Some((at, want)) = under {
                    self.wants.insert(self.open[at], want);
                }
            } else {
                let at = self
                    .open
                    .iter()
                    .filter(|open| {
                        let place = |k: &&str| self.order.iter().position(|o| o == k);
                        place(open) < place(&key)
                    })
                    .count();
                if let Some((under, want)) = on_open(&heights, at, self.wants.get(key).copied()) {
                    self.wants.insert(self.open[under], want);
                }
                self.open.insert(at, key);
            }
        }
        fn drag(&mut self, at: usize, by: f32) {
            let heights = dragged(&self.heights(), at, by);
            for (key, height) in self.open.iter().zip(heights) {
                self.wants.insert(key, height);
            }
        }
    }

    #[test]
    fn opening_closing_and_dragging_share_the_height() {
        // A panel whose lists have 561 together, as at 1920x1080.
        let mut panel = Panel::new(561.0);
        assert_eq!(panel.shown(), [("local", 561.0)]);
        panel.toggle("remote");
        assert_eq!(panel.shown(), [("local", 132.0), ("remote", 429.0)]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [132.0, 132.0, 297.0]);
        // The first of three closed: its room goes to the one under it.
        panel.toggle("local");
        assert_eq!(panel.heights(), [264.0, 297.0]);
        panel.toggle("local");
        assert_eq!(panel.heights(), [132.0, 132.0, 297.0]);
        // Opened between two for the first time: from the last.
        panel.toggle("worktrees");
        assert_eq!(panel.heights(), [132.0, 132.0, 132.0, 165.0]);
        panel.toggle("worktrees");
        assert_eq!(panel.heights(), [132.0, 132.0, 297.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [132.0, 429.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [132.0, 132.0, 297.0]);

        // Dragged: kept through closing and opening again.
        panel.drag(1, 100.0);
        assert_eq!(panel.heights(), [232.0, 108.0, 221.0]);
        panel.drag(1, -500.0);
        assert_eq!(panel.heights(), [108.0, 232.0, 221.0]);
        panel.drag(1, 40.0);
        assert_eq!(panel.heights(), [148.0, 192.0, 221.0]);
        panel.drag(2, -500.0);
        assert_eq!(panel.heights(), [108.0, 108.0, 345.0]);
        panel.drag(2, 500.0);
        assert_eq!(panel.heights(), [108.0, 345.0, 108.0]);
        panel.drag(1, 160.0);
        assert_eq!(panel.heights(), [268.0, 185.0, 108.0]);
        panel.drag(1, 500.0);
        assert_eq!(panel.heights(), [345.0, 108.0, 108.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [345.0, 216.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [345.0, 108.0, 108.0]);
        panel.drag(1, -145.0);
        assert_eq!(panel.heights(), [200.0, 253.0, 108.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [200.0, 361.0]);
        panel.toggle("tags");
        assert_eq!(panel.heights(), [200.0, 253.0, 108.0]);
        // A fourth: too little room, the lists give from the bottom up.
        panel.toggle("stashes");
        assert_eq!(panel.heights(), [200.0, 145.0, 108.0, 108.0]);

        // A smaller window squeezes them for as long as it lasts.
        panel.room = 201.0;
        assert_eq!(panel.heights(), [108.0; 4]);
        panel.room = 561.0;
        assert_eq!(panel.heights(), [200.0, 145.0, 108.0, 108.0]);
    }

    #[test]
    fn closing_and_opening_hold_nothing_that_is_not_there() {
        assert_eq!(on_close(&[], 0), None);
        assert_eq!(on_close(&[132.0], 0), None);
        assert_eq!(
            on_close(&[132.0, 429.0], 0),
            None,
            "the last takes the rest"
        );
        assert_eq!(on_close(&[132.0, 132.0, 297.0], 7), None);
        assert_eq!(on_open(&[], 0, None), None);
        assert_eq!(on_open(&[561.0], 0, None), None);
        assert_eq!(on_open(&[132.0, 429.0], 9, None), None);
        assert_eq!(
            on_open(&[264.0, 297.0], 0, Some(f32::NAN)),
            Some((0, 132.0))
        );
    }
}
