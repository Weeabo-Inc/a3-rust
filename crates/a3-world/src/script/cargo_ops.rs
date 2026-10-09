//! Crew and vehicle-cargo commands: `moveInDriver`, `moveInGunner`, `moveInCargo`,
//! `assignAsCargo`, `allowCrewInImmobile` and `setVehicleAmmo`.
//!
//! The seats a command fills are [`ObjectState`]'s; the commands that read them back
//! (`vehicle`, `driver`, `crew`, `assignedVehicleRole`) are a later batch, so nothing in SQF
//! observes a seat yet — the state itself is what the engine keeps. From the decompiled handlers
//! (RVAs from `docs/re/sqf-commands.tsv`) and the offline wiki for the argument layout.

use a3_sqf::{Registry, Value};

use super::{ARR, BOOL, NOTHING, NUM, OBJ, WorldHost, object_arg};
use crate::{EntityClass, EntityId, ObjectRef, World};

/// The seat a unit was put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    Driver,
    Gunner,
    Cargo(i32),
}

fn items(v: &Value) -> Vec<Value> {
    v.as_array().map(|a| a.borrow().clone()).unwrap_or_default()
}

/// The Entity a value names.
fn entity_id(world: &World, value: &Value) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => world.entity(id).map(|_| id),
        ObjectRef::Static(_) => None,
    }
}

/// A Man (the seat holder) and a Transport (the vehicle), the pair every `moveIn*` needs.
fn unit_and_vehicle(world: &World, a: &Value, b: &Value) -> Option<(EntityId, EntityId)> {
    let unit = entity_id(world, a).filter(|&id| {
        world
            .entity(id)
            .is_some_and(|e| e.class().is_kind_of(EntityClass::Man))
    })?;
    let vehicle = entity_id(world, b).filter(|&id| {
        world
            .entity(id)
            .is_some_and(|e| e.class().is_kind_of(EntityClass::Transport))
    })?;
    Some((unit, vehicle))
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // 0x537df0: `unit moveInDriver vehicle`.
    r.binary("moveInDriver", OBJ, OBJ, NOTHING, |ctx, a, b| {
        if let Some((unit, vehicle)) = unit_and_vehicle(ctx.host.world(), &a, &b) {
            ctx.host.world_mut().seat_unit(unit, vehicle, Seat::Driver);
        }
        Ok(Value::Nothing)
    });
    // 0x538060: `unit moveInGunner vehicle`.
    r.binary("moveInGunner", OBJ, OBJ, NOTHING, |ctx, a, b| {
        if let Some((unit, vehicle)) = unit_and_vehicle(ctx.host.world(), &a, &b) {
            ctx.host.world_mut().seat_unit(unit, vehicle, Seat::Gunner);
        }
        Ok(Value::Nothing)
    });
    // 0x5376a0: `unit moveInCargo vehicle`, the first free seat.
    r.binary("moveInCargo", OBJ, OBJ, NOTHING, |ctx, a, b| {
        if let Some((unit, vehicle)) = unit_and_vehicle(ctx.host.world(), &a, &b) {
            let seat = free_cargo_seat(ctx.host.world(), vehicle);
            ctx.host.world_mut().seat_unit(unit, vehicle, seat);
        }
        Ok(Value::Nothing)
    });
    // 0x537680: `unit moveInCargo [vehicle, index]`; a negative index takes the first free seat.
    r.binary("moveInCargo", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let Some(vehicle_value) = args.first().cloned() else {
            return Ok(Value::Nothing);
        };
        let Some((unit, vehicle)) = unit_and_vehicle(ctx.host.world(), &a, &vehicle_value) else {
            return Ok(Value::Nothing);
        };
        let index = args
            .get(1)
            .and_then(Value::as_number)
            .map(|n| n.round() as i32)
            .unwrap_or(-1);
        let seat = if index < 0 {
            free_cargo_seat(ctx.host.world(), vehicle)
        } else {
            Seat::Cargo(index)
        };
        ctx.host.world_mut().seat_unit(unit, vehicle, seat);
        Ok(Value::Nothing)
    });
    // 0x5250e0: `unit assignAsCargo vehicle` — the seat is taken when he boards.
    r.binary("assignAsCargo", OBJ, OBJ, NOTHING, |ctx, a, b| {
        if let Some((unit, vehicle)) = unit_and_vehicle(ctx.host.world(), &a, &b) {
            let seat = free_cargo_seat(ctx.host.world(), vehicle);
            let state = ctx.host.world_mut().object_state_mut(vehicle);
            state.assigned_cargo.retain(|(u, _)| *u != unit);
            if let Seat::Cargo(index) = seat {
                state.assigned_cargo.push((unit, index));
            }
        }
        Ok(Value::Nothing)
    });
    // 0x47e3b0: `vehicle allowCrewInImmobile allow`, and `[allow, "CARGO"]` for the cargo seats.
    r.binary(
        "allowCrewInImmobile",
        OBJ,
        BOOL.union(ARR),
        NOTHING,
        |ctx, a, b| {
            let Some(vehicle) = entity_id(ctx.host.world(), &a).filter(|&id| {
                ctx.host
                    .world()
                    .entity(id)
                    .is_some_and(|e| e.class().is_kind_of(EntityClass::Transport))
            }) else {
                return Ok(Value::Nothing);
            };
            let (allow, cargo) = match b.as_bool() {
                Some(allow) => (allow, false),
                None => {
                    let args = items(&b);
                    let allow = args.first().and_then(Value::as_bool).unwrap_or(false);
                    let role = args.get(1).and_then(Value::as_str).unwrap_or_default();
                    (allow, role.eq_ignore_ascii_case("CARGO"))
                }
            };
            let state = ctx.host.world_mut().object_state_mut(vehicle);
            if cargo {
                state.crew_in_immobile_cargo = allow;
            } else {
                state.crew_in_immobile = allow;
            }
            Ok(Value::Nothing)
        },
    );
    // 0x56f960: `vehicleName setVehicleAmmo value`, the fraction of its full ammo; the engine
    // clamps to 0..1, where 1 fills it.
    r.binary("setVehicleAmmo", OBJ, NUM, NOTHING, |ctx, a, b| {
        let Some(vehicle) = entity_id(ctx.host.world(), &a).filter(|&id| {
            ctx.host
                .world()
                .entity(id)
                .is_some_and(|e| e.class().is_kind_of(EntityClass::Transport))
        }) else {
            return Ok(Value::Nothing);
        };
        let value = b.as_number().unwrap_or(0.0);
        ctx.host.world_mut().object_state_mut(vehicle).vehicle_ammo = Some(value.clamp(0.0, 1.0));
        Ok(Value::Nothing)
    });
}

/// The first cargo seat no one is in, `Cargo(0)` and up.
fn free_cargo_seat(world: &World, vehicle: EntityId) -> Seat {
    let taken: Vec<i32> = world
        .object_state(vehicle)
        .map(|s| s.cargo_seats.iter().map(|(i, _)| *i).collect())
        .unwrap_or_default();
    let mut index = 0;
    while taken.contains(&index) {
        index += 1;
    }
    Seat::Cargo(index)
}

impl World {
    /// Puts `unit` in a seat of `vehicle`: he leaves the seat he was in, here or in another
    /// vehicle (the engine moves him, it does not clone him).
    pub fn seat_unit(&mut self, unit: EntityId, vehicle: EntityId, seat: Seat) {
        self.clear_seat(unit);
        let state = self.object_state_mut(vehicle);
        match seat {
            Seat::Driver => state.driver = Some(unit),
            Seat::Gunner => state.gunner = Some(unit),
            Seat::Cargo(index) => {
                state.cargo_seats.retain(|(i, u)| *i != index && *u != unit);
                state.cargo_seats.push((index, unit));
                state.assigned_cargo.retain(|(u, _)| *u != unit);
            }
        }
    }

    /// Takes `unit` out of every seat he holds (in any vehicle).
    pub fn clear_seat(&mut self, unit: EntityId) {
        let vehicles: Vec<EntityId> = self
            .entities()
            .filter(|e| e.class().is_kind_of(EntityClass::Transport))
            .map(|e| e.id())
            .collect();
        for vehicle in vehicles {
            let state = self.object_state_mut(vehicle);
            if state.driver == Some(unit) {
                state.driver = None;
            }
            if state.gunner == Some(unit) {
                state.gunner = None;
            }
            state.cargo_seats.retain(|(_, u)| *u != unit);
        }
    }
}
