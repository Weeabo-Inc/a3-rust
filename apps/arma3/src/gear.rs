//! The Man's gear: what a soldier class wears and carries, resolved from the config, and where
//! each piece goes on his posed body.
//!
//! Two kinds of piece, told apart by the data (`docs/re/model-animations.md`, "Gear on a Man"):
//!
//! - **Worn** models — the head, the vest, the helmet, the NVG — carry the Man's own Skeleton
//!   (`OFP2_ManSkeleton`) and are modelled in the pivots model's space. They are skinned with
//!   the Man's pose, bone by bone by name, and placed at his origin; an autocentred one (the
//!   head) is offset by its own `bounding_center`.
//! - **Held** models — the primary weapon — hang on a proxy of the Man's model that follows one
//!   bone rigidly (`proxy:\a3\characters_f\proxies\weapon` on the `weapon` bone). A weapon's
//!   attachments (optic, pointer) hang the same way on the weapon's slot proxies
//!   (`WeaponSlotsInfo >> <slot> >> linkProxy`). A proxy's 3x3 orientation is column-major.

use a3_config::{ConfigRef, ConfigTree, Value};
use a3_p3d::{Model, Skeleton};
use a3_pose::ManRig;
use a3_render_models::{ModelId, ModelRenderer, PlacedObject};
use a3_vfs::Vfs;
use glam::{Affine3A, DAffine3, Vec3};

use crate::man::{ManAnimation, vfs_path};

/// The proxy of a Man's model that holds his primary weapon.
pub const WEAPON_PROXY: &str = r"\a3\characters_f\proxies\weapon";

/// What a Man of one `CfgVehicles` class wears and holds, as model paths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loadout {
    /// The head from his identity, with the face animation that keeps its face rig on it.
    pub head: Option<Head>,
    /// Linked items drawn on the body (vest, helmet, NVG), in `linkedItems` order.
    pub worn: Vec<String>,
    /// The first rifle of `weapons[]`, in his hands.
    pub primary: Option<Weapon>,
}

/// A head model and its neutral face animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    /// `CfgHeads >> <head> >> model`.
    pub model: String,
    /// The face RTM the head's `Grimaces` play when nothing else does (`NeutralFace >> anim`).
    pub face: Option<String>,
}

/// A weapon model and the attachments on its slot proxies.
#[derive(Debug, Clone, PartialEq)]
pub struct Weapon {
    /// `CfgWeapons >> <weapon> >> model`.
    pub model: String,
    /// Per linked attachment: the weapon model's proxy it hangs on, and its model.
    pub attachments: Vec<Attachment>,
}

/// An attachment on one of a weapon's slot proxies.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// The weapon's slot proxy (`linkProxy`), e.g. `\A3\data_f\proxies\weapon_slots\TOP`.
    pub proxy: String,
    /// The attachment's model.
    pub model: String,
}

/// The loadout of `CfgVehicles >> class`.
pub fn loadout(config: &ConfigTree, class: &str) -> Loadout {
    let root = config.root();
    let man = root.get("CfgVehicles").get(class);
    let weapons = root.get("CfgWeapons");
    let worn = strings(&man.get("linkedItems"))
        .iter()
        .filter_map(|item| {
            let model = weapons.get(item).get("ItemInfo").get("uniformModel").text();
            // `-` is "nothing to draw" (a uniform's own item).
            (!model.is_empty() && model != "-").then_some(model)
        })
        .collect();
    let primary = strings(&man.get("weapons")).iter().find_map(|name| {
        let weapon = weapons.get(name);
        // `type = 1`: a primary weapon (rifle).
        (weapon.get("type").number() == 1.0).then(|| Weapon {
            model: weapon.get("model").text(),
            attachments: attachments(&weapons, &weapon),
        })
    });
    Loadout {
        head: head(config, &man),
        worn,
        primary,
    }
}

/// The head of a Man: the first face of his `faceType` in config order that is not disabled
/// and shares one of his `identityTypes`. (The engine picks among those at random; the first
/// one keeps screenshots stable.)
fn head(config: &ConfigTree, man: &ConfigRef<'_>) -> Option<Head> {
    let root = config.root();
    let identities: Vec<String> = strings(&man.get("identityTypes"))
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let faces = root.get("CfgFaces").get(&man.get("faceType").text());
    let face = faces.entries().into_iter().find(|face| {
        face.is_class()
            && face.get("disabled").number() != 1.0
            && strings(&face.get("identityTypes"))
                .iter()
                .any(|t| identities.contains(&t.to_ascii_lowercase()))
    })?;
    let head = root.get("CfgHeads").get(&face.get("head").text());
    let model = head.get("model").text();
    if model.is_empty() {
        return None;
    }
    let face_rtm = head.get("Grimaces").get("NeutralFace").get("anim").text();
    Some(Head {
        model,
        face: (!face_rtm.is_empty()).then_some(face_rtm),
    })
}

/// The linked attachments of `weapon` (`LinkedItems >> * >> slot / item`) on their slots'
/// proxies.
fn attachments(weapons: &ConfigRef<'_>, weapon: &ConfigRef<'_>) -> Vec<Attachment> {
    weapon
        .get("LinkedItems")
        .entries_with_inherited()
        .into_iter()
        .filter(|link| link.is_class())
        .filter_map(|link| {
            let proxy = weapon
                .get("WeaponSlotsInfo")
                .get(&link.get("slot").text())
                .get("linkProxy")
                .text();
            let model = weapons.get(&link.get("item").text()).get("model").text();
            (!proxy.is_empty() && !model.is_empty()).then_some(Attachment { proxy, model })
        })
        .collect()
}

/// The strings of a config array (other values left out).
fn strings(entry: &ConfigRef<'_>) -> Vec<String> {
    entry
        .array()
        .into_iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// A worn model, loaded: its renderer id, its offset from the pivots origin and its Skeleton
/// (the palette is built in its own bone order).
struct WornPart {
    model: ModelId,
    offset: Vec3,
    skeleton: Skeleton,
}

/// A model held on a proxy, loaded: its renderer id, the proxy's frame in the host's model
/// space, the host bone the proxy follows (`None` for a host that is not posed) and its own
/// offset.
struct HeldPart {
    model: ModelId,
    proxy: Affine3A,
    bone: Option<usize>,
    offset: Vec3,
}

/// A Man's gear, loaded into a model renderer and ready to follow his pose every frame.
pub struct ManGear {
    worn: Vec<WornPart>,
    /// The primary weapon, then its attachments (held by the weapon, not by the Man).
    weapon: Option<(HeldPart, Vec<HeldPart>)>,
}

impl ManGear {
    /// Load `loadout`'s models for the Man model `man_model` (VFS path) into `models`. Pieces
    /// whose model is missing from the game data, or that the Man's model has no proxy for, are
    /// left out with a warning.
    pub fn load(vfs: &Vfs, models: &mut ModelRenderer, loadout: &Loadout, man_model: &str) -> Self {
        let read = |path: &str| -> Option<Model> {
            let model = vfs
                .open(&vfs_path(path))
                .ok()
                .and_then(|bytes| Model::from_bytes(&bytes).ok());
            if model.is_none() {
                log::warn!("gear model {path} is not in the game data; not drawn");
            }
            model
        };
        let mut worn = Vec::new();
        let head = loadout.head.as_ref().map(|h| h.model.as_str());
        for path in head
            .into_iter()
            .chain(loadout.worn.iter().map(String::as_str))
        {
            let Some(model) = read(path) else { continue };
            let Some(skeleton) = model.skeleton.clone() else {
                log::warn!("worn model {path} has no Skeleton; not drawn");
                continue;
            };
            let id = models.model(&vfs_path(path));
            models.preload(id);
            worn.push(WornPart {
                model: id,
                offset: own_offset(&model),
                skeleton,
            });
        }
        let weapon = loadout.primary.as_ref().and_then(|weapon| {
            let host = read(man_model)?;
            let held = held_part(models, &host, WEAPON_PROXY, &weapon.model, &read)?;
            let weapon_model = read(&weapon.model)?;
            let attachments = weapon
                .attachments
                .iter()
                .filter_map(|a| {
                    let mut part = held_part(models, &weapon_model, &a.proxy, &a.model, &read)?;
                    // The attachment hangs on the weapon, which is not posed.
                    part.bone = None;
                    Some(part)
                })
                .collect();
            Some((held, attachments))
        });
        ManGear { worn, weapon }
    }

    /// Put the gear on a Man whose model is at `man` (instance transform, his model's space)
    /// with `composed` pose (pivot space) on `rig`; `man_offset` is his model's offset.
    pub fn place(
        &self,
        models: &mut ModelRenderer,
        man: DAffine3,
        rig: &ManRig,
        composed: &[Affine3A],
        man_offset: Vec3,
    ) {
        for part in &self.worn {
            models.add_skinned(
                PlacedObject {
                    model: part.model,
                    transform: worn_placement(man, man_offset, part.offset),
                },
                &rig.palette_for(composed, &part.skeleton, part.offset),
            );
        }
        let Some((weapon, attachments)) = &self.weapon else {
            return;
        };
        let palette = rig.palette(composed, man_offset);
        let bone = weapon
            .bone
            .and_then(|b| palette.get(b).copied())
            .unwrap_or(Affine3A::IDENTITY);
        let held = held_placement(man, bone, weapon.proxy, weapon.offset);
        models.add_dynamic(PlacedObject {
            model: weapon.model,
            transform: held,
        });
        // The weapon's own origin, for its attachments: its placement without its offset.
        let weapon_origin = held * DAffine3::from_translation(-weapon.offset.as_dvec3());
        for a in attachments {
            models.add_dynamic(PlacedObject {
                model: a.model,
                transform: held_placement(weapon_origin, Affine3A::IDENTITY, a.proxy, a.offset),
            });
        }
    }
}

/// Put a posed Man and his gear in this frame: his model `body` with its ground point on
/// `transform` (the entity's position and heading), skinned with `man`'s pose, and `gear` on
/// him. `false` (nothing placed) when his Move is not loaded yet.
pub fn place_man(
    models: &mut ModelRenderer,
    body: ModelId,
    man: &ManAnimation,
    gear: Option<&ManGear>,
    transform: DAffine3,
) -> bool {
    let Some(composed) = man.composed() else {
        return false;
    };
    let offset = man.model_offset();
    let placed = crate::scene::ground_placement(transform, man.ground());
    models.add_skinned(
        PlacedObject {
            model: body,
            transform: placed,
        },
        &man.rig().palette(&composed, offset),
    );
    if let Some(gear) = gear {
        gear.place(models, placed, man.rig(), &composed, offset);
    }
    true
}

/// A model held on `host`'s proxy named `proxy` (first Resolution LOD), loaded into `models`.
fn held_part(
    models: &mut ModelRenderer,
    host: &Model,
    proxy: &str,
    path: &str,
    read: &dyn Fn(&str) -> Option<Model>,
) -> Option<HeldPart> {
    let Some(p) = host
        .lods
        .first()
        .and_then(|lod| lod.proxies.iter().find(|p| same_proxy(&p.model, proxy)))
    else {
        log::warn!("no proxy {proxy} to hold {path}; not drawn");
        return None;
    };
    let model = read(path)?;
    let id = models.model(&vfs_path(path));
    models.preload(id);
    Some(HeldPart {
        model: id,
        proxy: Affine3A::from_mat3_translation(p.orientation, p.position),
        bone: usize::try_from(p.bone).ok(),
        offset: own_offset(&model),
    })
}

/// Where a model's stored vertices sit from its own origin: its `bounding_center` when the
/// model is autocentred, else nothing.
fn own_offset(model: &Model) -> Vec3 {
    if model.info.auto_center {
        model.info.bounding_center
    } else {
        Vec3::ZERO
    }
}

/// Whether two proxy paths name the same proxy (case, slashes and a leading separator do not
/// matter).
pub fn same_proxy(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim_start_matches(['\\', '/'])
            .trim_end_matches(".p3d")
            .replace('/', "\\")
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Where a worn model goes: the Man's instance transform `man` (his model's space) moved so
/// the worn model's own origin, `part_offset` from the pivots origin, lands where the Man's
/// model has it (`man_offset` is the Man's own offset, his `bounding_center`).
pub fn worn_placement(man: DAffine3, man_offset: Vec3, part_offset: Vec3) -> DAffine3 {
    man * DAffine3::from_translation((part_offset - man_offset).as_dvec3())
}

/// Where a model held on a proxy goes: the host's instance transform, then the host bone's
/// palette matrix (identity for a host that is not posed), then the proxy's frame in the
/// host's model space, then the held model's own `part_offset` (its `bounding_center` when
/// autocentred).
pub fn held_placement(
    host: DAffine3,
    bone: Affine3A,
    proxy: Affine3A,
    part_offset: Vec3,
) -> DAffine3 {
    let local = bone * proxy * Affine3A::from_translation(part_offset);
    host * DAffine3 {
        matrix3: local.matrix3.as_dmat3(),
        translation: local.translation.as_dvec3(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;
    use glam::{DVec3, Mat3};

    const CONFIG: &str = r#"
class CfgVehicles {
    class Man_Base { faceType = "Man_A3"; identityTypes[] = {"Head_NATO"}; };
    class B_Soldier_F: Man_Base {
        weapons[] = {"hgun_P07_F", "arifle_MX_ACO_pointer_F", "Throw"};
        linkedItems[] = {"V_PlateCarrier1_rgr", "H_HelmetB", "ItemMap"};
    };
};
class CfgFaces {
    class Man_A3 {
        class Default { disabled = 1; identityTypes[] = {"Head_NATO"}; head = "Bad"; };
        class AfricanHead_01: Default { disabled = 0; identityTypes[] = {"Head_African"}; head = "AfricanHead_A3"; };
        class WhiteHead_01: Default { disabled = 0; identityTypes[] = {"Head_NATO", "Head_Euro"}; head = "NATOHead_A3"; };
    };
};
class CfgHeads {
    class NATOHead_A3 {
        model = "\A3\Characters_F\Heads\m_white_01";
        class Grimaces { class NeutralFace { anim = "A3\Characters_F\Heads\Anim\male\Neutral.rtm"; }; };
    };
};
class CfgWeapons {
    class ItemCore { class ItemInfo { uniformModel = ""; }; };
    class V_PlateCarrier1_rgr: ItemCore { class ItemInfo { uniformModel = "\A3\Characters_F\BLUFOR\equip_b_vest02"; }; };
    class H_HelmetB: ItemCore { class ItemInfo { uniformModel = "\A3\Characters_F\BLUFOR\headgear_b_helmet_plain"; }; };
    class ItemMap: ItemCore {};
    class hgun_P07_F { type = 2; model = "\A3\Weapons_F\Pistols\P07\p07_F.p3d"; };
    class arifle_MX_F {
        type = 1;
        model = "\A3\Weapons_F\Rifles\MX\MX_F.p3d";
        class WeaponSlotsInfo {
            class CowsSlot { linkProxy = "\A3\data_f\proxies\weapon_slots\TOP"; };
            class PointerSlot { linkProxy = "\A3\data_f\proxies\weapon_slots\SIDE"; };
        };
    };
    class arifle_MX_ACO_pointer_F: arifle_MX_F {
        class LinkedItems {
            class LinkedItemsOptic { slot = "CowsSlot"; item = "optic_Aco"; };
            class LinkedItemsAcc { slot = "PointerSlot"; item = "acc_pointer_IR"; };
        };
    };
    class optic_Aco { model = "\A3\weapons_f\acc\acco_Aco_F"; };
    class acc_pointer_IR { model = "\A3\weapons_f\acc\accv_pointer_F"; };
};
"#;

    fn config() -> ConfigTree {
        ConfigTree::from_config(&parse_text(CONFIG).unwrap())
    }

    #[test]
    fn the_soldier_wears_his_linked_items_and_holds_his_rifle() {
        let l = loadout(&config(), "B_Soldier_F");
        assert_eq!(
            l.worn,
            [
                r"\A3\Characters_F\BLUFOR\equip_b_vest02",
                r"\A3\Characters_F\BLUFOR\headgear_b_helmet_plain"
            ],
            "the map has no model to wear"
        );
        let primary = l.primary.unwrap();
        assert_eq!(primary.model, r"\A3\Weapons_F\Rifles\MX\MX_F.p3d");
        assert_eq!(
            primary.attachments,
            [
                Attachment {
                    proxy: r"\A3\data_f\proxies\weapon_slots\TOP".into(),
                    model: r"\A3\weapons_f\acc\acco_Aco_F".into(),
                },
                Attachment {
                    proxy: r"\A3\data_f\proxies\weapon_slots\SIDE".into(),
                    model: r"\A3\weapons_f\acc\accv_pointer_F".into(),
                },
            ]
        );
    }

    #[test]
    fn the_head_is_the_first_enabled_face_of_his_identity() {
        let head = loadout(&config(), "B_Soldier_F").head.unwrap();
        assert_eq!(head.model, r"\A3\Characters_F\Heads\m_white_01");
        assert_eq!(
            head.face.as_deref(),
            Some(r"A3\Characters_F\Heads\Anim\male\Neutral.rtm")
        );
        assert_eq!(loadout(&config(), "NoSuchClass"), Loadout::default());
    }

    #[test]
    fn proxy_paths_compare_loosely() {
        assert!(same_proxy(
            r"\a3\characters_f\proxies\weapon",
            r"A3/Characters_F/Proxies/Weapon.p3d"
        ));
        assert!(!same_proxy(
            r"\a3\characters_f\proxies\weapon",
            r"\a3\x\pistol"
        ));
    }

    #[test]
    fn a_worn_model_lands_on_the_mans_pivot_origin() {
        let man = DAffine3::from_translation(DVec3::new(100.0, 5.0, 200.0));
        let man_offset = Vec3::new(0.3, 0.8, -0.4);
        // The head is autocentred 0.4 m up; a vest is not.
        let head = worn_placement(man, man_offset, Vec3::new(0.0, 0.4, 0.0));
        // The head's vertex at its own origin is the pivot point (0, 0.4, 0), which is
        // (0, 0.4, 0) - man_offset in the Man's model.
        let p = head.transform_point3(DVec3::ZERO);
        let expected = man.transform_point3((Vec3::new(0.0, 0.4, 0.0) - man_offset).as_dvec3());
        assert!((p - expected).length() < 1e-9);
        let vest = worn_placement(man, man_offset, Vec3::ZERO);
        assert!(
            (vest.transform_point3(DVec3::ZERO) - man.transform_point3(-man_offset.as_dvec3()))
                .length()
                < 1e-9
        );
    }

    #[test]
    fn a_held_model_follows_its_bone_and_its_proxy_frame() {
        // The soldier's weapon proxy: a quarter turn about Y, columns (0,0,1), (0,1,0), (-1,0,0).
        let proxy = Affine3A::from_mat3_translation(
            Mat3::from_cols(Vec3::Z, Vec3::Y, -Vec3::X),
            Vec3::new(-1.3, -0.8, 0.4),
        );
        let bone = Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let host = DAffine3::IDENTITY;
        let held = held_placement(host, bone, proxy, Vec3::ZERO);
        // The rifle's muzzle is at -x in its own model: it points along -z, the Man's front.
        let muzzle = held.transform_vector3(DVec3::new(-1.0, 0.0, 0.0));
        assert!(
            (muzzle - DVec3::new(0.0, 0.0, -1.0)).length() < 1e-6,
            "{muzzle}"
        );
        let origin = held.transform_point3(DVec3::ZERO);
        assert!(
            (origin - DVec3::new(-1.3, 0.2, 0.4)).length() < 1e-6,
            "{origin}"
        );
    }
}
