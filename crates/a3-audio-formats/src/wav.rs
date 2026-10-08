//! RIFF WAVE, through `hound`.

use std::io::Cursor;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::{Error, Format, Result, Sound, SoundInfo};

fn wav_err(e: hound::Error) -> Error {
    Error::Wav(e.to_string())
}

pub fn probe(data: &[u8]) -> Result<SoundInfo> {
    let reader = WavReader::new(Cursor::new(data)).map_err(wav_err)?;
    let spec = reader.spec();
    Ok(SoundInfo {
        format: Format::Wav,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: Some(spec.bits_per_sample),
        frames: Some(u64::from(reader.duration())),
    })
}

pub fn decode(data: &[u8]) -> Result<Sound> {
    let mut reader = WavReader::new(Cursor::new(data)).map_err(wav_err)?;
    let spec = reader.spec();
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|s| s.map(|v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16))
            .collect::<std::result::Result<Vec<_>, _>>(),
        (SampleFormat::Int, bits @ 1..=16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v << (16 - bits)))
            .collect(),
        (SampleFormat::Int, bits @ 17..=32) => reader
            .samples::<i32>()
            .map(|s| s.map(|v| (v >> (bits - 16)) as i16))
            .collect(),
        (format, bits) => {
            return Err(Error::Unsupported(format!(
                "WAV {format:?} with {bits} bits per sample"
            )));
        }
    }
    .map_err(wav_err)?;
    Ok(Sound {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

pub fn encode(sound: &Sound) -> Vec<u8> {
    let spec = WavSpec {
        channels: sound.channels,
        sample_rate: sound.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(&mut out, spec).expect("writing to memory");
        let mut samples = writer.get_i16_writer(sound.samples.len() as u32);
        for &s in &sound.samples {
            samples.write_sample(s);
        }
        samples.flush().expect("writing to memory");
        writer.finalize().expect("writing to memory");
    }
    out.into_inner()
}
