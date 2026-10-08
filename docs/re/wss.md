# WSS sound format (and the other sound files)

Implemented in `crates/a3-audio-formats`. Sources: the WSS reader of `arma3_x64.exe` (build
2.22.0.154103, addresses are VAs in the Ghidra project) and a survey of every sound file in the
install. Confidence: **high** unless marked.

## Survey

| Kind | Files |
|---|---|
| `.ogg` (all Ogg Vorbis; 1 ch 44.1 kHz: 178,512, 1 ch 48 kHz: 5,805, 2 ch 44.1 kHz: 986, rest 8–88.2 kHz) | 185,540 |
| `.wss` delta-8 (`compression = 8`, 16-bit; 1 ch: 11,051, 2 ch: 8,498; 8–96 kHz) | 19,049 |
| `.wss` PCM (`compression = 0`; 16-bit: 400, 24-bit: 2, 8-bit: 2) | 404 |
| `.wss` delta-4 (`compression = 4`) | 0 |
| `.wav` | 0 |

Every file probes, every WSS fully decodes, and a sample of Ogg files fully decodes with the
frame count of the last page's granule position. 380 of 404 PCM WSS and 5,213 of 19,049 delta-8
WSS reach full scale (±32767) somewhere, so game sounds are normalised to 0 dBFS.

## Layout

All little-endian. The reader (`0x1403991b0`) reads 8 bytes, checks the signature, then reads an
18-byte `WAVEFORMATEX`; sample data starts at byte 26.

```
char[4]  "WSS0"                0x30535357
u32      compression           0 PCM, 4 delta-4, 8 delta-8; the engine keeps only the low byte
u16      format tag            1 (PCM) in every file
u16      channels
u32      sample rate
u32      average bytes/second  as for the decoded 16-bit PCM
u16      block align
u16      bits per sample       16 for delta-coded files
u16      extra size            0 except 18 files (sheep, seagull, click, air raid) with junk
u8[]     sample data           to end of file
```

A trailing partial frame (data length not a multiple of the frame size) is ignored _(medium:
the engine's handling of it is not checked)_.

## PCM (`compression = 0`)

Interleaved PCM as in a WAV file: 8-bit unsigned, 16/24-bit signed. `a3-audio-formats` returns
16-bit samples (24-bit keeps the top 16 bits).

## Delta-8 (`compression = 8`)

One byte per output sample, interleaved by channel. Decoder `0x140398da0`, table built at start-up
by `0x14001ce20` into a 256-entry `int` array indexed by the signed byte:

```
table[0]    = 0
table[c]    = round_f32(pow(B, c)),  c = 1..127     B = 1.0853122 (f32 0x3f8aeb83, ~32767^(1/127))
table[-c]   = -table[c]
table[-128] = 0                                     (never written)
```

`pow` runs in double precision; the result is narrowed to `f32` and converted with `cvtss2si`
(round to nearest even). Endpoints: `table[1] = 1`, `table[64] = 189`, `table[127] = 32768`.

Each channel keeps a running sum in an `int`, starting at 0: `sum += table[code]`. The output
sample is the sum clamped to `-32768..=32767`; **the sum itself is not clamped**, so a channel
that overshoots full scale comes back by the same deltas.

## Delta-4 (`compression = 4`)

Not in the install; decoder `0x140398af0`. Each byte holds two 4-bit codes, high nibble first.
Stereo: the high nibble is the left sample and the low nibble the right one; mono: two
consecutive samples. Delta table (`0x141aad880`):

```
-8192 -4096 -2048 -1024 -512 -256 -64 0 64 256 512 1024 2048 4096 8192 0
```

Running sums and output clamping as for delta-8. **Medium** (read from the decompiled code, no
sample file to test against).

## Ogg Vorbis

Standard Ogg Vorbis, decoded with `lewton`. The last page's granule position is the stream
length in frames; the final packet decodes padding past it, which is dropped.

## Open questions

- Whether the engine streams large WSS/Ogg files rather than decoding them whole (the WSS
  reader decodes on demand from an offset; `0x140398ab0` treats files of 256 KiB+ of PCM as
  "large").
- The meaning of the junk extra-size field (it does not change the header size).
