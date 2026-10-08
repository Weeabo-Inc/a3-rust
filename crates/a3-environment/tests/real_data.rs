//! The environment of the shipped worlds. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_environment::WorldEnvironment;
use a3_gamedata::{GameData, LoadOptions};

#[test]
fn shipped_worlds_light_up_by_day_and_go_dark_at_night() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(Path::new(&root))).unwrap();
    let worlds = data.config.root().get("CfgWorlds");
    for name in ["Altis", "Stratis", "Tanoa", "Malden"] {
        let world = worlds.get(name);
        assert!(world.is_class(), "{name}");
        let env = WorldEnvironment::from_config(&world);
        eprintln!(
            "{name}: {} lighting entries, {} overcast levels, observer {:?}, start {:?}",
            env.lighting.len(),
            env.overcast.levels.len(),
            env.observer,
            env.start.date_time
        );
        assert!(env.lighting.len() > 20, "{name}");
        assert!(env.overcast.levels.len() >= 4, "{name}");
        let mut state = env.start;
        state.set_date(2035, 6, 24, 0, 0);
        let mut noon_luma = 0.0;
        let mut midnight_luma = f32::MAX;
        for hour in 0..24 {
            let f = env.evaluate(&state, 50.0);
            let luma = f.light.diffuse.dot(glam::Vec3::new(0.299, 0.587, 0.114));
            eprintln!(
                "  {hour:02}:00 sun {:6.1} deg  diffuse {:10.1}  ambient {:9.1}  sky {:9.1}  aperture {:6.1}  stars {:4.1}",
                f.sun_elevation,
                luma,
                f.light.ambient.y,
                f.light.sky.y,
                f.light.aperture_standard,
                f.stars
            );
            if hour == 12 {
                noon_luma = luma;
            }
            if hour == 0 {
                midnight_luma = luma;
            }
            state.skip_time(1.0);
        }
        assert!(noon_luma > 10_000.0, "{name} noon {noon_luma}");
        assert!(midnight_luma < 100.0, "{name} midnight {midnight_luma}");
    }
}
