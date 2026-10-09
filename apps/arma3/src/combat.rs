//! Weapons in play: the player's rifle, the shots it fires and what they hit.
//!
//! The client's Man is still driven by [`crate::player::Player`]; this module keeps an
//! `a3-world` World beside it that holds the player as a unit with a Loadout, the terrain and
//! its Static objects in a collision world, and the shots. Each frame the player's unit is put
//! where the Man is, aimed where the camera looks, its trigger follows the fire action
//! (`defaultAction`), and the World steps: the rate of fire, the magazine reloads, the
//! ballistics and the hits are the World's (`docs/re/sim-ballistics.md`, `docs/re/sim-weapons.md`).
//!
//! Tracer rounds draw as a short bright streak along their flight; every hit leaves a marker
//! for a few seconds and a line in the overlay.

use std::collections::VecDeque;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_physics::{CollisionWorld, Interest, ModelBank};
use a3_render::{Color, DrawList, MeshDraw, MeshId};
use a3_world::{
    Aim, ClassState, ClientId, Create, EntityId, ListKind, ObjectRef, TypeBank, World, WorldEvent,
};
use a3_wrp::Terrain;
use glam::{DAffine3, DQuat, DVec3};

/// The rifle a player whose class carries no weapon gets: an MX with tracer rounds.
pub const FALLBACK_WEAPON: &str = "arifle_MX_F";
pub const FALLBACK_MAGAZINE: &str = "30Rnd_65x39_caseless_mag_Tracer";
/// Magazines the fallback rifle comes with besides the loaded one.
pub const FALLBACK_SPARES: u32 = 5;

/// How far around the player Static objects are streamed into the collision world, metres: past
/// the 6.5 mm round's 6 s of flight at low angles.
const STREAM_RADIUS: f64 = 1_500.0;

/// The muzzle sits this far ahead of, below and right of the eye in first person (the weapon's
/// own memory points are not posed yet).
const MUZZLE_AHEAD: f64 = 0.6;
const MUZZLE_DOWN: f64 = 0.08;
const MUZZLE_RIGHT: f64 = 0.1;

/// How long a tracer streak is, seconds of flight behind the round.
const TRACER_TAIL: f64 = 0.012;

/// A tracer bar's width per metre from the eye (about two pixels at 1080p), and its least width.
const TRACER_WIDTH_PER_METRE: f64 = 0.0015;
const TRACER_MIN_WIDTH: f64 = 0.02;

/// How long a hit marker stays, seconds.
const HIT_MARKER_TIME: f64 = 4.0;

/// How many hit lines the overlay keeps.
const HIT_LOG: usize = 4;

/// What a hit left to draw.
#[derive(Debug, Clone)]
struct HitMarker {
    position: DVec3,
    normal: DVec3,
    direct: bool,
    age: f64,
}

/// The World the player's weapon fires in.
pub struct Combat {
    world: World,
    unit: EntityId,
    markers: Vec<HitMarker>,
    log: VecDeque<String>,
    shots_fired: u64,
}

impl Combat {
    /// A World on `terrain` with the game's config, its Static objects streamed around `at`, and
    /// the player's unit there with the rifle and its magazines.
    pub fn new(
        terrain: Arc<Terrain>,
        vfs: a3_vfs::Vfs,
        config: Arc<ConfigTree>,
        unit_class: &str,
        at: DVec3,
    ) -> anyhow::Result<Combat> {
        let mut world = World::new(ClientId::SERVER);
        world.set_config(config.clone());
        world.load_terrain(terrain.clone())?;
        let bank = ModelBank::new(Arc::new(vfs), Some(&config));
        let mut collision = CollisionWorld::new(bank);
        collision.set_terrain(Some(terrain));
        world.set_collision_world(collision);
        world.stream_collision(&[Interest {
            center: at,
            radius: STREAM_RADIUS,
        }]);

        let mut types = TypeBank::new(config);
        // A hit building becomes an Entity of its own config class (armor, hit points, ruin).
        world.set_model_type_resolver(Some(types.resolver()));
        let unit = world.create(Create::new(types.get(unit_class)?, at))?;
        // The client moves the Man itself; the World only carries him as the shooter.
        if let Some(e) = world.entity_mut(unit) {
            e.set_simulation_enabled(false);
        }
        // Creation armed the unit with its class's `weapons[]` and `magazines[]`.
        if world.weapons_of(unit).is_empty() {
            for _ in 0..=FALLBACK_SPARES {
                world.add_magazine(unit, FALLBACK_MAGAZINE, None)?;
            }
            world.add_weapon(unit, FALLBACK_WEAPON)?;
        }
        world.drain_events();
        log::info!(
            "the player carries {:?}: {} with {} rounds of {}, magazines {:?}",
            world.weapons_of(unit),
            world.current_weapon(unit),
            world.ammo_in(unit, &world.current_muzzle(unit)),
            world.current_magazine(unit),
            world.magazines_of(unit)
        );
        Ok(Combat {
            world,
            unit,
            markers: Vec::new(),
            log: VecDeque::new(),
            shots_fired: 0,
        })
    }

    /// One frame: the unit follows the Man (`feet`, facing `yaw`), aims from `eye` along `look`
    /// (first person puts the muzzle beside the eye), the trigger follows `fire`, and the World
    /// steps by `dt`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        feet: DVec3,
        yaw: f64,
        eye: DVec3,
        look: DVec3,
        first_person: bool,
        fire: bool,
        reload: bool,
        next_mode: bool,
        dt: f64,
    ) {
        let unit = self.unit;
        if let Some(e) = self.world.entity_mut(unit) {
            e.set_position(feet);
            e.set_heading(yaw.to_degrees());
        }
        let look = look.normalize_or(DVec3::Z);
        let right = look.cross(DVec3::Y).normalize_or(DVec3::X);
        let from = if first_person {
            eye + look * MUZZLE_AHEAD - DVec3::Y * MUZZLE_DOWN - right * MUZZLE_RIGHT
        } else {
            eye + look * MUZZLE_AHEAD
        };
        self.world.set_aim(
            unit,
            Some(Aim {
                from,
                direction: look,
            }),
        );
        self.world.set_trigger(unit, fire);
        if reload {
            self.world.reload(unit);
        }
        if next_mode {
            self.cycle_mode();
        }
        self.world.stream_collision(&[Interest {
            center: feet,
            radius: STREAM_RADIUS,
        }]);
        self.world.simulate(dt);
        for marker in &mut self.markers {
            marker.age += dt;
        }
        self.markers.retain(|m| m.age < HIT_MARKER_TIME);
        for event in self.world.drain_events() {
            self.note(event);
        }
    }

    /// Selects the current muzzle's next fire mode (`Single` → `FullAuto` → ...).
    fn cycle_mode(&mut self) {
        let Some(loadout) = self.world.unit_loadout(self.unit) else {
            return;
        };
        let Some((w, m)) = loadout.current else {
            return;
        };
        let modes = &loadout.weapons[w].kind.muzzles[m].modes;
        if modes.is_empty() {
            return;
        }
        let next = (loadout.weapons[w].muzzles[m].mode + 1) % modes.len();
        let name = modes[next].name.clone();
        self.world.select_mode(self.unit, &name);
    }

    fn note(&mut self, event: WorldEvent) {
        match event {
            WorldEvent::Fired { .. } => self.shots_fired += 1,
            WorldEvent::Hit {
                target,
                position,
                normal,
                speed_in,
                value,
                direct,
                damage,
                component,
                ..
            } => {
                let name = self.describe(target);
                let line = format!(
                    "HIT {name}{} AT {:.0} M/S  VALUE {value:.2}  DAMAGE {:.3}{}",
                    component.map(|c| format!(" {c}")).unwrap_or_default(),
                    speed_in,
                    damage,
                    if direct { "" } else { "  (BLAST)" }
                );
                log::info!("{}", line.to_lowercase());
                self.log.push_front(line);
                self.log.truncate(HIT_LOG);
                self.markers.push(HitMarker {
                    position,
                    normal,
                    direct,
                    age: 0.0,
                });
            }
            WorldEvent::Exploded {
                position, radius, ..
            } => {
                log::info!("explosion at {position:.1}, radius {radius} m");
            }
            _ => {}
        }
    }

    /// A hit target as the overlay names it: its type, or the Static object's model.
    fn describe(&self, target: ObjectRef) -> String {
        match target {
            ObjectRef::Entity(id) => self
                .world
                .entity(id)
                .map(|e| e.type_name().to_uppercase())
                .unwrap_or_else(|| "OBJECT".to_owned()),
            ObjectRef::Static(key) => self
                .world
                .static_model(key)
                .map(|m| {
                    let m = m.as_str();
                    m.rsplit(['\\', '/']).next().unwrap_or(m).to_uppercase()
                })
                .unwrap_or_else(|| "STATIC".to_owned()),
        }
    }

    /// Tracer streaks and hit markers. A tracer burns from the ammo's `tracerStartTime` to its
    /// `tracerEndTime` of flight and is drawn as a bar of `cube` (a unit cube mesh) at least a
    /// couple of pixels wide at any distance from `eye`, with a line along it.
    pub fn draw(&self, draws: &mut DrawList, cube: Option<MeshId>, eye: DVec3) {
        for &id in self.world.list(ListKind::Projectiles) {
            let Some(e) = self.world.entity(id) else {
                continue;
            };
            let ClassState::Projectile(state) = e.class_state() else {
                continue;
            };
            let Some(ammo) = state.ammo.as_ref().filter(|_| state.tracer) else {
                continue;
            };
            let age = e.simulated_time();
            if age < ammo.tracer_start_time || age > ammo.tracer_end_time {
                continue;
            }
            let head = e.render_position();
            let tail = head - e.velocity() * TRACER_TAIL;
            draws.lines.line(tail, head, TRACER_COLOR);
            if let Some(cube) = cube {
                let along = head - tail;
                let width = (head.distance(eye) * TRACER_WIDTH_PER_METRE).max(TRACER_MIN_WIDTH);
                let rotation = DQuat::from_rotation_arc(DVec3::Z, along.normalize_or(DVec3::Z));
                draws.mesh(MeshDraw {
                    mesh: cube,
                    texture: None,
                    transform: DAffine3::from_scale_rotation_translation(
                        DVec3::new(width, width, along.length()),
                        rotation,
                        (head + tail) * 0.5,
                    ),
                    color: TRACER_COLOR,
                    transparent: false,
                });
            }
        }
        let lines = &mut draws.lines;
        for m in &self.markers {
            let fade = (1.0 - m.age / HIT_MARKER_TIME) as f32;
            let color: Color = if m.direct {
                [1.0, 0.25, 0.1, fade]
            } else {
                [1.0, 0.8, 0.1, fade]
            };
            lines.wire_sphere(m.position, 0.06, 12, color);
            lines.line(m.position, m.position + m.normal * 0.3, color);
        }
    }

    /// The rounds in the selected muzzle's magazine (`None` when none is loaded) and the further
    /// magazines carried for it, for the HUD's ammo counter.
    pub fn rounds(&self) -> (Option<u32>, u32) {
        let w = &self.world;
        let Some(loadout) = w.unit_loadout(self.unit) else {
            return (None, 0);
        };
        let Some((wi, mi)) = loadout.current else {
            return (None, 0);
        };
        let muzzle = &loadout.weapons[wi].kind.muzzles[mi];
        let loaded = loadout.weapons[wi].muzzles[mi]
            .magazine
            .as_ref()
            .map(|m| m.ammo);
        let spare = loadout
            .magazines
            .iter()
            .filter(|m| m.ammo > 0)
            .filter(|m| {
                muzzle
                    .magazines
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(m.name()))
            })
            .count() as u32;
        (loaded, spare)
    }

    /// Overlay lines: the weapon, its mode and rounds, and the latest hits.
    pub fn overlay(&self) -> Vec<String> {
        let w = &self.world;
        let unit = self.unit;
        let muzzle = w.current_muzzle(unit);
        let reloading = w
            .unit_loadout(unit)
            .and_then(|l| {
                let (wi, mi) = l.current?;
                l.weapons[wi].muzzles[mi]
                    .magazine
                    .as_ref()
                    .map(|m| m.reload_left)
            })
            .filter(|left| *left > 0.0)
            .map(|left| format!("  RELOADING {left:.1} S"))
            .unwrap_or_default();
        let mut out = vec![format!(
            "WEAPON {}  {}  {} ROUNDS  {} MAGAZINES{}  FIRED {}  LMB FIRE  R RELOAD  F MODE",
            w.current_weapon(unit).to_uppercase(),
            w.current_weapon_mode(unit).to_uppercase(),
            w.ammo_in(unit, &muzzle),
            w.magazines_of(unit).len(),
            reloading,
            self.shots_fired
        )];
        out.extend(self.log.iter().cloned());
        out
    }
}

/// Tracer red-orange, bright: lines are not lit, so this is what the eye sees.
const TRACER_COLOR: Color = [1.0, 0.45, 0.15, 1.0];

#[cfg(test)]
mod tests {
    use super::*;
    use a3_gamedata::{GameData, LoadOptions};
    use a3_landscape::WorldConfig;
    use a3_world::Near;

    /// On Stratis, the player stands 25 m from a house, aims at its middle and holds the
    /// trigger: tracers fly and the house is hit.
    #[test]
    fn the_player_hits_a_house_on_stratis() {
        let Some(root) = std::env::var_os("A3_ROOT") else {
            eprintln!("skipping: A3_ROOT not set");
            return;
        };
        let data = GameData::load(&LoadOptions::new(root)).unwrap();
        let config = WorldConfig::load(&data.config, "stratis").unwrap();
        let bytes = data.vfs.open(config.wrp.as_str()).unwrap();
        let terrain = Arc::new(Terrain::parse(&bytes).unwrap());

        // A house near the centre of the island: the nearest Static object with "house" in its
        // model path.
        let mut probe = World::new(ClientId::SERVER);
        probe.load_terrain(terrain.clone()).unwrap();
        let centre = DVec3::new(
            f64::from(config.center_position.x),
            0.0,
            f64::from(config.center_position.y),
        );
        let (house, house_at) = probe
            .objects_near(centre, 3_000.0, Near::Statics)
            .into_iter()
            .find_map(|(o, _)| {
                let ObjectRef::Static(key) = o else {
                    return None;
                };
                let model = probe.static_model(key)?.as_str().to_ascii_lowercase();
                (model.contains("house") && !model.contains("ruin"))
                    .then(|| (o, probe.object_position(o).unwrap()))
            })
            .expect("a house on Stratis");

        let ground = probe.surface_height(house_at.x - 25.0, house_at.z);
        let feet = DVec3::new(house_at.x - 25.0, ground, house_at.z);
        let mut combat = Combat::new(
            terrain,
            data.vfs.clone(),
            data.config.clone(),
            crate::player::PLAYER_CLASS,
            feet,
        )
        .unwrap();
        let eye = feet + DVec3::new(0.0, 1.6, 0.0);
        let target = DVec3::new(house_at.x, ground + 2.0, house_at.z);
        let mut hit = false;
        let mut tracers = 0;
        for frame in 0..60 {
            combat.update(
                feet,
                90f64.to_radians(),
                eye,
                target - eye,
                true,
                frame < 10,
                false,
                false,
                1.0 / 60.0,
            );
            tracers = tracers.max(combat.world.list(ListKind::Projectiles).len());
            hit |= !combat.markers.is_empty();
        }
        eprintln!(
            "house {house:?} at {house_at:.1}; overlay: {:#?}",
            combat.overlay()
        );
        assert!(combat.shots_fired > 0, "the rifle fired");
        assert!(tracers > 0, "tracer rounds flew");
        assert!(hit, "something was hit");
    }
}
