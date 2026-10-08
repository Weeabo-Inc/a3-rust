//! The move graph and the engine's move path search (`docs/re/moves.md`).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::{MoveId, Moves};

/// How a transition to the next move happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// The next move starts when this one has played to its end (`connectTo`; engine type 1).
    Connect,
    /// The next move blends in at once, at its `interpolationSpeed` (`interpolateTo`; engine
    /// type 2).
    Interpolate,
}

/// One edge of the move graph: a move that may follow the one it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edge {
    pub to: MoveId,
    /// Config cost × 1000, rounded and clamped to `i16`, as the engine stores it.
    pub cost: i16,
    pub kind: EdgeKind,
    /// The target is in the source move's `ignoreMinPlayTime[]`: an interpolation may start
    /// before the source reaches its `minPlayTime`.
    pub ignore_min_play_time: bool,
}

/// The stored cost of a config cost.
pub(crate) fn stored_cost(cost: f32) -> i16 {
    (cost * 1000.0)
        .round()
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

impl Moves {
    /// Adds or replaces the edge `from → to`; self edges are ignored (engine `0x140603c60`).
    pub(crate) fn set_edge(
        &mut self,
        from: MoveId,
        to: MoveId,
        kind: EdgeKind,
        cost: i16,
        ignore_min_play_time: bool,
    ) {
        if from == to {
            return;
        }
        let list = &mut self.edges[from.index()];
        let edge = Edge {
            to,
            cost,
            kind,
            ignore_min_play_time,
        };
        match list.iter_mut().find(|e| e.to == to) {
            Some(e) => *e = edge,
            None => list.push(edge),
        }
    }

    pub(crate) fn remove_edge(&mut self, from: MoveId, to: MoveId) {
        self.edges[from.index()].retain(|e| e.to != to);
    }

    /// The moves to play, in order, to get from `from` to `to`; the last is `to`. Empty when
    /// `from == to`; `None` when `to` cannot be reached.
    ///
    /// As the engine does it (`0x140604c40`): a direct edge is taken whatever its cost;
    /// otherwise a Dijkstra search over the integer costs that **stops as soon as `to` first
    /// gets a distance**, so the result is the cheapest path through the first expanded move
    /// with an edge into `to`, not always the cheapest path overall.
    pub fn find_path(&self, from: MoveId, to: MoveId) -> Option<Vec<MoveId>> {
        let n = self.moves.len();
        if from == to {
            return Some(Vec::new());
        }
        if from.index() >= n || to.index() >= n {
            return None;
        }
        if self.edge(from, to).is_some() {
            return Some(vec![to]);
        }
        const UNREACHED: i32 = i32::MAX;
        let mut dist = vec![UNREACHED; n];
        let mut prev: Vec<Option<MoveId>> = vec![None; n];
        let mut heap = BinaryHeap::new();
        dist[from.index()] = 0;
        heap.push(Reverse((0i32, from.0)));
        while dist[to.index()] == UNREACHED {
            let Some(Reverse((d, u))) = heap.pop() else {
                break;
            };
            if d != dist[u as usize] {
                continue; // a stale entry
            }
            for e in self.edges(MoveId(u)) {
                let v = e.to.index();
                let nd = d + i32::from(e.cost);
                if nd < dist[v] {
                    dist[v] = nd;
                    prev[v] = Some(MoveId(u));
                    heap.push(Reverse((nd, e.to.0)));
                }
            }
        }
        prev[to.index()]?;
        let mut path = vec![to];
        let mut at = to;
        while let Some(p) = prev[at.index()] {
            // Negative costs (never shipped) could leave a cycle; a path has at most n moves.
            if p == from || path.len() > n {
                break;
            }
            path.push(p);
            at = p;
        }
        path.reverse();
        Some(path)
    }
}
