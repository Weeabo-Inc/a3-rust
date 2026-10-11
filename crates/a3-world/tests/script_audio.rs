//! The sound commands run as scripts: the scripted volumes reach the host's [`AudioHost`], and a
//! host without one answers the engine's full volume.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Handle, Host, Registry, ScriptError, Value, Vm};
use a3_world::script::{AudioHost, ScriptWorld, WorldHost, register_world_commands};
use a3_world::{ClientId, TypeBank, World};

/// An [`AudioHost`] that records what the commands asked of it.
#[derive(Debug, Default)]
struct RecordingAudio {
    sound: f32,
    music: f32,
    radio: f32,
    fades: Vec<(&'static str, f32, f32)>,
    played: Vec<(&'static str, String)>,
}

impl AudioHost for RecordingAudio {
    fn fade_sound(&mut self, seconds: f32, volume: f32) {
        self.sound = volume;
        self.fades.push(("sound", seconds, volume));
    }

    fn sound_volume(&self) -> f32 {
        self.sound
    }

    fn fade_music(&mut self, seconds: f32, volume: f32) {
        self.music = volume;
        self.fades.push(("music", seconds, volume));
    }

    fn music_volume(&self) -> f32 {
        self.music
    }

    fn fade_radio(&mut self, seconds: f32, volume: f32) {
        self.radio = volume;
        self.fades.push(("radio", seconds, volume));
    }

    fn radio_volume(&self) -> f32 {
        self.radio
    }

    fn play_sound(&mut self, name: &str) -> bool {
        self.played.push(("sound", name.to_owned()));
        true
    }

    fn play_music(&mut self, name: &str) -> bool {
        self.played.push(("music", name.to_owned()));
        true
    }
}

/// A [`ScriptWorld`] with an audio engine, or without one when `audio` is `None`.
#[derive(Debug)]
struct AudioWorld {
    inner: ScriptWorld,
    audio: Option<RecordingAudio>,
}

impl Host for AudioWorld {
    fn time(&self) -> f32 {
        self.inner.time()
    }

    fn is_null(&self, handle: Handle) -> bool {
        self.inner.is_null(handle)
    }

    fn format_handle(&self, handle: Handle) -> String {
        self.inner.format_handle(handle)
    }

    fn report_error(&mut self, error: &ScriptError) {
        panic!("unexpected script error: {}", error.report);
    }
}

impl WorldHost for AudioWorld {
    fn world(&self) -> &World {
        self.inner.world()
    }

    fn world_mut(&mut self) -> &mut World {
        self.inner.world_mut()
    }

    fn types(&mut self) -> &mut TypeBank {
        self.inner.types()
    }

    fn audio(&mut self) -> Option<&mut dyn AudioHost> {
        self.audio.as_mut().map(|audio| audio as &mut dyn AudioHost)
    }
}

fn vm(audio: Option<RecordingAudio>) -> Vm<AudioWorld> {
    let config = parse_text("class CfgVehicles {};").unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(
        AudioWorld {
            inner: ScriptWorld::new(World::new(ClientId::SERVER), types),
            audio,
        },
        Rc::new(registry),
    )
}

fn eval(vm: &mut Vm<AudioWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn number(vm: &mut Vm<AudioWorld>, code: &str) -> f32 {
    eval(vm, code)
        .as_number()
        .unwrap_or_else(|| panic!("{code}: not a number"))
}

#[test]
fn the_fades_reach_the_audio_host_and_the_getters_read_it_back() {
    let mut vm = vm(Some(RecordingAudio::default()));
    eval(
        &mut vm,
        "2 fadeSound 0.25; 3 fadeMusic 0.5; 0 fadeRadio 1.5",
    );
    assert_eq!(number(&mut vm, "soundVolume"), 0.25);
    assert_eq!(number(&mut vm, "musicVolume"), 0.5);
    assert_eq!(number(&mut vm, "radioVolume"), 1.5);
    let audio = vm.host.audio.as_ref().unwrap();
    assert_eq!(
        audio.fades,
        [
            ("sound", 2.0, 0.25),
            ("music", 3.0, 0.5),
            ("radio", 0.0, 1.5)
        ]
    );
}

#[test]
fn play_sound_and_play_music_take_the_name_of_either_form() {
    let mut vm = vm(Some(RecordingAudio::default()));
    let speaker = eval(&mut vm, r#"playSound "Alarm""#);
    let Value::Handle(speaker) = speaker else {
        panic!("playSound answers an Object, not {speaker:?}");
    };
    assert!(vm.host.is_null(speaker));
    eval(
        &mut vm,
        r#"playSound ["Beep", true]; playMusic "LeadTrack01_F"; playMusic ["", 0]"#,
    );
    let audio = vm.host.audio.as_ref().unwrap();
    assert_eq!(
        audio.played,
        [
            ("sound", "Alarm".to_owned()),
            ("sound", "Beep".to_owned()),
            ("music", "LeadTrack01_F".to_owned()),
            ("music", String::new()),
        ]
    );
}

#[test]
fn without_an_audio_engine_the_volumes_are_full_and_music_follows_the_world() {
    let mut vm = vm(None);
    assert_eq!(number(&mut vm, "soundVolume"), 1.0);
    assert_eq!(number(&mut vm, "radioVolume"), 1.0);
    eval(&mut vm, "1 fadeSound 0; 1 fadeMusic 0.3");
    assert_eq!(number(&mut vm, "soundVolume"), 1.0);
    assert_eq!(number(&mut vm, "musicVolume"), 0.3);
}
