//! Pure residency decisions: which textures to load, refine, coarsen or evict under a byte
//! budget. No GPU types here, so the policy is unit-testable.

use crate::texture::{MipLayout, TextureFormat};

/// Shape of a streamable texture, known once its first mips have loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureInfo {
    pub format: TextureFormat,
    /// Size of mip 0 in pixels.
    pub width: u32,
    pub height: u32,
    /// Mip levels in the full chain the source can provide.
    pub mip_count: u32,
}

impl TextureInfo {
    /// Bytes of mips `top..mip_count` on the GPU.
    pub fn bytes_from(&self, top: u32) -> u64 {
        (top..self.mip_count)
            .map(|level| {
                MipLayout::new(self.format, self.width, self.height, level).byte_len() as u64
            })
            .sum()
    }

    /// Size of mip `level`.
    pub fn mip_size(&self, level: u32) -> (u32, u32) {
        crate::texture::mip_size(self.width, self.height, level)
    }

    /// The coarsest mip that may be the finest resident one: block-compressed textures need a
    /// base of at least one whole block (4x4), others at least 1x1.
    pub fn coarsest_top(&self) -> u32 {
        let dim = self.format.block_dim();
        (0..self.mip_count)
            .rev()
            .find(|&level| {
                let (w, h) = self.mip_size(level);
                w % dim == 0 && h % dim == 0
            })
            .unwrap_or(0)
    }

    /// The first mip of the initial low-detail load: the finest mip whose larger edge is at most
    /// `max_size`, but never coarser than [`coarsest_top`](Self::coarsest_top).
    pub fn tail_start(&self, max_size: u32) -> u32 {
        let fitting = (0..self.mip_count)
            .find(|&level| {
                let (w, h) = self.mip_size(level);
                w.max(h) <= max_size
            })
            .unwrap_or(self.mip_count.saturating_sub(1));
        fitting.min(self.coarsest_top())
    }

    /// Clamp a wanted top mip to what this texture can have.
    pub fn clamp_top(&self, wanted: u32) -> u32 {
        wanted.min(self.coarsest_top())
    }
}

/// The finest mip needed to draw a texture `texture_size` pixels wide over `screen_size` pixels
/// on screen (one texel per pixel): `floor(log2(texture_size / screen_size))`, at least 0.
pub fn required_mip(texture_size: u32, screen_size: f32) -> u32 {
    if screen_size <= 0.0 || !screen_size.is_finite() {
        return u32::MAX;
    }
    let ratio = texture_size as f32 / screen_size;
    if ratio <= 1.0 {
        0
    } else {
        ratio.log2().floor() as u32
    }
}

/// A slot as the planner sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotView {
    pub id: u32,
    /// Handles held outside the manager.
    pub refs: usize,
    pub info: Option<TextureInfo>,
    /// Finest resident mip, `None` when nothing is resident.
    pub resident_top: Option<u32>,
    /// A load is in flight.
    pub pending: bool,
    /// Loading failed; never requested again.
    pub failed: bool,
    /// Last frame the texture was used or wanted.
    pub last_used: u64,
    /// Finest mip wanted this frame (`u32::MAX`: no particular need).
    pub wanted: u32,
}

impl SlotView {
    fn resident_bytes(&self) -> u64 {
        match (self.info, self.resident_top) {
            (Some(info), Some(top)) => info.bytes_from(top),
            _ => 0,
        }
    }
}

/// What to load for a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipRequest {
    /// First load: the low-detail tail of the chain, mips no larger than `max_size`.
    Tail { max_size: u32 },
    /// Refinement: mips `first..end` (finer than the resident ones).
    Range { first: u32, end: u32 },
}

/// Decisions for one frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Drop the slot entirely (unreferenced).
    pub evict: Vec<u32>,
    /// Keep only mips from this top (coarser) down.
    pub coarsen: Vec<(u32, u32)>,
    pub load: Vec<(u32, MipRequest)>,
}

/// Residency policy settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    /// GPU bytes all streamed textures may use.
    pub budget_bytes: u64,
    /// Largest edge of the initial low-detail load.
    pub tail_size: u32,
    /// Most loads in flight at once.
    pub max_pending: usize,
}

/// Decide this frame's evictions, coarsenings and loads.
///
/// 1. Referenced textures with nothing loaded get their tail requested, most recently used
///    first.
/// 2. Referenced textures that want finer mips are refined, most recently used first, if the
///    extra bytes fit the budget after freeing memory: unreferenced textures are evicted least
///    recently used first, then textures holding finer mips than they want are coarsened, least
///    recently used first.
/// 3. If the budget is still exceeded, the same freeing applies without a refinement to make room
///    for.
pub fn plan(slots: &[SlotView], policy: &Policy) -> Plan {
    let mut plan = Plan::default();
    let mut used: u64 = slots.iter().map(SlotView::resident_bytes).sum();
    let mut pending = slots.iter().filter(|s| s.pending).count();
    let mut by_recent: Vec<&SlotView> = slots.iter().collect();
    by_recent.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.id.cmp(&b.id)));

    // Freeing candidates, least recently used first.
    let mut evictable: Vec<&SlotView> = slots
        .iter()
        .filter(|s| s.refs == 0 && !s.pending && s.resident_top.is_some())
        .collect();
    evictable.sort_by(|a, b| a.last_used.cmp(&b.last_used).then(a.id.cmp(&b.id)));
    let mut coarsenable: Vec<(&SlotView, u32)> = slots
        .iter()
        .filter(|s| s.refs > 0 && !s.pending)
        .filter_map(|s| {
            let info = s.info?;
            let top = s.resident_top?;
            // Holds finer mips than it wants: may drop to the wanted level, but never below
            // the initial tail.
            let tail = info.tail_start(policy.tail_size);
            let target = info.clamp_top(s.wanted).min(tail);
            (target > top).then_some((s, target))
        })
        .collect();
    coarsenable.sort_by(|a, b| a.0.last_used.cmp(&b.0.last_used).then(a.0.id.cmp(&b.0.id)));
    let mut evict_at = 0;
    let mut coarsen_at = 0;
    let mut free_until = |needed: u64, used: &mut u64, plan: &mut Plan| {
        while *used + needed > policy.budget_bytes && evict_at < evictable.len() {
            let s = evictable[evict_at];
            evict_at += 1;
            *used -= s.resident_bytes();
            plan.evict.push(s.id);
        }
        while *used + needed > policy.budget_bytes && coarsen_at < coarsenable.len() {
            let (s, target) = coarsenable[coarsen_at];
            coarsen_at += 1;
            let info = s.info.expect("filtered");
            *used -= s.resident_bytes() - info.bytes_from(target);
            plan.coarsen.push((s.id, target));
        }
        *used + needed <= policy.budget_bytes
    };

    for s in &by_recent {
        if pending >= policy.max_pending {
            break;
        }
        if s.refs > 0 && s.info.is_none() && !s.pending && !s.failed {
            plan.load.push((
                s.id,
                MipRequest::Tail {
                    max_size: policy.tail_size,
                },
            ));
            pending += 1;
        }
    }

    for s in &by_recent {
        if pending >= policy.max_pending {
            break;
        }
        let (Some(info), Some(top)) = (s.info, s.resident_top) else {
            continue;
        };
        if s.refs == 0 || s.pending || s.failed || s.wanted == u32::MAX {
            continue;
        }
        let wanted = info.clamp_top(s.wanted);
        if wanted >= top {
            continue;
        }
        let extra = info.bytes_from(wanted) - info.bytes_from(top);
        if free_until(extra, &mut used, &mut plan) {
            used += extra;
            plan.load.push((
                s.id,
                MipRequest::Range {
                    first: wanted,
                    end: top,
                },
            ));
            pending += 1;
        }
    }

    free_until(0, &mut used, &mut plan);
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(size: u32) -> TextureInfo {
        TextureInfo {
            format: TextureFormat::Bc1,
            width: size,
            height: size,
            mip_count: 32 - size.leading_zeros(),
        }
    }

    fn slot(id: u32) -> SlotView {
        SlotView {
            id,
            refs: 1,
            info: None,
            resident_top: None,
            pending: false,
            failed: false,
            last_used: 0,
            wanted: u32::MAX,
        }
    }

    fn resident(id: u32, size: u32, top: u32) -> SlotView {
        SlotView {
            info: Some(info(size)),
            resident_top: Some(top),
            ..slot(id)
        }
    }

    const POLICY: Policy = Policy {
        budget_bytes: 1 << 30,
        tail_size: 64,
        max_pending: 8,
    };

    #[test]
    fn mip_math() {
        let i = info(1024);
        assert_eq!(i.mip_count, 11);
        // BC1: 1024^2 / 2 bytes for mip 0, a quarter per level, at least one block.
        assert_eq!(i.bytes_from(10), 8);
        assert_eq!(i.bytes_from(0) - i.bytes_from(1), 512 * 1024);
        assert_eq!(i.coarsest_top(), 8, "4x4 is the smallest BC base");
        assert_eq!(i.tail_start(64), 4);
        assert_eq!(i.tail_start(2), 8);
        let rgba = TextureInfo {
            format: TextureFormat::Rgba8,
            ..i
        };
        assert_eq!(rgba.coarsest_top(), 10);
    }

    #[test]
    fn required_mip_matches_texels_per_pixel() {
        assert_eq!(required_mip(1024, 1024.0), 0);
        assert_eq!(required_mip(1024, 2000.0), 0);
        assert_eq!(required_mip(1024, 512.0), 1);
        assert_eq!(required_mip(1024, 100.0), 3);
        assert_eq!(required_mip(1024, 0.0), u32::MAX);
    }

    #[test]
    fn referenced_textures_load_their_tail_first() {
        let mut idle = slot(2);
        idle.refs = 0;
        let p = plan(&[slot(1), idle], &POLICY);
        assert_eq!(p.load, vec![(1, MipRequest::Tail { max_size: 64 })]);
    }

    #[test]
    fn wanted_finer_mips_are_requested() {
        let mut s = resident(1, 1024, 4);
        s.wanted = 1;
        let p = plan(&[s], &POLICY);
        assert_eq!(p.load, vec![(1, MipRequest::Range { first: 1, end: 4 })]);
    }

    #[test]
    fn refinement_evicts_unreferenced_textures_least_recently_used_first() {
        let i = info(1024);
        let mut want = resident(1, 1024, 4);
        want.wanted = 0;
        want.last_used = 10;
        let mut old = resident(2, 1024, 0);
        old.refs = 0;
        old.last_used = 1;
        let mut newer = resident(3, 1024, 0);
        newer.refs = 0;
        newer.last_used = 5;
        let budget = i.bytes_from(4) + 2 * i.bytes_from(0);
        let p = plan(
            &[want, old, newer],
            &Policy {
                budget_bytes: budget,
                ..POLICY
            },
        );
        assert_eq!(p.evict, vec![2]);
        assert_eq!(p.load, vec![(1, MipRequest::Range { first: 0, end: 4 })]);
    }

    #[test]
    fn over_detailed_textures_are_coarsened_before_refusing() {
        let i = info(1024);
        let mut want = resident(1, 1024, 4);
        want.wanted = 0;
        want.last_used = 10;
        // Referenced, holds mip 0 but only wants mip 3.
        let mut lazy = resident(2, 1024, 0);
        lazy.wanted = 3;
        lazy.last_used = 9;
        let budget = i.bytes_from(0) + i.bytes_from(3);
        let p = plan(
            &[want, lazy],
            &Policy {
                budget_bytes: budget,
                ..POLICY
            },
        );
        assert_eq!(p.coarsen, vec![(2, 3)]);
        assert_eq!(p.load, vec![(1, MipRequest::Range { first: 0, end: 4 })]);
    }

    #[test]
    fn refinement_that_cannot_fit_is_skipped() {
        let i = info(1024);
        let mut want = resident(1, 1024, 4);
        want.wanted = 0;
        let p = plan(
            &[want],
            &Policy {
                budget_bytes: i.bytes_from(1),
                ..POLICY
            },
        );
        assert!(p.load.is_empty() && p.evict.is_empty());
    }

    #[test]
    fn over_budget_evicts_without_a_refinement() {
        let i = info(256);
        let mut a = resident(1, 256, 0);
        a.refs = 0;
        a.last_used = 3;
        let mut b = resident(2, 256, 0);
        b.refs = 0;
        b.last_used = 7;
        let p = plan(
            &[a, b],
            &Policy {
                budget_bytes: i.bytes_from(0),
                ..POLICY
            },
        );
        assert_eq!(p.evict, vec![1]);
    }

    #[test]
    fn unwanted_detail_falls_back_to_the_tail_under_pressure() {
        let i = info(1024);
        let mut s = resident(1, 1024, 0);
        s.wanted = u32::MAX;
        let p = plan(
            &[s],
            &Policy {
                budget_bytes: i.bytes_from(2),
                ..POLICY
            },
        );
        assert_eq!(p.coarsen, vec![(1, 4)]);
    }

    #[test]
    fn pending_and_failed_slots_are_left_alone_and_loads_are_capped() {
        let mut busy = slot(1);
        busy.pending = true;
        let mut broken = slot(2);
        broken.failed = true;
        let more: Vec<SlotView> = (3..20).map(slot).collect();
        let mut all = vec![busy, broken];
        all.extend(more);
        let p = plan(
            &all,
            &Policy {
                max_pending: 4,
                ..POLICY
            },
        );
        assert_eq!(p.load.len(), 3, "one already pending");
        assert!(p.load.iter().all(|(id, _)| *id >= 3));
    }
}
