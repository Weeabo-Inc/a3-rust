//! Loading a mission into a [`World`](a3_world::World): `mission.sqm`, `description.ext`, the
//! units, groups, markers and triggers, and the mission scripts.
//!
//! The pipeline, in the order a mission runs them:
//!
//! 1. [`load_mission`] reads `mission.sqm` (text or rapified) through the VFS and, when the
//!    mission has one, `description.ext` through the config preprocessor and parser into a
//!    [`Mission`] value. Nothing is created in the World yet.
//! 2. [`spawn_mission`] creates the World's Entities at the SQM positions (the third component of
//!    `position[]` is height above sea level, so `CAN_COLLIDE` placements keep it and surface
//!    placements let the terrain set it), puts them into their groups and applies their headings.
//! 3. [`run_scripts`] installs the mission's variables (`player`, each named unit), then runs the
//!    units' `init` fields — unscheduled, `this` = the unit — and finally `init.sqf`, which runs
//!    scheduled, so it may `sleep` and `waitUntil`.
//!
//! What is parsed but not yet applied: `skill`, `rank`, `special`, `lock`, waypoints and triggers
//! (they are values on [`Mission`]; the World has no AI or trigger system yet), and markers (no
//! marker system either — [`Spawned::markers`] carries them). Script commands a mission uses that
//! the VM does not implement are collected in [`RunReport::missing_commands`] instead of stopping
//! the run.
//!
//! ```
//! # use a3_mission::{Mission, parse_sqm};
//! let sqm = b"version=12;\nclass Mission {\n  class Groups {\n    items=1;\n    class Item0 {\n      side=\"WEST\";\n      class Vehicles {\n        items=1;\n        class Item0 { id=1; vehicle=\"B_Soldier_F\"; position[]={100,0,200}; azimut=45; leader=1; text=\"boss\"; };\n      };\n    };\n  };\n};";
//! let mission = Mission::from_config(&parse_sqm(sqm).unwrap()).unwrap();
//! assert_eq!(mission.units().count(), 1);
//! assert_eq!(mission.unit(1).unwrap().class, "B_Soldier_F");
//! ```

mod load;
mod mission;
mod run;
mod spawn;
mod sqm;

pub use load::{LoadError, install_mission_config, load_mission};
pub use mission::{
    AttributeValue, EntityAttribute, Intel, Marker, Mission, MissionError, MissionGroup,
    MissionVariable, Sensor, Unit, Waypoint, side_from_sqm,
};
pub use run::{
    MAX_INIT_FRAMES, MissionHost, MissionState, MissionVmHost, RunReport, ScriptRun, StartOptions,
    mission_registry, register_mission_commands, run_scripts, start_mission, step,
};
pub use spawn::{Spawned, SpawnedGroup, Unspawned, spawn_mission};
pub use sqm::{SqmError, parse_sqm};
