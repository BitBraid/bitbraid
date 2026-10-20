//! The highlight of a branch's own run of commits in time:
//! when it is lit, and how the rows outside it step back.
//!
//! What the pointer rests on asks for a run: the labels of a row, or a
//! branch in the references panel. What is asked is lit only once it has
//! been asked for [`DELAY`] without a change, and that holds for nothing
//! as well: the light is put out, or moves to another run, only a delay
//! after the pointer left. A pass over several labels then lights none
//! of them, and a move from one label to the next does not bring every
//! row back in between.
//!
//! When what is lit changes, each row goes from how far it stepped back to
//! how far it steps back now over [`FADE`], on the scheduler's clock (which
//! the off-screen tests move by hand), while the view asks for a frame
//! after each; with reduced motion at once. A change cannot come while
//! another is under way: changes are a delay apart, and the fade is no
//! longer than the delay; were it to, the rows would go on from where the
//! last change was going.
//!
//! Sources: the toolkit's `BackgroundExecutor::now`, `timer` and
//! `Window::request_animation_frame` documentation.

use std::{
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

/// How long the pointer rests on what asks for a run before it is lit:
/// a tenth of a second, about the longest a response still reads as at
/// once, and longer than the pointer takes to cross a label of 22 points
/// on its way to somewhere else.
pub(super) const DELAY: Duration = Duration::from_millis(100);

/// How long the rows take to step back, or to come forward again: quick,
/// and no longer than [`DELAY`], so that one change is over before the
/// next can begin.
pub(super) const FADE: Duration = Duration::from_millis(100);

/// A run of commits: the rows that stay as they are while it is lit.
pub(super) type Run = Rc<HashSet<usize>>;

/// The highlight as it is lit and as it is drawn.
#[derive(Debug, Default)]
pub(super) struct Light {
    /// The row it is lit for, with its run; none while nothing is lit.
    lit: Option<(usize, Run)>,
    /// While the rows still go from what was lit before to what is lit
    /// now: what that was (none: nothing), and when the change began.
    fading: Option<(Option<Run>, Instant)>,
    /// How far that change has gone, from 0 to 1, as of the last frame.
    progress: f32,
    /// The row whose labels the pointer is on.
    on_labels: Option<usize>,
    /// How many times what is asked changed: a change is lit only when no
    /// other came after it.
    asked: u64,
}

/// How far a row steps back while `run` is lit: not at all when it is in
/// the run or when nothing is lit, all the way otherwise.
fn back(run: Option<&HashSet<usize>>, row: usize) -> f32 {
    match run {
        Some(run) if !run.contains(&row) => 1.0,
        _ => 0.0,
    }
}

impl Light {
    /// The row it is lit for and its run.
    pub(super) fn lit(&self) -> Option<&(usize, Run)> {
        self.lit.as_ref()
    }

    /// The row whose labels the pointer is on.
    pub(super) fn on_labels(&self) -> Option<usize> {
        self.on_labels
    }

    /// The pointer came onto the labels of a row, or left them: whether
    /// that changes what they ask for.
    pub(super) fn point_at_labels(&mut self, row: Option<usize>) -> bool {
        if self.on_labels == row {
            return false;
        }
        self.on_labels = row;
        true
    }

    /// What is asked for changed: the turn that is lit when its delay is
    /// over and nothing changed meanwhile.
    pub(super) fn ask(&mut self) -> u64 {
        self.asked = self.asked.wrapping_add(1);
        self.asked
    }

    /// Whether `turn` is still the last change of what is asked.
    pub(super) fn is_turn(&self, turn: u64) -> bool {
        self.asked == turn
    }

    /// Light `lit` from `now` on: the rows begin to go there from where
    /// they are. The same row as is lit changes nothing.
    pub(super) fn light(&mut self, lit: Option<(usize, Run)>, now: Instant) {
        if self.lit.as_ref().map(|(row, _)| *row) == lit.as_ref().map(|(row, _)| *row) {
            return;
        }
        let before = std::mem::replace(&mut self.lit, lit).map(|(_, run)| run);
        self.fading = Some((before, now));
        self.progress = 0.0;
    }

    /// Nothing is lit or asked, at once: the rows it was lit for are
    /// other rows now. A turn that is still waiting is not lit.
    pub(super) fn reset(&mut self) {
        self.lit = None;
        self.fading = None;
        self.progress = 1.0;
        self.on_labels = None;
        self.ask();
    }

    /// Bring the fade to `now` (to its end at once with reduced motion):
    /// whether it is still under way, so that another frame is wanted.
    pub(super) fn advance(&mut self, now: Instant, reduce_motion: bool) -> bool {
        let progress = self.progress_at(now, reduce_motion);
        self.progress = progress;
        if progress >= 1.0 {
            self.fading = None;
            return false;
        }
        true
    }

    /// How far the change of what is lit has gone at `now`, from 0 to 1;
    /// 1 when none is under way.
    pub(super) fn progress_at(&self, now: Instant, reduce_motion: bool) -> f32 {
        match &self.fading {
            None => 1.0,
            Some(_) if reduce_motion => 1.0,
            Some((_, since)) => {
                let gone = now.saturating_duration_since(*since).as_secs_f32();
                let whole = FADE.as_secs_f32();
                if whole > 0.0 && gone.is_finite() {
                    (gone / whole).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            }
        }
    }

    /// How far the row `row` steps back as of the last frame, from 0 (as
    /// it is) to 1 (as far as it goes).
    pub(super) fn back(&self, row: usize) -> f32 {
        let now = back(self.lit.as_ref().map(|(_, run)| &**run), row);
        match &self.fading {
            None => now,
            Some((before, _)) => {
                let before = back(before.as_deref(), row);
                before + (now - before) * self.progress.clamp(0.0, 1.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, rc::Rc, time::Duration};

    use super::{FADE, Light};

    fn run(rows: &[usize]) -> super::Run {
        Rc::new(rows.iter().copied().collect::<HashSet<usize>>())
    }

    #[test]
    fn rows_outside_a_lit_run_step_back_over_the_fade() {
        let start = std::time::Instant::now();
        let mut light = Light::default();
        assert_eq!(light.back(5), 0.0, "nothing is lit");
        light.light(Some((1, run(&[1, 2]))), start);
        assert!(light.advance(start, false), "under way");
        assert_eq!(light.back(5), 0.0, "it has only begun");
        light.advance(start + FADE / 2, false);
        assert!((light.back(5) - 0.5).abs() < 0.01, "{}", light.back(5));
        assert_eq!(light.back(2), 0.0, "a row of the run stays");
        assert!(!light.advance(start + FADE, false), "over");
        assert_eq!(light.back(5), 1.0);
        assert_eq!(light.back(1), 0.0);

        // To another run: what is in both stays, what is in neither stays
        // back, the rest goes from one to the other.
        let later = start + FADE * 3;
        light.light(Some((2, run(&[2, 3]))), later);
        light.advance(later + FADE / 4, false);
        assert_eq!(light.back(2), 0.0);
        assert_eq!(light.back(5), 1.0, "no flash in between");
        assert!((light.back(1) - 0.25).abs() < 0.01, "{}", light.back(1));
        assert!((light.back(3) - 0.75).abs() < 0.01, "{}", light.back(3));

        // Put out, and with reduced motion at once.
        light.light(None, later + FADE * 2);
        assert!(!light.advance(later + FADE * 2, true));
        assert_eq!(light.back(5), 0.0);
        assert_eq!(light.back(1), 0.0);
    }

    #[test]
    fn a_turn_is_lit_only_while_nothing_was_asked_after_it() {
        let mut light = Light::default();
        assert!(light.point_at_labels(Some(3)));
        let first = light.ask();
        assert!(
            !light.point_at_labels(Some(3)),
            "the same row asks nothing new"
        );
        assert!(light.point_at_labels(None));
        let second = light.ask();
        assert!(!light.is_turn(first) && light.is_turn(second));
        light.light(Some((3, run(&[3]))), std::time::Instant::now());
        light.reset();
        assert!(light.lit().is_none() && light.on_labels().is_none());
        assert!(!light.is_turn(second), "a reset puts out what waited");
        // A clock that went back, or a fade of nothing, is no fault.
        let now = std::time::Instant::now();
        light.light(Some((1, run(&[1]))), now + Duration::from_secs(1));
        assert_eq!(light.progress_at(now, false), 0.0);
    }
}
