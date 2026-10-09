//! Resolving config sound paths in a VFS and caching decoded clips.

use a3_audio::{SoundLoader, Source, resolve_sound_path};
use a3_audio_formats::Sound;
use a3_pbo::{Pbo, PboWriter};
use a3_vfs::Vfs;

const OGG: &[u8] = include_bytes!("../../a3-audio-formats/tests/fixtures/sine440_stereo.ogg");

fn vfs() -> Vfs {
    let wav = Sound {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0, 100, -100, 0],
    }
    .to_wav();
    let bytes = PboWriter::new()
        .property("prefix", r"a3\sounds_f")
        .file(r"ambient\sea.ogg", OGG.to_vec())
        .file(r"ui\beep.wav", wav)
        .file(r"ui\explicit.wss.bak", b"junk".to_vec())
        .to_bytes();
    let vfs = Vfs::new();
    vfs.mount_pbo(Pbo::from_bytes(bytes).unwrap(), None);
    vfs
}

#[test]
fn config_paths_resolve_with_or_without_extension_and_leading_backslash() {
    let vfs = vfs();
    let found = |p: &str| resolve_sound_path(&vfs, p).map(|p| p.as_str().to_string());
    assert_eq!(
        found(r"\A3\Sounds_F\ambient\sea").as_deref(),
        Some(r"a3\sounds_f\ambient\sea.ogg")
    );
    assert_eq!(
        found(r"a3\sounds_f\ui\beep.wav").as_deref(),
        Some(r"a3\sounds_f\ui\beep.wav")
    );
    assert_eq!(found(r"a3\sounds_f\ui\missing"), None);
    assert_eq!(found(""), None);
}

#[test]
fn clips_are_decoded_once_and_shared() {
    let loader = SoundLoader::new(vfs());
    let a = loader.clip(r"a3\sounds_f\ambient\sea").unwrap();
    let b = loader.clip(r"\a3\sounds_f\ambient\SEA.ogg").unwrap();
    assert_eq!(a.frames(), 5513);
    assert_eq!(a.frames(), b.frames());
    assert!(matches!(
        loader.source(r"a3\sounds_f\ui\beep", false),
        Some(Source::Clip(_))
    ));
    assert!(loader.source(r"a3\sounds_f\ui\missing", false).is_none());
}
