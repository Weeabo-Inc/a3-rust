//! Long Ogg Vorbis files decoded on a background thread while they play.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use a3_audio_formats::VorbisStream;

use crate::Result;

/// Decoded chunks buffered ahead of playback (each is one Vorbis packet, at most a few thousand
/// frames).
const CHUNKS_AHEAD: usize = 32;

/// A streamed sound being decoded by its own thread. Hand it to the mixer with
/// [`crate::Source::Stream`].
pub struct Stream {
    rx: Receiver<Vec<i16>>,
    sample_rate: u32,
    channels: u16,
    total_frames: Option<u64>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

impl Stream {
    /// Starts decoding an Ogg Vorbis file. With `looping`, the decoder rewinds at the end and
    /// the stream never ends.
    pub fn open<D>(data: D, looping: bool) -> Result<Self>
    where
        D: AsRef<[u8]> + Clone + Send + 'static,
    {
        let mut decoder = VorbisStream::new(data)?;
        let (sample_rate, channels) = (decoder.sample_rate(), decoder.channels());
        let total_frames = decoder.total_frames();
        let (tx, rx) = sync_channel(CHUNKS_AHEAD);
        std::thread::Builder::new()
            .name("audio-stream".into())
            .spawn(move || decode_loop(&mut decoder, &tx, looping))
            .map_err(|e| crate::Error::Device(e.to_string()))?;
        Ok(Self {
            rx,
            sample_rate,
            channels,
            total_frames,
        })
    }

    /// Frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Length in frames of one pass through the file.
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    pub(crate) fn into_reader(self) -> StreamReader {
        StreamReader {
            rx: self.rx,
            channels: self.channels.max(1),
            sample_rate: self.sample_rate,
            frames: VecDeque::new(),
            base: 0,
            ended: false,
            underruns: 0,
        }
    }
}

fn decode_loop<D: AsRef<[u8]> + Clone>(
    decoder: &mut VorbisStream<D>,
    tx: &SyncSender<Vec<i16>>,
    looping: bool,
) {
    loop {
        match decoder.next_chunk() {
            Ok(Some(chunk)) => {
                if chunk.is_empty() {
                    continue;
                }
                if tx.send(chunk).is_err() {
                    return; // the voice is gone
                }
            }
            Ok(None) if looping => {
                if let Err(e) = decoder.rewind() {
                    log::warn!("audio stream: cannot rewind: {e}");
                    return;
                }
            }
            Ok(None) => return,
            Err(e) => {
                log::warn!("audio stream: decode error: {e}");
                return;
            }
        }
    }
}

/// The mixer side of a [`Stream`]: decoded frames from `base` on.
pub(crate) struct StreamReader {
    rx: Receiver<Vec<i16>>,
    channels: u16,
    pub sample_rate: u32,
    frames: VecDeque<[f32; 2]>,
    base: u64,
    ended: bool,
    pub underruns: u64,
}

impl StreamReader {
    /// Frame `index` (never below a released frame). Silence while the decoder is behind; `None`
    /// once the stream has ended.
    pub fn frame(&mut self, index: u64) -> Option<[f32; 2]> {
        while index >= self.base + self.frames.len() as u64 {
            if self.ended {
                return None;
            }
            match self.rx.try_recv() {
                Ok(chunk) => self.push(&chunk),
                Err(TryRecvError::Empty) => {
                    self.underruns += 1;
                    return Some([0.0; 2]);
                }
                Err(TryRecvError::Disconnected) => self.ended = true,
            }
        }
        let offset = index.checked_sub(self.base)?;
        self.frames.get(usize::try_from(offset).ok()?).copied()
    }

    /// Drops frames before `index`.
    pub fn release(&mut self, index: u64) {
        while self.base < index && !self.frames.is_empty() {
            self.frames.pop_front();
            self.base += 1;
        }
    }

    fn push(&mut self, chunk: &[i16]) {
        let to_f32 = |v: i16| f32::from(v) / 32768.0;
        let n = usize::from(self.channels);
        for frame in chunk.chunks_exact(n) {
            let left = to_f32(frame[0]);
            let right = if n > 1 { to_f32(frame[1]) } else { left };
            self.frames.push_back([left, right]);
        }
    }
}
