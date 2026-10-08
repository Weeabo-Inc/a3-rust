//! Ogg Vorbis, through `lewton`.

use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;

use crate::{Error, Format, Result, Sound, SoundInfo};

fn reader(data: &[u8]) -> Result<OggStreamReader<Cursor<&[u8]>>> {
    OggStreamReader::new(Cursor::new(data)).map_err(|e| Error::Vorbis(e.to_string()))
}

/// Reads the Vorbis identification header from the first Ogg page, without setting up a decoder.
pub fn probe(data: &[u8]) -> Result<SoundInfo> {
    // Page header: "OggS", version, flags, granule (8), serial, sequence, CRC, segment count,
    // then the segment table. The identification packet starts the first page.
    let segments = usize::from(*data.get(26).ok_or(Error::Truncated)?);
    let packet = data.get(27 + segments..).ok_or(Error::Truncated)?;
    let id = packet.get(..16).ok_or(Error::Truncated)?;
    if id[0] != 1 || &id[1..7] != b"vorbis" {
        return Err(Error::Unsupported("Ogg stream is not Vorbis".into()));
    }
    let channels = u16::from(id[11]);
    let sample_rate = u32::from_le_bytes([id[12], id[13], id[14], id[15]]);
    if channels == 0 || sample_rate == 0 {
        return Err(Error::Malformed(
            "Vorbis header with 0 channels or rate".into(),
        ));
    }
    Ok(SoundInfo {
        format: Format::OggVorbis,
        sample_rate,
        channels,
        bits_per_sample: None,
        frames: last_granule_position(data),
    })
}

pub fn decode(data: &[u8]) -> Result<Sound> {
    let mut reader = reader(data)?;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let mut samples = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|e| Error::Vorbis(e.to_string()))?
    {
        samples.extend_from_slice(&packet);
    }
    // The last page's granule position is the stream length; the final packet decodes padding
    // past it, which a player must drop. `lewton` leaves that to the caller.
    if let Some(frames) = last_granule_position(data) {
        let len = usize::try_from(frames)
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::from(channels));
        samples.truncate(len);
    }
    Ok(Sound {
        sample_rate,
        channels,
        samples,
    })
}

/// The granule position (for Vorbis: the frame count) of the last Ogg page.
fn last_granule_position(data: &[u8]) -> Option<u64> {
    // An Ogg page is at most 65,307 bytes, so the last page starts within that many bytes.
    let tail_start = data.len().saturating_sub(65_307 + 27);
    let tail = &data[tail_start..];
    let page = tail.windows(4).rposition(|w| w == b"OggS")?;
    let granule = tail.get(page + 6..page + 14)?;
    let value = u64::from_le_bytes(granule.try_into().ok()?);
    // -1 marks a page on which no packet ends.
    (value != u64::MAX).then_some(value)
}

/// Incremental Ogg Vorbis decoding, one Vorbis packet at a time, for streaming long files.
///
/// `D` is the whole file (for example a memory-mapped VFS entry); only the decoded audio is
/// produced in chunks.
pub struct VorbisStream<D: AsRef<[u8]> + Clone> {
    data: D,
    reader: OggStreamReader<Cursor<D>>,
    total_frames: Option<u64>,
    emitted_frames: u64,
}

impl<D: AsRef<[u8]> + Clone> VorbisStream<D> {
    /// Opens a stream and reads its headers.
    pub fn new(data: D) -> Result<Self> {
        let total_frames = last_granule_position(data.as_ref());
        let reader = OggStreamReader::new(Cursor::new(data.clone()))
            .map_err(|e| Error::Vorbis(e.to_string()))?;
        Ok(Self {
            data,
            reader,
            total_frames,
            emitted_frames: 0,
        })
    }

    /// Frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.reader.ident_hdr.audio_sample_rate
    }

    /// Interleaved channels per frame.
    pub fn channels(&self) -> u16 {
        u16::from(self.reader.ident_hdr.audio_channels)
    }

    /// Stream length in frames, from the last page's granule position.
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    /// The next chunk of interleaved 16-bit samples, or `None` at the end of the stream. Chunks
    /// may be empty; the padding past the stream length is dropped as in [`crate::decode`].
    pub fn next_chunk(&mut self) -> Result<Option<Vec<i16>>> {
        let Some(mut packet) = self
            .reader
            .read_dec_packet_itl()
            .map_err(|e| Error::Vorbis(e.to_string()))?
        else {
            return Ok(None);
        };
        let channels = u64::from(self.channels().max(1));
        if let Some(total) = self.total_frames {
            let left = total.saturating_sub(self.emitted_frames);
            if packet.len() as u64 / channels > left {
                packet.truncate(usize::try_from(left * channels).unwrap_or(usize::MAX));
            }
        }
        self.emitted_frames += packet.len() as u64 / channels;
        Ok(Some(packet))
    }

    /// Restarts the stream from its first frame.
    pub fn rewind(&mut self) -> Result<()> {
        self.reader = OggStreamReader::new(Cursor::new(self.data.clone()))
            .map_err(|e| Error::Vorbis(e.to_string()))?;
        self.emitted_frames = 0;
        Ok(())
    }
}
