//! Reads every texture and texHeaders.bin of a real game install. Skipped when `A3_ROOT` is
//! unset. Slow in debug builds; run with `cargo test --release -p a3-paa --test real_data
//! -- --nocapture` for the report.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use a3_paa::{
    Color, Compression, PaaHeader, Procedural, TexHeaders, Texture, TextureType, decode_rgba8,
};
use a3_vfs::{Vfs, optional_mod_dirs};

fn mount() -> Option<Vfs> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));
    Some(vfs)
}

/// Collapses an error message to its kind, for counting failures by reason.
fn reason(error: &a3_paa::Error) -> String {
    let text = error.to_string();
    let cut = text
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(text.len());
    text[..cut].to_owned()
}

/// Counters from scanning a set of textures; merged across threads.
#[derive(Default)]
struct Survey {
    formats: BTreeMap<String, usize>,
    storage: BTreeMap<String, usize>,
    taggs: BTreeMap<String, usize>,
    smallest: BTreeMap<String, usize>,
    failures: BTreeMap<String, Vec<String>>,
    offsets_mismatch: Vec<String>,
    /// Smallest and largest raw size of the levels stored each way, per format.
    level_bytes: BTreeMap<String, (usize, usize)>,
    /// Formats of `.pac` files (the engine reads only DXT from `.pac`).
    pac_formats: BTreeMap<String, usize>,
    /// Textures with a side above 4096 (the engine rejects them).
    oversized: Vec<String>,
    mips: usize,
    bytes: usize,
    decoded: usize,
    rewritten: usize,
}

fn add<K: Ord>(map: &mut BTreeMap<K, usize>, key: K) {
    *map.entry(key).or_default() += 1;
}

impl Survey {
    fn fail(&mut self, error: &a3_paa::Error, context: String) {
        self.failures
            .entry(reason(error))
            .or_default()
            .push(format!("{context}: {error}"));
    }

    fn merge(&mut self, other: Survey) {
        for (mine, theirs) in [
            (&mut self.formats, other.formats),
            (&mut self.storage, other.storage),
            (&mut self.taggs, other.taggs),
            (&mut self.smallest, other.smallest),
        ] {
            for (k, v) in theirs {
                *mine.entry(k).or_default() += v;
            }
        }
        for (k, v) in other.failures {
            self.failures.entry(k).or_default().extend(v);
        }
        self.offsets_mismatch.extend(other.offsets_mismatch);
        for (k, (lo, hi)) in other.level_bytes {
            let e = self.level_bytes.entry(k).or_insert((usize::MAX, 0));
            *e = (e.0.min(lo), e.1.max(hi));
        }
        for (k, v) in other.pac_formats {
            *self.pac_formats.entry(k).or_default() += v;
        }
        self.oversized.extend(other.oversized);
        self.mips += other.mips;
        self.bytes += other.bytes;
        self.decoded += other.decoded;
        self.rewritten += other.rewritten;
    }

    /// Parses `data` and decompresses every mipmap. Every `SAMPLE`th texture is also fully
    /// decoded to RGBA and written back out and re-read.
    fn texture(&mut self, path: &str, data: &[u8], sample: bool) {
        self.bytes += data.len();
        let header = match PaaHeader::read(data) {
            Ok(h) => h,
            Err(e) => return self.fail(&e, path.to_owned()),
        };
        let format = header.meta.format;
        add(&mut self.formats, format!("{format:?}"));
        if path.ends_with(".pac") {
            add(&mut self.pac_formats, format!("{format:?}"));
        }
        if header.mips[0].width > 4096 || header.mips[0].height > 4096 {
            self.oversized.push(path.to_owned());
        }
        for t in &header.meta.other_taggs {
            add(
                &mut self.taggs,
                String::from_utf8_lossy(&t.name).into_owned(),
            );
        }
        let (first, last) = (&header.mips[0], &header.mips[header.mips.len() - 1]);
        if first.width != first.height {
            let class = if format.is_dxt() { "DXT" } else { "raw" };
            add(
                &mut self.smallest,
                format!("{class} {}x{}", last.width, last.height),
            );
        }
        let actual: Vec<u32> = header.mips.iter().map(|m| m.offset as u32).collect();
        if !header.declared_offsets.is_empty() && header.declared_offsets != actual {
            self.offsets_mismatch.push(path.to_owned());
        }
        let mut mips = Vec::new();
        for (index, info) in header.mips.iter().enumerate() {
            let key = format!("{format:?}/{:?}", info.compression);
            let raw = format.data_len(info.width, info.height);
            let range = self
                .level_bytes
                .entry(key.clone())
                .or_insert((usize::MAX, 0));
            *range = (range.0.min(raw), range.1.max(raw));
            add(&mut self.storage, key);
            self.mips += 1;
            match header.read_mip(data, index) {
                Ok(mip) => mips.push(mip),
                Err(e) => self.fail(&e, format!("{path} mip {index}")),
            }
        }
        if !sample || mips.len() != header.mips.len() {
            return;
        }
        if let Err(e) = decode_rgba8(format, &mips[0]) {
            return self.fail(&e, format!("{path} decode"));
        }
        self.decoded += 1;
        let texture = Texture {
            mips,
            ..header.meta
        };
        match texture.to_bytes().and_then(|b| Texture::read(&b)) {
            Ok(back) if back == texture => self.rewritten += 1,
            Ok(_) => self
                .failures
                .entry("rewrite differs".into())
                .or_default()
                .push(path.to_owned()),
            Err(e) => self.fail(&e, format!("{path} rewrite")),
        }
    }
}

/// Every how many textures one is fully decoded and rewritten.
const SAMPLE: usize = 100;

#[test]
fn every_shipped_texture_parses_and_decompresses() {
    let Some(vfs) = mount() else { return };
    let start = Instant::now();
    let mut paths = vfs.glob(r"**\*.paa");
    paths.extend(vfs.glob(r"**\*.pac"));
    // `A3_PAA_FILTER=<substring>` narrows the run while investigating.
    let filter = std::env::var("A3_PAA_FILTER").ok();
    if let Some(filter) = &filter {
        paths.retain(|p| p.as_str().contains(filter.as_str()));
    }

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = AtomicUsize::new(0);
    let mut survey = Survey::default();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Survey::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(i) else { break };
                        let data = vfs.open(path.as_str()).unwrap();
                        local.texture(path.as_str(), &data, i % SAMPLE == 0);
                    }
                    local
                })
            })
            .collect();
        for worker in workers {
            survey.merge(worker.join().unwrap());
        }
    });

    eprintln!(
        "{} textures, {} MiB, {} mipmaps decompressed, {} fully decoded and rewritten ({} rewrites identical) in {:.2?} on {threads} threads",
        paths.len(),
        survey.bytes >> 20,
        survey.mips,
        survey.decoded,
        survey.rewritten,
        start.elapsed()
    );
    eprintln!("formats: {:?}", survey.formats);
    eprintln!("storage: {:?}", survey.storage);
    eprintln!("other TAGGs: {:?}", survey.taggs);
    eprintln!(
        "raw level bytes (min, max) by storage: {:?}",
        survey.level_bytes
    );
    eprintln!(".pac formats: {:?}", survey.pac_formats);
    eprintln!("textures above 4096: {:?}", survey.oversized);
    eprintln!(
        "smallest mipmap of non-square textures: {:?}",
        survey.smallest
    );
    eprintln!(
        "OFFS differs from actual offsets: {} {:?}",
        survey.offsets_mismatch.len(),
        survey.offsets_mismatch.iter().take(5).collect::<Vec<_>>()
    );
    for (why, list) in &survey.failures {
        eprintln!("FAIL x{} {why}", list.len());
        for item in list.iter().take(5) {
            eprintln!("    {item}");
        }
    }
    assert!(
        filter.is_some() || paths.len() > 40_000,
        "found {} textures",
        paths.len()
    );
    assert!(survey.failures.is_empty());
    if filter.is_none() {
        let lzo = format!("{:?}", Compression::Lzo);
        assert!(survey.storage.keys().any(|k| k.ends_with(&lzo)));
    }
}

#[test]
fn every_texheaders_bin_parses_round_trips_and_matches_its_textures() {
    let Some(vfs) = mount() else { return };
    let start = Instant::now();
    let files = vfs.glob(r"**\texheaders.bin");
    let (mut entries, mut checked) = (0usize, 0usize);
    let mut usage: BTreeMap<u32, BTreeMap<String, usize>> = BTreeMap::new();
    let mut mismatches: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut kind_disagrees: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for file in &files {
        let data = vfs.open(file.as_str()).unwrap();
        let headers = match TexHeaders::read(&data) {
            Ok(h) => h,
            Err(e) => {
                failures.push(format!("{file}: {e}"));
                continue;
            }
        };
        if headers.to_bytes() != data[..] {
            failures.push(format!("{file}: does not write back identically"));
        }
        let dir = file.parent().unwrap_or_default();
        for t in &headers.textures {
            entries += 1;
            let stem = t.path.rsplit_once('.').map_or(&t.path[..], |(s, _)| s);
            let suffix = stem.rsplit_once('_').map_or("", |(_, s)| s);
            *usage
                .entry(t.texture_type_index)
                .or_default()
                .entry(suffix.to_ascii_lowercase())
                .or_default() += 1;
            if Some(TextureType::from_path(&t.path)) != t.texture_type() {
                add(&mut kind_disagrees, t.path.clone());
            }
            let path = dir.join(&t.path);
            let Ok(paa) = vfs.open(path.as_str()) else {
                mismatches
                    .entry("texture missing")
                    .or_default()
                    .push(path.to_string());
                continue;
            };
            let Ok(header) = PaaHeader::read(&paa) else {
                continue;
            };
            checked += 1;
            let mut check = |ok: bool, what: &'static str| {
                if !ok {
                    mismatches.entry(what).or_default().push(path.to_string());
                }
            };
            let meta = &header.meta;
            check(t.format == Some(meta.format), "format");
            check(t.file_size as usize == paa.len(), "file size");
            check(t.has_max_color == meta.max_color.is_some(), "has MAXC");
            check(
                t.max_color == meta.max_color.unwrap_or(Color::WHITE),
                "max colour",
            );
            let flags = meta.flags.unwrap_or_default();
            check(t.is_alpha == flags.is_interpolated(), "is_alpha");
            check(t.is_transparent == flags.is_binary(), "is_transparent");
            let avg_alpha = meta.average_color.map_or(0x80, |c| c.a);
            check(
                t.is_alpha_non_opaque == (flags.is_interpolated() && avg_alpha < 0x80),
                "is_alpha_non_opaque",
            );
            let avg = meta.average_color.unwrap_or_default();
            let close = |f: f32, b: u8| (f * 255.0 - f32::from(b)).abs() <= 1.0;
            check(
                close(t.average[0], avg.r)
                    && close(t.average[1], avg.g)
                    && close(t.average[2], avg.b)
                    && close(t.average[3], avg.a),
                "average floats",
            );
            let actual: Vec<_> = header
                .mips
                .iter()
                .map(|m| (m.width, m.height, m.offset as u32))
                .collect();
            let cached: Vec<_> = t
                .mips
                .iter()
                .map(|m| (m.width, m.height, m.offset))
                .collect();
            check(actual == cached, "mipmap table");
        }
    }
    eprintln!(
        "{} texHeaders.bin, {entries} entries, {checked} compared with their PAA, in {:.2?}",
        files.len(),
        start.elapsed()
    );
    for (value, suffixes) in &usage {
        let mut top: Vec<_> = suffixes.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        top.truncate(8);
        eprintln!("texture type {value}: {top:?}");
    }
    for (what, list) in &mismatches {
        eprintln!(
            "MISMATCH {what} x{}: {:?}",
            list.len(),
            &list[..list.len().min(3)]
        );
    }
    eprintln!(
        "TextureType::from_path disagrees with the cached type for {} entries: {:?}",
        kind_disagrees.len(),
        kind_disagrees.keys().collect::<Vec<_>>()
    );
    for f in failures.iter().take(10) {
        eprintln!("FAIL {f}");
    }
    assert!(failures.is_empty());
    assert!(mismatches.is_empty());
    assert!(files.len() > 200, "found {} texHeaders.bin", files.len());
    assert!(kind_disagrees.is_empty());
}

/// Pulls every `#(...)fn(...)` string out of `data`.
fn procedural_strings(data: &[u8], out: &mut BTreeMap<String, usize>) {
    let mut i = 0;
    while let Some(at) = data[i..].windows(2).position(|w| w == b"#(") {
        let start = i + at;
        i = start + 2;
        let tail = &data[start..data.len().min(start + 200)];
        if !tail.get(2).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        // Header `#(...)`, function name, `(`, arguments up to `)`.
        let Some(head_end) = tail.iter().position(|&b| b == b')') else {
            continue;
        };
        let Some(args_end) = tail[head_end + 1..].iter().position(|&b| b == b')') else {
            continue;
        };
        let text = &tail[..head_end + 1 + args_end + 1];
        if text.iter().all(|&b| (0x21..0x7f).contains(&b)) {
            add(out, String::from_utf8_lossy(text).into_owned());
        }
    }
}

#[test]
fn every_procedural_texture_in_shipped_data_parses_and_generates() {
    let Some(vfs) = mount() else { return };
    let start = Instant::now();
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = 0;
    for pattern in [
        r"**\*.rvmat",
        r"**\*.p3d",
        r"**\*.bin",
        r"**\*.cpp",
        r"**\*.hpp",
    ] {
        for path in vfs.glob(pattern) {
            if let Ok(data) = vfs.open(path.as_str()) {
                files += 1;
                procedural_strings(&data, &mut found);
            }
        }
    }
    let mut functions: BTreeMap<String, usize> = BTreeMap::new();
    let mut rejected = Vec::new();
    let mut failed = Vec::new();
    let mut runtime = 0;
    for (text, uses) in &found {
        match Procedural::parse(text) {
            Ok(p) => {
                let name = p.to_string();
                let name = name[name.find(')').unwrap() + 1..]
                    .split('(')
                    .next()
                    .unwrap();
                *functions.entry(name.to_owned()).or_default() += uses;
                if matches!(p.function, a3_paa::ProceduralFunction::Runtime { .. }) {
                    runtime += 1;
                } else if let Err(e) = p.generate() {
                    failed.push(format!("{text}: {e}"));
                }
            }
            Err(e) => rejected.push(format!("{text} ({uses} uses): {e}")),
        }
    }
    eprintln!(
        "{} distinct procedural strings ({} uses) in {files} files, {runtime} runtime sources, in {:.2?}",
        found.len(),
        found.values().sum::<usize>(),
        start.elapsed()
    );
    eprintln!("uses by function: {functions:?}");
    for r in &rejected {
        eprintln!("REJECTED {r}");
    }
    for f in &failed {
        eprintln!("FAILED {f}");
    }
    assert!(found.len() > 1000, "found {} strings", found.len());
    assert!(failed.is_empty());
    // The engine rejects these too (e.g. `fresnelGlass(0.9,0.9)`, `%1` macro placeholders).
    assert!(rejected.len() * 200 < found.len());
}
