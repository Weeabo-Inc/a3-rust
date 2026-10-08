//! `a3-tools sound ...`: inspect and convert sound files (WSS, Ogg Vorbis, WAV).

use std::path::PathBuf;

use a3_audio_formats::{Format, SoundInfo};
use anyhow::Context;
use clap::Subcommand;

use crate::input::InputArgs;

#[derive(Subcommand)]
pub enum SoundCommand {
    /// Print the format, channels, rate and length of a sound file.
    Info(InputArgs),
    /// Decode a sound file to a 16-bit PCM WAV file.
    Towav {
        #[command(flatten)]
        input: InputArgs,
        /// Output WAV file.
        output: PathBuf,
    },
}

pub fn run(cmd: SoundCommand) -> anyhow::Result<()> {
    match cmd {
        SoundCommand::Info(input) => print!("{}", info(&a3_audio_formats::probe(&input.read()?)?)),
        SoundCommand::Towav { input, output } => {
            let sound = a3_audio_formats::decode(&input.read()?)?;
            std::fs::write(&output, sound.to_wav())
                .with_context(|| format!("writing {}", output.display()))?;
            eprintln!(
                "wrote {} ({} frames, {:.2?})",
                output.display(),
                sound.frames(),
                sound.duration()
            );
        }
    }
    Ok(())
}

fn info(info: &SoundInfo) -> String {
    let format = match info.format {
        Format::Wss(compression) => format!("WSS ({compression:?})"),
        Format::OggVorbis => "Ogg Vorbis".to_string(),
        Format::Wav => "WAV".to_string(),
    };
    let mut out = format!(
        "format   {format}\nchannels {}\nrate     {} Hz\n",
        info.channels, info.sample_rate
    );
    if let Some(bits) = info.bits_per_sample {
        out += &format!("bits     {bits}\n");
    }
    if let Some(frames) = info.frames {
        let seconds = frames as f64 / f64::from(info.sample_rate.max(1));
        out += &format!("frames   {frames} ({seconds:.3} s)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_audio_formats::WssCompression;

    #[test]
    fn info_prints_length_in_seconds() {
        let text = info(&SoundInfo {
            format: Format::Wss(WssCompression::Delta8),
            sample_rate: 22050,
            channels: 1,
            bits_per_sample: Some(16),
            frames: Some(44100),
        });
        assert!(text.contains("WSS (Delta8)"), "{text}");
        assert!(text.contains("frames   44100 (2.000 s)"), "{text}");
    }
}
