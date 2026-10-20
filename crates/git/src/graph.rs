//! Lane layout for the commit graph.
//!
//! Input: commits in child-before-parent order (see [`crate::log`]). Output:
//! one [`GraphRow`] per commit describing the node column, the lanes passing
//! straight through below the row, and the lines leaving the node towards
//! its parents in the next row. A lane never changes column and its colour
//! id is that column, so a branch keeps its colour while scrolling and lanes
//! next to each other never share one. [`layout_with_wip`] adds a leading
//! row for uncommitted work, laid out as a child of HEAD.
//!
//! Pinned lanes ([`layout_pinned`]): a pinned branch keeps a lane at the
//! left edge. Its commits are its tip and the first parents below it, the
//! line a branch is usually said to be. The first pin has column 0, the
//! second column 1, and so on. A pinned lane is kept free from the top of
//! the graph down to the branch's tip, holds the branch from there, and
//! ends where the line reaches a root or a commit of a pinned lane further
//! left, which it joins; below that the column is anyone's. A pin whose
//! tip is not among the commits (it is hidden, or left out while only some
//! references are shown), or whose tip already lies on a pinned lane
//! further left, takes no column. Everything else is laid out as without
//! pins, to the right of the columns that are kept.
//!
//! Sources: none (original design).

use serde::{Deserialize, Serialize};

use crate::{Oid, log::GraphNode};

/// A lane slot with its colour id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaneRef {
    /// Column.
    pub slot: u16,
    /// Colour id: the column (renderers take it modulo their palette size).
    pub color: u32,
}

impl LaneRef {
    fn at(slot: usize) -> Self {
        let slot = to_u16(slot);
        Self {
            slot,
            color: u32::from(slot),
        }
    }
}

/// Layout of one commit row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphRow {
    /// Column of the commit node.
    pub lane: u16,
    /// Colour id of the node's lane.
    pub color: u32,
    /// The node's own lane continues straight down to its first parent.
    pub continues: bool,
    /// Other lanes passing straight through this row, ordered by slot.
    pub straight: Vec<LaneRef>,
    /// Lines from the node to a lane in the next row: further parents of a
    /// merge, joining an existing lane or opening a new one.
    pub forks: Vec<LaneRef>,
    /// Lanes from the previous row that end in this node: branches whose
    /// first parent is this commit, drawn as a curve into the node.
    pub merges_in: Vec<LaneRef>,
    /// Number of columns in use after this row (for width calculation).
    pub width: u16,
}

impl GraphRow {
    /// Whether a lane leaves this row downwards in `slot`: a lane passing
    /// through, the node's own lane when it continues, or a lane this node
    /// opens or joins for a further parent.
    pub fn leaves(&self, slot: u16) -> bool {
        (self.continues && self.lane == slot)
            || self
                .straight
                .binary_search_by_key(&slot, |lane| lane.slot)
                .is_ok()
            || self.forks.iter().any(|fork| fork.slot == slot)
    }
}
fn to_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// More pins than this are not kept: every pin is a column at the left
/// edge, and the list comes from a file anyone can edit.
pub const MAX_PINNED: usize = 16;

/// Lane assignment state, advanced one node at a time: per column, the
/// commit its lane is heading for.
#[derive(Debug, Default)]
struct Lanes {
    slots: Vec<Option<Oid>>,
    /// Per pinned lane, which is the column of the same index: the commit
    /// the pinned branch's line comes to next, starting with its tip. None
    /// once the line has ended, which gives the column back.
    pinned: Vec<Option<Oid>>,
    /// The columns whose lanes arrive at the node being placed, kept from
    /// one node to the next: a list made anew for every node is a fair
    /// part of the layout's time where allocating is dear (Windows).
    arriving: Vec<usize>,
}

impl Lanes {
    /// Lanes with the first columns kept for the branches whose tips are
    /// `pinned`, in that order. Only pins that will hold a commit of
    /// `commits` are kept: see the module's text.
    fn with_pins<T: GraphNode>(commits: &[T], pinned: &[Oid]) -> Self {
        Self::with_pins_of(commits, pinned, false)
    }

    /// [`Self::with_pins`] for commits that are the beginning of a history
    /// when `more` is set: a pin whose tip is not among them is taken to
    /// come further down and keeps its column.
    fn with_pins_of<T: GraphNode>(commits: &[T], pinned: &[Oid], more: bool) -> Self {
        let mut wanted: Vec<Oid> = Vec::new();
        for tip in pinned {
            if !wanted.contains(tip) && wanted.len() < MAX_PINNED {
                wanted.push(*tip);
            }
        }
        if wanted.is_empty() {
            return Self::default();
        }
        // One pass ahead of the layout: follow every pinned line down and
        // see which ones get a commit of their own. A line is at its tip
        // until that comes by, then at the first parent of what it took.
        let mut next: Vec<Option<Oid>> = wanted.iter().copied().map(Some).collect();
        let mut owns = vec![false; wanted.len()];
        let mut seen = vec![false; wanted.len()];
        let mut left = next.len();
        for commit in commits {
            if left == 0 {
                break;
            }
            let id = commit.id();
            let mut taken = false;
            for ((line, owned), seen) in next.iter_mut().zip(owns.iter_mut()).zip(seen.iter_mut()) {
                if *line != Some(id) {
                    continue;
                }
                *seen = true;
                if taken {
                    // It joins a pinned line further left here.
                    *line = None;
                } else {
                    taken = true;
                    *owned = true;
                    *line = commit.parents().first().copied();
                }
                if line.is_none() {
                    left = left.saturating_sub(1);
                }
            }
        }
        let pinned: Vec<Option<Oid>> = wanted
            .into_iter()
            .zip(owns.into_iter().zip(seen))
            .filter_map(|(tip, (owned, seen))| (owned || (more && !seen)).then_some(Some(tip)))
            .collect();
        Self {
            slots: vec![None; pinned.len()],
            pinned,
            arriving: Vec::new(),
        }
    }

    /// Whether a column is kept for a pinned line that has not ended.
    fn kept(&self, slot: usize) -> bool {
        self.pinned.get(slot).is_some_and(Option::is_some)
    }

    /// The pinned lane that comes to `id` next, the leftmost when several
    /// do.
    fn pin_at(&self, id: Oid) -> Option<usize> {
        if self.pinned.is_empty() {
            return None;
        }
        self.pinned.iter().position(|next| *next == Some(id))
    }

    /// First free slot at or right of `from`, or a new one. A column kept
    /// for a pinned line is not free.
    fn free_slot(&mut self, from: usize) -> usize {
        let mut slot = from;
        // Ends at the latest past the slots in use and the columns kept.
        while self.slots.get(slot).is_some_and(Option::is_some) || self.kept(slot) {
            slot = slot.saturating_add(1);
        }
        while self.slots.len() <= slot {
            self.slots.push(None);
        }
        slot
    }

    fn set(&mut self, slot: usize, expected: Oid) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = Some(expected);
        }
    }

    /// Place the next node. `id` is `None` for the uncommitted-work node,
    /// which no lane can be waiting for.
    fn place(&mut self, id: Option<Oid>, parents: &[Oid]) -> GraphRow {
        // Every lane whose next expected commit is this one arrives here. The
        // leftmost becomes the node's lane; the others curve into the node.
        let mut arriving = std::mem::take(&mut self.arriving);
        arriving.clear();
        if let Some(id) = id {
            arriving.extend(
                self.slots
                    .iter()
                    .enumerate()
                    .filter_map(|(i, slot)| (*slot == Some(id)).then_some(i)),
            );
        }
        // A commit of a pinned line sits in that line's column, whatever
        // else arrives. Uncommitted work on top of a pinned tip does too.
        let pin = match id {
            Some(id) => self.pin_at(id),
            None => parents.first().and_then(|head| self.pin_at(*head)),
        };
        let my_slot = match (pin, arriving.first().copied()) {
            (Some(pin), _) => {
                while self.slots.len() <= pin {
                    self.slots.push(None);
                }
                pin
            }
            (None, Some(i)) => i,
            (None, None) => self.free_slot(0),
        };
        if let (Some(pin), Some(id)) = (pin, id) {
            // The line goes on to the first parent, or ends at a root.
            // Lines further right that come to the same commit join here.
            for (lane, next) in self.pinned.iter_mut().enumerate() {
                if lane == pin {
                    *next = parents.first().copied();
                } else if *next == Some(id) {
                    *next = None;
                }
            }
        }
        let mut merges_in = Vec::new();
        for i in &arriving {
            if let Some(slot) = self.slots.get_mut(*i) {
                *slot = None;
            }
            if *i != my_slot {
                merges_in.push(LaneRef::at(*i));
            }
        }
        self.arriving = arriving;

        // Made at its size: built up as it is found, a wide graph's row
        // would be copied into ever larger lists several times over.
        let mut straight = Vec::with_capacity(self.slots.iter().flatten().count());
        straight.extend(
            self.slots
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| slot.map(|_| LaneRef::at(i))),
        );

        let mut forks = Vec::new();
        let mut continues = false;
        for (index, parent) in parents.iter().enumerate() {
            if index == 0 {
                // The first parent always continues in this lane, even when
                // another lane already heads there: branches keep their own
                // vertical line until the row where they started.
                self.set(my_slot, *parent);
                continues = true;
                continue;
            }
            let slot = match self.slots.iter().position(|slot| *slot == Some(*parent)) {
                Some(i) => i,
                None => {
                    // A parent that is the tip of a pinned line is joined
                    // in that line's column, which was kept for it.
                    let i = match self.pin_at(*parent) {
                        Some(pin) => {
                            while self.slots.len() <= pin {
                                self.slots.push(None);
                            }
                            pin
                        }
                        None => self.free_slot(my_slot.saturating_add(1)),
                    };
                    self.set(i, *parent);
                    i
                }
            };
            forks.push(LaneRef::at(slot));
        }

        let width = to_u16(self.slots.len().max(my_slot.saturating_add(1)));
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
        let node = LaneRef::at(my_slot);
        GraphRow {
            lane: node.slot,
            color: node.color,
            continues,
            straight,
            forks,
            merges_in,
            width,
        }
    }
}

/// Compute the lane layout for commits in child-before-parent order.
pub fn layout<T: GraphNode>(commits: &[T]) -> Vec<GraphRow> {
    layout_pinned(commits, &[])
}

/// Like [`layout`], with the leftmost columns kept for the branches whose
/// tips are `pinned`, in that order (see the module's text for what a
/// pinned lane holds and where it ends). Without pins it is [`layout`].
pub fn layout_pinned<T: GraphNode>(commits: &[T], pinned: &[Oid]) -> Vec<GraphRow> {
    let mut lanes = Lanes::with_pins(commits, pinned);
    commits
        .iter()
        .map(|commit| lanes.place(Some(commit.id()), commit.parents()))
        .collect()
}

/// Like [`layout`], with a leading row for uncommitted work drawn as a child
/// of `head`: its lane runs from the top of the graph down to that commit.
/// Returns that row and the commit rows. When `head` is absent or not among
/// `commits`, the leading row stands alone and takes no lane.
pub fn layout_with_wip<T: GraphNode>(
    commits: &[T],
    head: Option<Oid>,
) -> (GraphRow, Vec<GraphRow>) {
    layout_with_wip_pinned(commits, head, &[])
}

/// [`layout_with_wip`] with pinned lanes as in [`layout_pinned`]. The row
/// for uncommitted work sits in the pinned lane when `head` is the tip of
/// a pinned branch.
pub fn layout_with_wip_pinned<T: GraphNode>(
    commits: &[T],
    head: Option<Oid>,
    pinned: &[Oid],
) -> (GraphRow, Vec<GraphRow>) {
    let mut lanes = Lanes::with_pins(commits, pinned);
    let wip = match head.filter(|head| commits.iter().any(|commit| commit.id() == *head)) {
        Some(head) => lanes.place(None, &[head]),
        // Nothing to attach to: a lone node that takes no lane.
        None => GraphRow {
            lane: 0,
            color: 0,
            continues: false,
            straight: Vec::new(),
            forks: Vec::new(),
            merges_in: Vec::new(),
            width: 1,
        },
    };
    let rows = commits
        .iter()
        .map(|commit| lanes.place(Some(commit.id()), commit.parents()))
        .collect();
    (wip, rows)
}

/// The layout of the leading rows of a history of which more is to come
/// (see [`crate::pieces`]): what [`layout_with_wip_pinned`] or
/// [`layout_pinned`] give for the whole history, cut to these rows. What
/// lies further down is taken to be there: the lane of uncommitted work
/// runs on towards a `head` that is not among the commits, and a pinned
/// branch whose tip is not among them keeps its column.
///
/// `wip` says whether there is uncommitted work, and on which commit; work
/// on no commit (a branch without commits) is a lone node. Two things can
/// make the whole differ from what was laid out here: a `head` that turns
/// out not to be in the graph at all, and a pin that turns out to hold no
/// commit.
pub fn layout_piece<T: GraphNode>(
    commits: &[T],
    wip: Option<Option<Oid>>,
    pinned: &[Oid],
) -> (Option<GraphRow>, Vec<GraphRow>) {
    let mut lanes = Lanes::with_pins_of(commits, pinned, true);
    let wip = wip.map(|head| match head {
        Some(head) => lanes.place(None, &[head]),
        None => GraphRow {
            lane: 0,
            color: 0,
            continues: false,
            straight: Vec::new(),
            forks: Vec::new(),
            merges_in: Vec::new(),
            width: 1,
        },
    });
    let rows = commits
        .iter()
        .map(|commit| lanes.place(Some(commit.id()), commit.parents()))
        .collect();
    (wip, rows)
}

/// Render rows as ASCII art, one line per commit plus a connector line when
/// the node forks. Used for golden tests and the CLI. `label` supplies the
/// text after the node (typically short id and summary).
pub fn ascii<T: GraphNode>(
    commits: &[T],
    rows: &[GraphRow],
    label: impl Fn(&T) -> String,
) -> String {
    let mut out = String::new();
    for (commit, row) in commits.iter().zip(rows) {
        let width = usize::from(row.width.max(1));
        if !row.merges_in.is_empty() {
            // Lanes arriving from above curve into this node.
            let mut above = vec![' '; width * 2 + 2];
            for lane in &row.straight {
                if let Some(c) = above.get_mut(usize::from(lane.slot) * 2) {
                    *c = '|';
                }
            }
            if let Some(c) = above.get_mut(usize::from(row.lane) * 2) {
                *c = '|';
            }
            for lane in &row.merges_in {
                let glyph = if lane.slot > row.lane { '/' } else { '\\' };
                let pos = if lane.slot > row.lane {
                    usize::from(lane.slot) * 2 - 1
                } else {
                    usize::from(lane.slot) * 2 + 1
                };
                if let Some(c) = above.get_mut(pos) {
                    *c = glyph;
                }
            }
            let above: String = above.into_iter().collect();
            out.push_str(above.trim_end());
            out.push('\n');
        }
        let mut line = vec![' '; width * 2];
        for lane in &row.straight {
            if let Some(c) = line.get_mut(usize::from(lane.slot) * 2) {
                *c = '|';
            }
        }
        if let Some(c) = line.get_mut(usize::from(row.lane) * 2) {
            *c = '*';
        }
        let line: String = line.into_iter().collect();
        out.push_str(line.trim_end());
        out.push_str("  ");
        out.push_str(&label(commit));
        out.push('\n');

        if !row.forks.is_empty() {
            let mut connector = vec![' '; width * 2];
            for lane in &row.straight {
                if let Some(c) = connector.get_mut(usize::from(lane.slot) * 2) {
                    *c = '|';
                }
            }
            if row.continues
                && let Some(c) = connector.get_mut(usize::from(row.lane) * 2)
            {
                *c = '|';
            }
            for fork in &row.forks {
                let glyph = if fork.slot > row.lane {
                    '\\'
                } else if fork.slot < row.lane {
                    '/'
                } else {
                    '|'
                };
                let pos = usize::from(fork.slot) * 2;
                // Draw the diagonal one column towards the node.
                let pos = if fork.slot > row.lane {
                    pos.saturating_sub(1)
                } else if fork.slot < row.lane {
                    pos.saturating_add(1)
                } else {
                    pos
                };
                if let Some(c) = connector.get_mut(pos) {
                    *c = glyph;
                }
            }
            let connector: String = connector.into_iter().collect();
            out.push_str(connector.trim_end());
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommitInfo, Signature};

    fn commit(id: u8, parents: &[u8]) -> CommitInfo {
        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        CommitInfo {
            id: oid(id),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            author: Signature::default(),
            committer: Signature::default(),
            summary: format!("c{id}"),
            body: String::new(),
        }
    }

    #[test]
    fn linear_history_uses_one_lane() {
        let commits = vec![commit(3, &[2]), commit(2, &[1]), commit(1, &[])];
        let rows = layout(&commits);
        assert!(
            rows.iter()
                .all(|r| r.lane == 0 && r.color == 0 && r.width == 1)
        );
        assert_eq!(
            rows.iter().map(|r| r.continues).collect::<Vec<_>>(),
            [true, true, false]
        );
    }

    #[test]
    fn merge_opens_a_second_lane_that_runs_to_its_origin() {
        // 4 = merge(3, 2); 3 -> 1; 2 -> 1; 1 root
        let commits = vec![
            commit(4, &[3, 2]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let rows = layout(&commits);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].forks.len(), 1, "merge forks to the second parent");
        assert_eq!(rows[0].forks[0].slot, 1);
        assert_eq!(rows[2].lane, 1, "second parent sits in lane 1");
        assert!(
            rows[2].continues,
            "and keeps its own vertical line down to the origin"
        );
        assert!(rows[2].forks.is_empty());
        assert_eq!(rows[3].lane, 0);
        assert_eq!(
            rows[3].merges_in.len(),
            1,
            "the branch curves into its origin on the origin's row"
        );
        assert_eq!(rows[3].merges_in[0].slot, 1);
        assert_eq!(rows.iter().map(|r| r.width).max(), Some(2));
    }

    #[test]
    fn lane_colours_follow_the_column() {
        let commits = vec![
            commit(4, &[3, 2]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let rows = layout(&commits);
        assert_eq!(rows[0].color, 0);
        assert_eq!(rows[2].color, 1);
        assert_eq!(rows[3].color, 0);
        for row in &rows {
            assert_eq!(row.color, u32::from(row.lane));
            let lanes = row.straight.iter().chain(&row.forks).chain(&row.merges_in);
            for lane in lanes {
                assert_eq!(lane.color, u32::from(lane.slot));
            }
        }
    }

    #[test]
    fn a_reused_column_keeps_its_colour() {
        // Two short branches, one after the other, both beside the trunk:
        // 6 -> 4; 5 -> 4 (trunk); 4 -> 2; 3 -> 2; 2 -> 1.
        let commits = vec![
            commit(6, &[4]),
            commit(5, &[4]),
            commit(4, &[2]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let rows = layout(&commits);
        assert_eq!((rows[1].lane, rows[1].color), (1, 1));
        assert_eq!((rows[3].lane, rows[3].color), (1, 1));
        assert!(rows.iter().all(|r| r.width <= 2));
    }

    #[test]
    fn straight_lanes_are_ordered_by_slot_and_leave_the_row() {
        // 5 = merge(4, 3, 2): three lanes side by side below the merge.
        let commits = vec![
            commit(5, &[4, 3, 2]),
            commit(4, &[1]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let rows = layout(&commits);
        for row in &rows {
            assert!(row.straight.windows(2).all(|w| w[0].slot < w[1].slot));
        }
        assert!(rows[0].leaves(0) && rows[0].leaves(1) && rows[0].leaves(2));
        assert!(!rows[0].leaves(3));
        assert_eq!(
            rows[2].straight.iter().map(|l| l.slot).collect::<Vec<_>>(),
            [0, 2]
        );
        assert!(rows[2].leaves(0) && rows[2].leaves(1) && rows[2].leaves(2));
        assert!(!rows[4].leaves(0), "the root leaves nothing below it");
    }

    #[test]
    fn wip_row_takes_the_first_lane_down_to_head() {
        // 3 is a newer commit on another branch; HEAD is 2. 3 -> 1; 2 -> 1.
        let commits = vec![commit(3, &[1]), commit(2, &[1]), commit(1, &[])];
        let head = commits[1].id;
        let (wip, rows) = layout_with_wip(&commits, Some(head));
        assert_eq!((wip.lane, wip.color), (0, 0));
        assert!(wip.continues, "the lane runs down towards HEAD");
        assert_eq!(rows[0].lane, 1, "the newer commit moves out of the way");
        assert_eq!(rows[0].straight.len(), 1);
        assert_eq!(rows[0].straight[0].slot, 0);
        assert_eq!((rows[1].lane, rows[1].color), (0, 0), "HEAD ends the lane");
        assert!(rows[1].merges_in.is_empty());
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].merges_in.len(), 1);

        // Without uncommitted work the newer commit keeps the first lane.
        let plain = layout(&commits);
        assert_eq!(plain[0].lane, 0);
        assert_eq!(plain[1].lane, 1);
    }

    #[test]
    fn wip_row_stands_alone_when_head_is_not_in_the_graph() {
        let commits = vec![commit(2, &[1]), commit(1, &[])];
        let stranger = commit(9, &[]).id;
        for head in [None, Some(stranger)] {
            let (wip, rows) = layout_with_wip(&commits, head);
            assert!(!wip.continues);
            assert!(wip.forks.is_empty() && wip.straight.is_empty());
            assert_eq!(rows, layout(&commits));
        }
        let none: [CommitInfo; 0] = [];
        let (wip, rows) = layout_with_wip(&none, Some(stranger));
        assert!(!wip.continues);
        assert!(rows.is_empty());
    }

    #[test]
    fn duplicate_parents_do_not_open_a_second_lane() {
        // A hostile commit listing the same parent twice.
        let commits = vec![commit(2, &[1, 1]), commit(1, &[])];
        let rows = layout(&commits);
        assert_eq!(rows[0].forks.len(), 1);
        assert_eq!(rows[0].forks[0].slot, rows[0].lane);
        assert_eq!(rows.iter().map(|r| r.width).max(), Some(1));
    }

    fn lanes(rows: &[GraphRow]) -> Vec<u16> {
        rows.iter().map(|row| row.lane).collect()
    }

    #[test]
    fn a_pinned_branch_keeps_the_first_lane_from_the_top_down_to_the_root() {
        // 5 is a newer commit on a topic; main is 4 -> 2 -> 1; 3 -> 2 is
        // another topic. Unpinned, the newest commit takes the first lane.
        let commits = vec![
            commit(5, &[4]),
            commit(4, &[2]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        assert_eq!(lanes(&layout(&commits)), [0, 0, 1, 0, 0]);
        // Pin the branch at 3: its line is 3, 2, 1.
        let rows = layout_pinned(&commits, &[commits[2].id]);
        assert_eq!(lanes(&rows), [1, 1, 0, 0, 0]);
        // The lane is kept free above the tip, and the others come into
        // the pinned commit from the right.
        assert!(!rows[0].leaves(0) && !rows[1].leaves(0));
        assert_eq!(rows[0].width, 2);
        assert_eq!(rows[3].merges_in, [LaneRef::at(1)]);
        assert!(rows[3].continues && !rows[4].continues);
        // Pinning what is leftmost anyway changes nothing.
        assert_eq!(layout_pinned(&commits, &[commits[0].id]), layout(&commits));
    }

    #[test]
    fn pinned_lanes_come_in_pin_order_and_a_later_one_ends_where_it_joins() {
        // main: 6 -> 4 -> 2 -> 1; develop: 5 -> 3 -> 2; 7 is a topic on 6.
        let commits = vec![
            commit(7, &[6]),
            commit(6, &[4]),
            commit(5, &[3]),
            commit(4, &[2]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let (main, develop) = (commits[1].id, commits[2].id);
        let rows = layout_pinned(&commits, &[main, develop]);
        //                      7  6  5  4  3  2  1
        assert_eq!(lanes(&rows), [2, 0, 1, 0, 1, 0, 0]);
        assert_eq!(rows[5].merges_in, [LaneRef::at(1)], "develop joins main");
        // The other way round, develop has the first lane down to the
        // root and main ends where it joins.
        let rows = layout_pinned(&commits, &[develop, main]);
        assert_eq!(lanes(&rows), [2, 1, 0, 1, 0, 0, 0]);
        assert_eq!(rows[5].merges_in, [LaneRef::at(1)], "main joins develop");
    }

    #[test]
    fn a_column_is_given_back_below_where_its_pinned_line_ended() {
        // main: 5 -> 1; develop: 4 -> 1, joining main at the root;
        // 3 -> 2 -> (root of its own) comes later and may use lane 1.
        let commits = vec![
            commit(5, &[1]),
            commit(4, &[1]),
            commit(1, &[]),
            commit(3, &[2]),
            commit(2, &[]),
        ];
        let rows = layout_pinned(&commits, &[commits[0].id, commits[1].id]);
        assert_eq!(lanes(&rows), [0, 1, 0, 0, 0]);
    }

    #[test]
    fn pins_that_hold_no_commit_take_no_column() {
        let commits = vec![commit(3, &[2]), commit(2, &[1]), commit(1, &[])];
        let stranger = commit(9, &[]).id;
        let plain = layout(&commits);
        // Not in the graph; twice the same; a tip that lies on a pinned
        // line further left.
        assert_eq!(layout_pinned(&commits, &[stranger]), plain);
        assert_eq!(
            layout_pinned(&commits, &[commits[0].id, commits[0].id]),
            plain
        );
        assert_eq!(
            layout_pinned(&commits, &[commits[0].id, commits[1].id, stranger]),
            plain
        );
        // More pins than are kept, all strangers: nothing breaks and
        // nothing is kept free.
        let many: Vec<Oid> = (10..40).map(|n| commit(n, &[]).id).collect();
        assert_eq!(layout_pinned(&commits, &many), plain);
        let none: [CommitInfo; 0] = [];
        assert!(layout_pinned(&none, &many).is_empty());
    }

    #[test]
    fn a_merge_of_a_pinned_tip_goes_straight_to_its_lane() {
        // 4 = merge(3, 2) on a topic, where 2 is the tip of pinned main.
        let commits = vec![
            commit(4, &[3, 2]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let rows = layout_pinned(&commits, &[commits[2].id]);
        assert_eq!(lanes(&rows), [1, 1, 0, 0]);
        assert_eq!(rows[0].forks, [LaneRef::at(0)]);
        assert!(rows[2].merges_in.is_empty(), "it arrived in its own lane");
        assert_eq!(rows[3].merges_in, [LaneRef::at(1)]);
    }

    #[test]
    fn uncommitted_work_on_a_pinned_branch_sits_in_its_lane() {
        // 3 is newer, on another branch; HEAD is 2, which is pinned.
        let commits = vec![commit(3, &[1]), commit(2, &[1]), commit(1, &[])];
        let head = commits[1].id;
        let (wip, rows) = layout_with_wip_pinned(&commits, Some(head), &[head]);
        assert_eq!(wip.lane, 0);
        assert_eq!(lanes(&rows), [1, 0, 0]);
        // HEAD on another branch: the row for uncommitted work stays out
        // of the lane that is kept.
        let (wip, rows) = layout_with_wip_pinned(&commits, Some(commits[0].id), &[head]);
        assert_eq!(wip.lane, 1);
        assert_eq!(lanes(&rows), [1, 0, 0]);
        assert!(!wip.leaves(0));
    }

    #[test]
    fn the_layout_of_a_piece_is_the_layout_of_the_whole_cut_short() {
        // main: 8 -> 6 -> 4 -> 2 -> 1; develop: 7 -> 5 -> 3 -> 2; 9 is a
        // topic on 8. HEAD is 5, with uncommitted work.
        let commits = vec![
            commit(9, &[8]),
            commit(8, &[6]),
            commit(7, &[5]),
            commit(6, &[4]),
            commit(5, &[3]),
            commit(4, &[2]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let id = |n: usize| commits[9 - n].id;
        let cases: [(Option<Oid>, Vec<Oid>); 5] = [
            (None, vec![]),
            (Some(id(5)), vec![]),
            (Some(id(1)), vec![id(8)]),
            (Some(id(5)), vec![id(8), id(7)]),
            (None, vec![id(3), id(4)]),
        ];
        for (head, pins) in cases {
            let (whole_wip, whole) = match head {
                Some(head) => {
                    let (wip, rows) = layout_with_wip_pinned(&commits, Some(head), &pins);
                    (Some(wip), rows)
                }
                None => (None, layout_pinned(&commits, &pins)),
            };
            for size in 0..=commits.len() {
                let (wip, rows) = layout_piece(&commits[..size], head.map(Some), &pins);
                assert_eq!(wip, whole_wip, "{head:?} {pins:?} {size}: uncommitted work");
                assert_eq!(rows, whole[..size], "{head:?} {pins:?} {size}");
            }
        }
        // Uncommitted work on no commit is a lone node, as in the whole.
        let (lone, rows) = layout_piece(&commits[..3], Some(None), &[]);
        assert_eq!(Some(lone.unwrap()), Some(layout_with_wip(&commits, None).0));
        assert_eq!(rows, layout(&commits)[..3]);
        let none: [CommitInfo; 0] = [];
        let (wip, rows) = layout_piece(&none, Some(Some(id(5))), &[id(8)]);
        assert!(wip.is_some_and(|wip| wip.continues) && rows.is_empty());
    }

    #[test]
    fn ascii_renders_without_panicking_on_empty_input() {
        let none: [CommitInfo; 0] = [];
        assert_eq!(ascii(&none, &[], |c| c.summary.clone()), "");
    }
}
