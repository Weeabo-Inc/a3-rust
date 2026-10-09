//! Vehicle handling: what a `CfgVehicles` class says about how a vehicle drives, and the model
//! that turns driver input into motion.
//!
//! The shipped configs are written for the PhysX 3 vehicle SDK (`carx`/`tankx`, see
//! `docs/re/sim-vehicles.md`): an engine with a torque curve, a clutch, a gearbox with automatic
//! shifting, a differential, raycast suspension and a tire model. This crate reads those entries
//! and implements that model, so our numbers keep the meaning the config's authors gave them.
//!
//! The crate is pure: no world, no collision world, no clock. `a3-world`'s ground family calls
//! into it every fixed step with the vehicle's state and the surface under each wheel.

pub mod engine;
pub mod gearbox;
pub mod ground;
pub mod handling;
pub mod value;
pub mod vehicle;
pub mod wheel;
pub mod wheeled;
