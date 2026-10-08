//! LOD resolution values and the special LODs they name.

use std::fmt;

/// The resolution value that identifies a LOD inside a P3D.
///
/// Values below 1000 are Resolution LODs (visual detail levels, lower is more detailed). Fixed
/// reserved values and ranges name the Special LODs; see [`LodKind`].
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct LodResolution(pub f32);

/// What a [`LodResolution`] stands for.
///
/// Values from Object Builder's LOD list and the BI community wiki; every special value seen in
/// the 2.22 install maps to a named kind here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LodKind {
    /// A visual Resolution LOD with its resolution (`0.0 ..< 1000.0`).
    Resolution(f32),
    /// First-person view of a gunner (`1000`).
    ViewGunner,
    /// First-person view of a pilot or driver (`1100`).
    ViewPilot,
    /// First-person view of cargo passengers (`1200`).
    ViewCargo,
    /// Shadow volume with its sub-level (`10000 + n`).
    ShadowVolume(f32),
    /// Shadow buffer with its sub-level (`11000 + n`) _(uncertain name)_.
    ShadowBuffer(f32),
    /// Editor-only LOD with its sub-level (`20000 + n`).
    Edit(f32),
    /// Collision and mass (`1e13`).
    Geometry,
    /// Buoyancy geometry (`2e13`).
    GeometryBuoyancy,
    /// Old PhysX geometry (`3e13`).
    GeometryPhysxOld,
    /// PhysX geometry (`4e13`).
    GeometryPhysx,
    /// Named points: axes, attachment and muzzle positions (`1e15`).
    Memory,
    /// Ground contact points (`2e15`).
    LandContact,
    /// Walkable surfaces (`3e15`).
    Roadway,
    /// AI paths through buildings (`4e15`).
    Paths,
    /// Hit point positions (`5e15`).
    HitPoints,
    /// AI and player line-of-sight geometry (`6e15`).
    ViewGeometry,
    /// Bullet hit geometry (`7e15`).
    FireGeometry,
    /// View geometry for cargo view (`8e15`).
    ViewCargoGeometry,
    /// Fire geometry for cargo view (`9e15`).
    ViewCargoFireGeometry,
    /// First-person view of a commander (`1e16`).
    ViewCommander,
    /// View geometry for commander view (`1.1e16`).
    ViewCommanderGeometry,
    /// Fire geometry for commander view (`1.2e16`).
    ViewCommanderFireGeometry,
    /// View geometry for pilot view (`1.3e16`).
    ViewPilotGeometry,
    /// Fire geometry for pilot view (`1.4e16`).
    ViewPilotFireGeometry,
    /// View geometry for gunner view (`1.5e16`).
    ViewGunnerGeometry,
    /// Fire geometry for gunner view (`1.6e16`).
    ViewGunnerFireGeometry,
    /// Sub-parts (`1.7e16`).
    SubParts,
    /// Shadow volume for cargo view (`1.8e16`).
    ShadowVolumeViewCargo,
    /// Shadow volume for pilot view (`1.9e16`).
    ShadowVolumeViewPilot,
    /// Shadow volume for gunner view (`2e16`).
    ShadowVolumeViewGunner,
    /// Wreck (`2.1e16`).
    Wreck,
    /// A value outside every known range.
    Unknown(f32),
}

const SPECIAL: &[(f32, LodKind, &str)] = &[
    (1e13, LodKind::Geometry, "Geometry"),
    (2e13, LodKind::GeometryBuoyancy, "Geometry Buoyancy"),
    (3e13, LodKind::GeometryPhysxOld, "Geometry PhysX (old)"),
    (4e13, LodKind::GeometryPhysx, "Geometry PhysX"),
    (1e15, LodKind::Memory, "Memory"),
    (2e15, LodKind::LandContact, "Land Contact"),
    (3e15, LodKind::Roadway, "Roadway"),
    (4e15, LodKind::Paths, "Paths"),
    (5e15, LodKind::HitPoints, "Hit-points"),
    (6e15, LodKind::ViewGeometry, "View Geometry"),
    (7e15, LodKind::FireGeometry, "Fire Geometry"),
    (8e15, LodKind::ViewCargoGeometry, "View Cargo Geometry"),
    (
        9e15,
        LodKind::ViewCargoFireGeometry,
        "View Cargo Fire Geometry",
    ),
    (1e16, LodKind::ViewCommander, "View Commander"),
    (
        1.1e16,
        LodKind::ViewCommanderGeometry,
        "View Commander Geometry",
    ),
    (
        1.2e16,
        LodKind::ViewCommanderFireGeometry,
        "View Commander Fire Geometry",
    ),
    (1.3e16, LodKind::ViewPilotGeometry, "View Pilot Geometry"),
    (
        1.4e16,
        LodKind::ViewPilotFireGeometry,
        "View Pilot Fire Geometry",
    ),
    (1.5e16, LodKind::ViewGunnerGeometry, "View Gunner Geometry"),
    (
        1.6e16,
        LodKind::ViewGunnerFireGeometry,
        "View Gunner Fire Geometry",
    ),
    (1.7e16, LodKind::SubParts, "Sub Parts"),
    (
        1.8e16,
        LodKind::ShadowVolumeViewCargo,
        "Shadow Volume View Cargo",
    ),
    (
        1.9e16,
        LodKind::ShadowVolumeViewPilot,
        "Shadow Volume View Pilot",
    ),
    (
        2e16,
        LodKind::ShadowVolumeViewGunner,
        "Shadow Volume View Gunner",
    ),
    (2.1e16, LodKind::Wreck, "Wreck"),
];

impl LodResolution {
    /// Classifies the value.
    pub fn kind(self) -> LodKind {
        let v = self.0;
        match v {
            _ if (0.0..1000.0).contains(&v) => LodKind::Resolution(v),
            1000.0 => LodKind::ViewGunner,
            1100.0 => LodKind::ViewPilot,
            1200.0 => LodKind::ViewCargo,
            _ if (10000.0..11000.0).contains(&v) => LodKind::ShadowVolume(v - 10000.0),
            _ if (11000.0..20000.0).contains(&v) => LodKind::ShadowBuffer(v - 11000.0),
            _ if (20000.0..30000.0).contains(&v) => LodKind::Edit(v - 20000.0),
            _ => SPECIAL
                .iter()
                .find(|(value, _, _)| *value == v)
                .map_or(LodKind::Unknown(v), |(_, kind, _)| *kind),
        }
    }

    /// `true` for a visual Resolution LOD.
    pub fn is_visual(self) -> bool {
        matches!(self.kind(), LodKind::Resolution(_))
    }
}

impl fmt::Display for LodResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            LodKind::Resolution(r) => write!(f, "{r:.3}"),
            LodKind::ViewGunner => f.write_str("View Gunner"),
            LodKind::ViewPilot => f.write_str("View Pilot"),
            LodKind::ViewCargo => f.write_str("View Cargo"),
            LodKind::ShadowVolume(n) => write!(f, "Shadow Volume {n}"),
            LodKind::ShadowBuffer(n) => write!(f, "Shadow Buffer {n}"),
            LodKind::Edit(n) => write!(f, "Edit {n}"),
            LodKind::Unknown(v) => write!(f, "Unknown {v:e}"),
            kind => {
                let name = SPECIAL
                    .iter()
                    .find(|(_, k, _)| *k == kind)
                    .map_or("?", |(_, _, name)| name);
                f.write_str(name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_values_seen_in_shipped_models() {
        let cases = [
            (1.0, LodKind::Resolution(1.0)),
            (1100.0, LodKind::ViewPilot),
            (10010.0, LodKind::ShadowVolume(10.0)),
            (11000.0, LodKind::ShadowBuffer(0.0)),
            (9_999_999_827_968.0, LodKind::Geometry),
            (39_999_999_311_872.0, LodKind::GeometryPhysx),
            (999_999_986_991_104.0, LodKind::Memory),
            (6_000_000_056_164_352.0, LodKind::ViewGeometry),
            (6_999_999_976_046_592.0, LodKind::FireGeometry),
            (5e15, LodKind::HitPoints),
        ];
        for (value, kind) in cases {
            assert_eq!(LodResolution(value).kind(), kind, "{value:e}");
        }
    }

    #[test]
    fn displays_names() {
        assert_eq!(LodResolution(1e15).to_string(), "Memory");
        assert_eq!(LodResolution(10010.0).to_string(), "Shadow Volume 10");
        assert_eq!(LodResolution(2.0).to_string(), "2.000");
    }
}
