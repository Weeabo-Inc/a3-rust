//! The environment of a synthetic world, end to end.

use a3_config::{ConfigTree, parse_text};
use a3_environment::{Fog, FogLimits, OvercastTable, WorldEnvironment};

const WORLD: &str = r#"
class CfgWorlds {
    class Test {
        latitude = -35.152; longitude = 16.661;
        startDate = "24/6/2035"; startTime = "12:00";
        startWeather = 0.3; startFog = 0.1; startFogDecay = 0.02; startFogBase = 10;
        fogBeta0Min = 0; fogBeta0Max = 0.05;
        class Lighting { starEmissivity = 25; moonObjectColorFull[] = {460, 440, 400, 1}; };
        class Weather {
            class LightingNew {
                class Night { height = 0; overcast = 0.25; sunAngle = -24; sunOrMoon = 0;
                    diffuse[] = {0.1, 0.1, 0.1}; diffuseCloud[] = {0.1, 0.1, 0.1};
                    ambient[] = {0.2, 0.2, 0.2}; ambientCloud[] = {0.2, 0.2, 0.2}; };
                class Day { height = 0; overcast = 0.25; sunAngle = 45; sunOrMoon = 1;
                    diffuse[] = {{1, 1, 1}, 16}; diffuseCloud[] = {{1, 1, 1}, 12};
                    ambient[] = {{1, 1, 1}, 14}; ambientCloud[] = {{1, 1, 1}, 14}; };
            };
            class Overcast {
                class Weather1 { overcast = 0; through = 1; alpha = 0; lightingOvercast = 0;
                    skyR = "clear.paa"; };
                class Weather2 { overcast = 1; through = 0; alpha = 1; lightingOvercast = 1;
                    skyR = "overcast.paa"; };
            };
        };
    };
};
"#;

fn world() -> WorldEnvironment {
    let tree = ConfigTree::from_config(&parse_text(WORLD).unwrap());
    WorldEnvironment::from_config(&tree.root().get("CfgWorlds").get("Test"))
}

#[test]
fn reads_the_start_state_from_the_world_config() {
    let w = world();
    let s = w.start;
    assert_eq!(
        (s.date_time.year, s.date_time.month, s.date_time.day),
        (2035, 6, 24)
    );
    assert_eq!(s.overcast, 0.3);
    assert_eq!(
        s.fog,
        Fog {
            value: 0.1,
            decay: 0.02,
            base: 10.0
        }
    );
    assert_eq!(w.lighting.len(), 2);
    assert_eq!(w.overcast.levels.len(), 2);
}

#[test]
fn noon_is_lit_by_the_sun_and_midnight_by_the_moon() {
    let w = world();
    let mut state = w.start;
    state.overcast = 0.0;
    let noon = w.evaluate(&state, 10.0);
    assert!(noon.sun_direction.y > 0.9);
    assert_eq!(noon.light_direction, noon.sun_direction);
    // Above the 45 degree entry: the day light, 2^16 luminance.
    assert!((noon.light.diffuse.x - 65_536.0).abs() < 1.0);
    assert!(noon.stars == 0.0);

    state.skip_time(12.0);
    let midnight = w.evaluate(&state, 10.0);
    assert!(midnight.sun_direction.y < -0.3);
    assert_eq!(midnight.light_direction, midnight.moon_direction);
    assert!((midnight.light.diffuse.x - 0.1).abs() < 1e-4);
    assert!(midnight.stars > 20.0);
}

#[test]
fn clouds_over_the_sun_blend_towards_the_cloud_colours() {
    let w = world();
    let mut state = w.start;
    state.overcast = 1.0;
    let f = w.evaluate(&state, 10.0);
    assert_eq!(f.cloud_cover, 1.0);
    assert!((f.light.diffuse.x - 4096.0).abs() < 1.0);
    assert_eq!(f.weather.level.sky_reflection, "overcast.paa");
}

#[test]
fn shadows_never_come_from_below_the_engine_limit() {
    let w = world();
    let mut state = w.start;
    state.set_date(2035, 6, 24, 20, 0);
    let f = w.evaluate(&state, 10.0);
    // The engine sets the height to 0.4, then renormalises.
    let l = f.light_direction;
    assert!(l.y < 0.4);
    let expected = glam::Vec3::new(l.x, 0.4, l.z).normalize();
    assert!((f.shadow_direction - expected).length() < 1e-5);
}

#[test]
fn fog_follows_the_engine_curve_and_height_decay() {
    let limits = FogLimits {
        beta0_min: 0.0,
        beta0_max: 0.05,
    };
    assert_eq!(limits.beta0(0.0), 0.0);
    assert!((limits.beta0(1.0) - 0.05).abs() < 1e-7);
    // (e^2 - 1) / (e^4 - 1) of the range at half fog.
    assert!((limits.beta0(0.5) - 0.05 * 6.389_056 / 53.598_15).abs() < 1e-6);
    let fog = Fog {
        value: 1.0,
        decay: 0.01,
        base: 100.0,
    };
    assert!((limits.extinction(&fog, 100.0) - 0.05).abs() < 1e-6);
    assert!((limits.extinction(&fog, 200.0) - 0.05 / std::f32::consts::E).abs() < 1e-6);
    let f = world().evaluate(&world().start, 0.0);
    let expected = limits.beta0(0.1) * (0.02f32 * 10.0).exp();
    assert!((f.fog_sea_level - expected).abs() < 1e-7);
}

#[test]
fn overcast_levels_interpolate_and_clamp() {
    let t = OvercastTable::from_config(
        &ConfigTree::from_config(&parse_text(WORLD).unwrap())
            .root()
            .get("CfgWorlds")
            .get("Test")
            .get("Weather")
            .get("Overcast"),
    );
    let s = t.sample(0.25);
    assert!((s.level.alpha - 0.25).abs() < 1e-6);
    assert_eq!(s.level.sky_reflection, "clear.paa");
    assert_eq!(s.next_sky_reflection, "overcast.paa");
    assert!((s.blend - 0.25).abs() < 1e-6);
    assert_eq!(t.sample(2.0).level.alpha, 1.0);
}
