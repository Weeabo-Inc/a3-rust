//! ODOL model-wide data: ModelInfo, Skeleton and binarised animations.

use crate::error::Result;
use crate::model::{
    Animation, AnimationAxis, AnimationBinding, AnimationTransform, Bone, ModelInfo, Skeleton,
    SpecialLods, ThermalParams,
};
use crate::reader::Reader;

pub(crate) fn model_info(
    r: &mut Reader,
    lod_count: usize,
) -> Result<(ModelInfo, Option<Skeleton>)> {
    let mut m = ModelInfo {
        special_flags: r.u32()?,
        bounding_sphere: r.f32()?,
        geometry_sphere: r.f32()?,
        remarks: r.u32()?,
        and_hints: r.u32()?,
        or_hints: r.u32()?,
        aiming_center: r.vec3()?,
        color: r.u32()?,
        color_type: r.u32()?,
        view_density: r.f32()?,
        bbox_min: r.vec3()?,
        bbox_max: r.vec3()?,
        lod_density_coef: r.f32()?,
        draw_importance: r.f32()?,
        bbox_visual_min: r.vec3()?,
        bbox_visual_max: r.vec3()?,
        bounding_center: r.vec3()?,
        geometry_center: r.vec3()?,
        center_of_mass: r.vec3()?,
        inv_inertia: r.mat3()?,
        auto_center: r.bool()?,
        lock_auto_center: r.bool()?,
        can_occlude: r.bool()?,
        can_be_occluded: r.bool()?,
        ai_covers: r.bool()?,
        thermal: ThermalParams {
            ht_min: r.f32()?,
            ht_max: r.f32()?,
            af_max: r.f32()?,
            mf_max: r.f32()?,
            m_fact: r.f32()?,
            t_body: r.f32()?,
        },
        force_not_alpha: r.bool()?,
        shadow_source: r.i32()?.into(),
        prefer_shadow_volume: r.bool()?,
        shadow_offset: r.f32()?,
        animated: r.bool()?,
        ..ModelInfo::default()
    };
    let skeleton = skeleton(r)?;
    m.map_type = r.u8()?;
    m.mass_array = mass_array(r)?;
    m.mass = r.f32()?;
    m.inv_mass = r.f32()?;
    m.armor = r.f32()?;
    m.inv_armor = r.f32()?;
    m.explosion_shielding = r.f32()?;
    let mut lod_index = || -> Result<Option<u8>> {
        let i = r.i8()?;
        Ok(u8::try_from(i).ok())
    };
    m.special_lods = SpecialLods {
        memory: lod_index()?,
        geometry: lod_index()?,
        geometry_simple: lod_index()?,
        geometry_physx: lod_index()?,
        fire_geometry: lod_index()?,
        view_geometry: lod_index()?,
        view_pilot_geometry: lod_index()?,
        view_gunner_geometry: lod_index()?,
        view_commander_geometry: lod_index()?,
        view_cargo_geometry: lod_index()?,
        land_contact: lod_index()?,
        roadway: lod_index()?,
        paths: lod_index()?,
        hitpoints: lod_index()?,
    };
    m.min_shadow = r.u32()?;
    m.can_blend = r.bool()?;
    m.class = r.asciiz()?;
    m.damage = r.asciiz()?;
    m.frequent = r.bool()?;
    let n = r.count(1)?;
    m.obsolete_names = (0..n).map(|_| r.asciiz()).collect::<Result<_>>()?;
    let mut per_lod = || (0..lod_count).map(|_| r.i32()).collect::<Result<Vec<_>>>();
    m.preferred_shadow_volume_lod = per_lod()?;
    m.preferred_shadow_buffer_lod = per_lod()?;
    m.preferred_shadow_buffer_lod_visible = per_lod()?;
    Ok((m, skeleton))
}

/// The mass array is a compressed array; every shipped model stores it empty. A non-empty one
/// needs the LZO path of the LOD reader and is rejected here until that lands.
fn mass_array(r: &mut Reader) -> Result<Vec<f32>> {
    let n = r.count(4)?;
    if n == 0 {
        return Ok(Vec::new());
    }
    if r.u8()? != 0 {
        return Err(r.malformed("compressed mass array is not supported yet"));
    }
    (0..n).map(|_| r.f32()).collect()
}

fn skeleton(r: &mut Reader) -> Result<Option<Skeleton>> {
    let name = r.asciiz()?;
    if name.is_empty() {
        return Ok(None);
    }
    let inherited = r.bool()?;
    let n = r.count(2)?;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        names.push((r.asciiz()?, r.asciiz()?));
    }
    let mut bones = Vec::with_capacity(n);
    for (bone, parent_name) in &names {
        let parent = names
            .iter()
            .position(|(b, _)| !parent_name.is_empty() && b.eq_ignore_ascii_case(parent_name));
        bones.push(Bone {
            name: bone.clone(),
            parent,
            parent_name: parent_name.clone(),
        });
    }
    let pivots_model = r.asciiz()?;
    Ok(Some(Skeleton {
        name,
        inherited,
        bones,
        pivots_model,
    }))
}

/// Per bone: the indices of the animations acting on it, for one LOD.
pub(crate) type BoneAnimations = Vec<Vec<u32>>;

const HIDE: u32 = 9;
const DIRECT: u32 = 8;

/// Animation classes, then per LOD the bone → animations table, then per LOD and animation
/// the bone and axis.
pub(crate) fn animations(
    r: &mut Reader,
    lod_count: usize,
    skeleton: Option<&Skeleton>,
) -> Result<(Vec<Animation>, Vec<BoneAnimations>)> {
    let n = r.count(4)?;
    let mut kinds = Vec::with_capacity(n);
    let mut anims = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = r.u32()?;
        let name = r.asciiz()?;
        let source = r.asciiz()?;
        let min_value = r.f32()?;
        let max_value = r.f32()?;
        let min_phase = r.f32()?;
        let max_phase = r.f32()?;
        let anim_period = r.f32()?;
        let init_phase = r.f32()?;
        let source_address = r.u32()?.into();
        let axis = |k: u32| match k % 4 {
            0 => AnimationAxis::Custom,
            1 => AnimationAxis::X,
            2 => AnimationAxis::Y,
            _ => AnimationAxis::Z,
        };
        let transform = match kind {
            0..=3 => AnimationTransform::Rotation {
                axis: axis(kind),
                angle0: r.f32()?,
                angle1: r.f32()?,
            },
            4..=7 => AnimationTransform::Translation {
                axis: axis(kind),
                offset0: r.f32()?,
                offset1: r.f32()?,
            },
            DIRECT => AnimationTransform::Direct {
                axis_pos: r.vec3()?,
                axis_dir: r.vec3()?,
                angle: r.f32()?,
                axis_offset: r.f32()?,
            },
            HIDE => AnimationTransform::Hide {
                hide_value: r.f32()?,
                unhide_value: r.f32()?,
            },
            other => return Err(r.malformed(format!("unknown animation type {other}"))),
        };
        kinds.push(kind);
        anims.push(Animation {
            name,
            source,
            transform,
            min_value,
            max_value,
            min_phase,
            max_phase,
            anim_period,
            init_phase,
            source_address,
            bindings: Vec::with_capacity(lod_count),
        });
    }

    // Some models store zero tables: no animation binds to a bone in any LOD.
    let resolutions = r.count(4)?;
    if resolutions == 0 {
        for anim in &mut anims {
            anim.bindings = vec![None; lod_count];
        }
        return Ok((anims, vec![Vec::new(); lod_count]));
    }
    if resolutions != lod_count {
        return Err(r.malformed(format!(
            "animation tables for {resolutions} LODs, model has {lod_count}"
        )));
    }
    let bone_count = skeleton.map_or(0, |s| s.bones.len());
    let mut bones_to_anims = Vec::with_capacity(lod_count);
    for _ in 0..lod_count {
        let bones = r.count(4)?;
        if bones > bone_count {
            return Err(r.malformed(format!(
                "{bones} bones in an animation table, skeleton has {bone_count}"
            )));
        }
        let mut table = Vec::with_capacity(bones);
        for _ in 0..bones {
            let k = r.count(4)?;
            let list = (0..k).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
            if let Some(&bad) = list.iter().find(|&&a| a as usize >= n) {
                return Err(r.malformed(format!("animation index {bad} out of range")));
            }
            table.push(list);
        }
        bones_to_anims.push(table);
    }
    for _ in 0..lod_count {
        for (anim, &kind) in anims.iter_mut().zip(&kinds) {
            let bone = r.i32()?;
            let binding = match u32::try_from(bone) {
                Err(_) => None,
                Ok(bone) => {
                    let axis = if kind == HIDE || kind == DIRECT {
                        None
                    } else {
                        Some((r.vec3()?, r.vec3()?))
                    };
                    Some(AnimationBinding { bone, axis })
                }
            };
            anim.bindings.push(binding);
        }
    }
    Ok((anims, bones_to_anims))
}
