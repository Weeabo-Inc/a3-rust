//! Turning an [`a3_ui::DrawList`] into GPU vertices and draw batches.
//!
//! Pure CPU work, unit-testable without a device: quads in screen pixels become two
//! triangles each, [`a3_ui::Uv::Texels`] coordinates are normalized with the loaded texture
//! size, and consecutive quads sharing a texture and a scissor rectangle are joined into one
//! [`Batch`] so the render pass stays in painter order with few state changes.

use a3_ui::{DrawList, Uv};

use crate::cache::TextureInfo;

/// The one-by-one white texture every untextured quad samples.
pub const WHITE_SLOT: u32 = 0;

/// One UI vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiVertex {
    /// Position in screen pixels, from the top-left corner.
    pub position: [f32; 2],
    /// Texture coordinates in 0..1.
    pub uv: [f32; 2],
    /// sRGB colour, straight (not premultiplied) alpha.
    pub color: [f32; 4],
}

/// A run of vertices drawn with one texture slot and one scissor rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    /// First vertex in [`Geometry::vertices`].
    pub start: u32,
    /// Number of vertices (a multiple of 6).
    pub count: u32,
    /// Bind group slot in the UI texture cache ([`WHITE_SLOT`] when untextured).
    pub slot: u32,
    /// Scissor rectangle `[x, y, w, h]` in pixels, clamped to the viewport.
    pub scissor: [u32; 4],
}

/// The vertices and batches of one frame, in drawing order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<UiVertex>,
    pub batches: Vec<Batch>,
    /// Quads skipped because their texture is not loaded (yet).
    pub missing_textures: usize,
    /// Quads skipped because they are empty, transparent or fully clipped away.
    pub skipped: usize,
}

/// Builds the frame's geometry.
///
/// `textures` holds the resolved [`TextureInfo`] of each texture of the list, indexed by
/// [`a3_ui::TextureKey`]; a quad whose texture resolves to `None` is dropped and counted in
/// [`Geometry::missing_textures`]. `viewport` is the target size in pixels, used to clamp
/// scissor rectangles.
pub fn build_geometry(
    list: &DrawList,
    textures: &[Option<TextureInfo>],
    viewport: (u32, u32),
) -> Geometry {
    let mut g = Geometry::default();
    for quad in &list.quads {
        if quad.color[3] <= 0.0 || quad.rect[2] <= 0.0 || quad.rect[3] <= 0.0 {
            g.skipped += 1;
            continue;
        }
        let scissor = scissor_rect(quad.clip, viewport);
        if scissor[2] == 0 || scissor[3] == 0 {
            g.skipped += 1;
            continue;
        }
        let (slot, size) = match quad.texture {
            Some(key) => match textures.get(key.0 as usize).copied().flatten() {
                Some(info) => (info.slot, Some(info.size)),
                None => {
                    g.missing_textures += 1;
                    continue;
                }
            },
            None => (WHITE_SLOT, None),
        };
        // A texel uv without a texture to size it against cannot be normalized.
        let Some([u0, v0, u1, v1]) = uv_range(quad.uv, size) else {
            g.missing_textures += 1;
            continue;
        };
        let c = corners(quad.rect, quad.angle);
        let uvs = [[u0, v0], [u1, v0], [u0, v1], [u0, v1], [u1, v0], [u1, v1]];
        let start = g.vertices.len() as u32;
        for (i, uv) in [0usize, 1, 2, 2, 1, 3].into_iter().zip(uvs) {
            g.vertices.push(UiVertex {
                position: c[i],
                uv,
                color: quad.color,
            });
        }
        // Join the previous run when nothing about the draw state changed.
        match g.batches.last_mut() {
            Some(batch) if batch.slot == slot && batch.scissor == scissor => batch.count += 6,
            _ => g.batches.push(Batch {
                start,
                count: 6,
                slot,
                scissor,
            }),
        }
    }
    g
}

/// The scissor rectangle of `clip` (or the whole viewport), clamped, in pixels.
pub(crate) fn scissor_rect(clip: Option<[f32; 4]>, viewport: (u32, u32)) -> [u32; 4] {
    let (vw, vh) = (viewport.0 as f32, viewport.1 as f32);
    let Some([x, y, w, h]) = clip else {
        return [0, 0, viewport.0, viewport.1];
    };
    // Round outwards so a partially covered pixel is still drawn.
    let x0 = x.floor().max(0.0).min(vw) as u32;
    let y0 = y.floor().max(0.0).min(vh) as u32;
    let x1 = (x + w).ceil().max(0.0).min(vw) as u32;
    let y1 = (y + h).ceil().max(0.0).min(vh) as u32;
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}

/// The four corners of a quad, clockwise from the top-left, rotated about the centre.
fn corners(rect: [f32; 4], angle: f32) -> [[f32; 2]; 4] {
    let [x, y, w, h] = rect;
    let (cx, cy) = (x + w * 0.5, y + h * 0.5);
    if angle == 0.0 {
        return [[x, y], [x + w, y], [x, y + h], [x + w, y + h]];
    }
    // Screen space has y down, so a positive angle turns clockwise on screen.
    let (sin, cos) = angle.to_radians().sin_cos();
    let rotate = |p: [f32; 2]| {
        let (dx, dy) = (p[0] - cx, p[1] - cy);
        [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]
    };
    [
        rotate([x, y]),
        rotate([x + w, y]),
        rotate([x, y + h]),
        rotate([x + w, y + h]),
    ]
}

/// `[u0, v0, u1, v1]` of a quad, normalized for texel coordinates.
fn uv_range(uv: Uv, size: Option<(u32, u32)>) -> Option<[f32; 4]> {
    match uv {
        Uv::Normalized(range) => Some(range),
        Uv::Texels([x, y, w, h]) => {
            let (tw, th) = size?;
            let (tw, th) = (tw as f32, th as f32);
            if tw == 0.0 || th == 0.0 {
                return None;
            }
            Some([x / tw, y / th, (x + w) / tw, (y + h) / th])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_ui::{Quad, TextureKey};

    fn quad(rect: [f32; 4], texture: Option<TextureKey>) -> Quad {
        Quad {
            rect,
            uv: a3_ui::draw::FULL,
            color: [1.0, 0.5, 0.25, 1.0],
            texture,
            clip: None,
            angle: 0.0,
        }
    }

    /// The corner uv of each of the six vertices, in emission order.
    fn corners_uv(u0: f32, v0: f32, u1: f32, v1: f32) -> [[f32; 2]; 6] {
        [[u0, v0], [u1, v0], [u0, v1], [u0, v1], [u1, v0], [u1, v1]]
    }

    fn info(slot: u32, size: (u32, u32)) -> Option<TextureInfo> {
        Some(TextureInfo { slot, size })
    }

    fn list(quads: Vec<Quad>) -> DrawList {
        let mut list = DrawList::default();
        for q in quads {
            list.quads.push(q);
        }
        list
    }

    fn positions(g: &Geometry) -> Vec<[f32; 2]> {
        g.vertices.iter().map(|v| v.position).collect()
    }

    #[test]
    fn a_solid_quad_becomes_two_triangles_of_one_batch() {
        let g = build_geometry(
            &list(vec![quad([10.0, 20.0, 30.0, 40.0], None)]),
            &[],
            (100, 100),
        );
        assert_eq!(g.vertices.len(), 6);
        assert_eq!(
            positions(&g),
            [
                [10.0, 20.0],
                [40.0, 20.0],
                [10.0, 60.0],
                [10.0, 60.0],
                [40.0, 20.0],
                [40.0, 60.0],
            ]
        );
        let uv = corners_uv(0.0, 0.0, 1.0, 1.0);
        assert!(
            g.vertices.iter().zip(uv).all(|(v, uv)| v.uv == uv),
            "{:?}",
            g.vertices
        );
        assert!(g.vertices.iter().all(|v| v.color == [1.0, 0.5, 0.25, 1.0]));
        assert_eq!(
            g.batches,
            [Batch {
                start: 0,
                count: 6,
                slot: WHITE_SLOT,
                scissor: [0, 0, 100, 100],
            }]
        );
        assert_eq!(g.missing_textures, 0);
        assert_eq!(g.skipped, 0);
    }

    #[test]
    fn texel_uvs_are_normalized_by_the_texture_size() {
        let mut q = quad([0.0, 0.0, 16.0, 16.0], Some(TextureKey(0)));
        q.uv = Uv::Texels([8.0, 4.0, 16.0, 16.0]);
        let g = build_geometry(&list(vec![q]), &[info(1, (64, 32))], (64, 32));
        assert_eq!(g.vertices.len(), 6);
        let uv = corners_uv(8.0 / 64.0, 4.0 / 32.0, 24.0 / 64.0, 20.0 / 32.0);
        assert!(
            g.vertices.iter().zip(uv).all(|(v, uv)| v.uv == uv),
            "{:?}",
            g.vertices
        );
        assert_eq!(g.batches[0].slot, 1);
    }

    #[test]
    fn a_missing_texture_drops_the_quad() {
        let q = quad([0.0, 0.0, 10.0, 10.0], Some(TextureKey(0)));
        let g = build_geometry(&list(vec![q]), &[None], (100, 100));
        assert!(g.vertices.is_empty() && g.batches.is_empty());
        assert_eq!(g.missing_textures, 1);
    }

    #[test]
    fn empty_transparent_and_clipped_away_quads_are_skipped() {
        let mut invisible = quad([0.0, 0.0, 10.0, 10.0], None);
        invisible.color[3] = 0.0;
        let mut flat = quad([0.0, 0.0, 0.0, 10.0], None);
        flat.color[3] = 1.0;
        let mut away = quad([0.0, 0.0, 10.0, 10.0], None);
        away.clip = Some([200.0, 200.0, 10.0, 10.0]);
        let g = build_geometry(&list(vec![invisible, flat, away]), &[], (100, 100));
        assert!(g.vertices.is_empty() && g.batches.is_empty(), "{g:?}");
        assert_eq!(g.skipped, 3);
    }

    #[test]
    fn clip_becomes_a_clamped_scissor_rectangle() {
        let mut inside = quad([0.0, 0.0, 10.0, 10.0], None);
        inside.clip = Some([10.4, 20.6, 30.0, 40.0]);
        // Sticks out of the viewport on the top-left and the bottom-right.
        let mut outside = quad([0.0, 0.0, 10.0, 10.0], None);
        outside.clip = Some([-5.2, -3.0, 40.0, 50.0]);
        let g = build_geometry(&list(vec![inside, outside]), &[], (100, 100));
        assert_eq!(g.batches.len(), 2);
        assert_eq!(g.batches[0].scissor, [10, 20, 31, 41]);
        assert_eq!(g.batches[1].scissor, [0, 0, 35, 47]);
    }

    #[test]
    fn batches_join_only_consecutive_quads_with_the_same_texture_and_clip() {
        let clip = Some([0.0, 0.0, 50.0, 50.0]);
        let mut b = quad([0.0, 0.0, 10.0, 10.0], Some(TextureKey(1)));
        b.clip = clip;
        let mut c = quad([10.0, 0.0, 10.0, 10.0], Some(TextureKey(1)));
        c.clip = clip;
        let other = quad([20.0, 0.0, 10.0, 10.0], Some(TextureKey(0)));
        let a = quad([30.0, 0.0, 10.0, 10.0], Some(TextureKey(1)));
        let g = build_geometry(
            &list(vec![a.clone(), b, c, other, a]),
            // Key 0 resolves to slot 1 and key 1 to slot 2.
            &[info(1, (8, 8)), info(2, (8, 8))],
            (100, 100),
        );
        let starts: Vec<(u32, u32, u32)> = g
            .batches
            .iter()
            .map(|b| (b.start, b.count, b.slot))
            .collect();
        // a (unclipped, slot 2), b+c (clipped, slot 2), other (slot 1), a again.
        assert_eq!(starts, [(0, 6, 2), (6, 12, 2), (18, 6, 1), (24, 6, 2)]);
        assert_eq!(g.batches[1].scissor, [0, 0, 50, 50]);
        assert_eq!(g.batches[0].scissor, [0, 0, 100, 100]);
    }

    #[test]
    fn rotation_turns_the_quad_about_its_centre() {
        let mut q = quad([0.0, 0.0, 10.0, 10.0], None);
        q.angle = 90.0;
        let g = build_geometry(&list(vec![q]), &[], (100, 100));
        let p = positions(&g);
        // Clockwise: the top-left corner turns into the top-right one, and so on.
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        assert!(close(p[0], [10.0, 0.0]), "{p:?}");
        assert!(close(p[1], [10.0, 10.0]), "{p:?}");
        assert!(close(p[2], [0.0, 0.0]), "{p:?}");
        assert!(close(p[5], [0.0, 10.0]), "{p:?}");
    }

    #[test]
    fn rotation_about_a_centre_off_the_screen_origin_keeps_the_centre() {
        let mut q = quad([100.0, 50.0, 20.0, 10.0], None);
        q.angle = 180.0;
        let g = build_geometry(&list(vec![q]), &[], (200, 200));
        let p = positions(&g);
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        assert!(close(p[0], [120.0, 60.0]), "{p:?}");
        assert!(close(p[5], [100.0, 50.0]), "{p:?}");
    }

    #[test]
    fn normalized_uvs_pass_through() {
        let mut q = quad([0.0, 0.0, 4.0, 4.0], Some(TextureKey(0)));
        q.uv = Uv::Normalized([0.25, 0.5, 0.75, 1.0]);
        let g = build_geometry(&list(vec![q]), &[info(1, (2, 2))], (10, 10));
        let uv = corners_uv(0.25, 0.5, 0.75, 1.0);
        assert!(
            g.vertices.iter().zip(uv).all(|(v, uv)| v.uv == uv),
            "{:?}",
            g.vertices
        );
    }
}
