//! The preprocessor's view of the outside world: clock, random numbers and game version.
//!
//! Injected so tests are deterministic.

use chrono::{Datelike, Timelike};

/// A calendar date and wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    /// Full year, e.g. 2026.
    pub year: i32,
    /// 1..=12.
    pub month: u32,
    /// 1..=31.
    pub day: u32,
    /// 0..=23.
    pub hour: u32,
    /// 0..=59.
    pub minute: u32,
    /// 0..=59.
    pub second: u32,
}

/// One instant, as the date/time built-in macros need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    /// Local time (`__DATE_ARR__`, `__DATE_STR__`, `__TIME__`).
    pub local: DateTime,
    /// UTC time (`__TIME_UTC__`, `__DATE_STR_ISO8601__`, `__DAY__`, `__MONTH__`, `__YEAR__`).
    pub utc: DateTime,
    /// Seconds since the Unix epoch (`__TIMESTAMP_UTC__`).
    pub unix: i64,
}

/// Source of the current time.
pub trait Clock {
    /// The current instant.
    fn now(&self) -> Now;
}

/// The real system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

fn from_chrono<T: Datelike + Timelike>(t: &T) -> DateTime {
    DateTime {
        year: t.year(),
        month: t.month(),
        day: t.day(),
        hour: t.hour(),
        minute: t.minute(),
        second: t.second(),
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Now {
        let utc = chrono::Utc::now();
        let local = utc.with_timezone(&chrono::Local);
        Now {
            local: from_chrono(&local),
            utc: from_chrono(&utc),
            unix: utc.timestamp(),
        }
    }
}

/// A clock frozen at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub Now);

impl Clock for FixedClock {
    fn now(&self) -> Now {
        self.0
    }
}

/// Source of random numbers for `__RAND_INT*__` / `__RAND_UINT*__`.
pub trait RandomSource {
    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64;
}

/// A small, seeded SplitMix64 generator. Deterministic for a given seed.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// A generator starting from `seed`.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A generator seeded from the system clock.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self(nanos)
    }
}

impl RandomSource for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The game version reported by `__GAME_VER__`, `__GAME_VER_MAJ__`, `__GAME_VER_MIN__` and
/// `__GAME_BUILD__`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameVersion {
    /// Major version (2).
    pub major: u32,
    /// Minor version (22).
    pub minor: u32,
    /// Build number (154103).
    pub build: u32,
}

impl Default for GameVersion {
    /// The version in [`a3_core::GAME_VERSION`].
    fn default() -> Self {
        let mut parts = a3_core::GAME_VERSION
            .split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        let build = parts.next_back().unwrap_or(0);
        Self {
            major,
            minor,
            build,
        }
    }
}
