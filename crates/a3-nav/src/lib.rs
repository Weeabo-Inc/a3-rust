//! AI navigation: pathfinding for the World's AI units (ADR 0009, RE notes in
//! `docs/re/navigation.md`).
//!
//! The engine plans a path as an A* over its **operational map** — a cell field baked from the
//! terrain (`OperMap`/`OperField`, `AIPathPlanner::ProcessSearching`). This crate is the same
//! shape:
//!
//! - [`NavGrid`] is the cell field: one **cost** ([`cost`]) per land cell, baked once from the
//!   terrain's geography flags, its heightmap slope and the road network. `0` means impassable.
//! - [`Planner`] is the A* over it, with a reusable scratch so many agents can plan without
//!   allocating, on 8 neighbours with no corner cutting.
//! - [`Navigator`] is the query an AI uses: `find_path(from, to, radius)`, which snaps both ends
//!   into the grid, plans, and pulls the cell path straight along the terrain
//!   ([`smooth::string_pull`]).
//! - [`PathMesh`] is a building's own path graph, built from a model's Paths LOD (its Roadway
//!   LOD as the fallback), for the part of a path that is indoors.
//! - [`Navigator::block_from_colliders`] patches the grid with obstacles physics has loaded.
//!
//! Everything is in `f32` World space (`glam::Vec3`), converted at the `a3-world` boundary
//! (ADR 0003). The man's costs are the ones baked here; vehicles need their own set before
//! `calculatePath` can be answered for them.

pub mod astar;
pub mod building;
pub mod grid;
pub mod nav;
pub mod smooth;

pub use astar::Planner;
pub use building::PathMesh;
pub use grid::{NavGrid, cost};
pub use nav::Navigator;

/// The steepest slope (rise over run) a man can walk, `CfgSlopeLimits >> maxRun` (RE notes in
/// `docs/re/sim-man-movement.md` §4).
pub const MAX_SLOPE: f32 = 0.6;

/// How far above the ground an obstacle must reach to block a cell, in metres: a wall, not a
/// kerb (the man's own height is about 1.8 m).
pub const BLOCKER_HEIGHT: f32 = 1.8;

/// How high a man can step up without slowing down, in metres.
pub const STEP_UP: f32 = 0.4;
