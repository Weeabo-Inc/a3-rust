//! The A2S_INFO `keywords` string: the game tags the server browser filters on.
//!
//! `docs/re/net-a2s.md`, "Game tags": a comma-separated list of `letter + value` items built from
//! the engine's tag table (RVA 0x20b6b70), joined in index order, at most 128 characters
//! (`"Error, Server tag string too long!"`). Letters the document does not pin down, or values our
//! server has no source for, are left out rather than guessed: a missing tag reads as "unknown"
//! to a filter, and inventing a value would make the browser lie.

/// The longest tag string the engine publishes.
pub const MAX_TAGS_LEN: usize = 128;

/// The tags this server can fill from its own state.
#[derive(Debug, Clone, PartialEq)]
pub struct GameTags {
    /// `b`: BattlEye is enabled. Always false on our server (ADR 0004).
    pub battleye: bool,
    /// `m`: believed "required mods equal" (medium confidence in the document) — `netServer+0x792`.
    pub equal_mods_required: bool,
    /// `r`: the actual version, `222`.
    pub actual_version: u16,
    /// `n`: the required build (server.cfg `requiredBuild`).
    pub required_build: u32,
    /// `t`: the mission game type, first 7 characters (empty with no mission loaded).
    pub mission_game_type: String,
    /// `s`: the session state number (`netServer+0x1038`).
    pub state: u8,
    /// `d`: a dedicated server (always true for us).
    pub dedicated: bool,
    /// `l`: the session is locked (`netServer+0x790`).
    pub locked: bool,
    /// `v`: `verifySignatures` (`netServer+0x793`). Always false on our server (ADR 0004).
    pub verify_signatures: bool,
    /// `g`: the language id, `%d`.
    pub language: u8,
    /// `i`: the difficulty index.
    pub difficulty: u8,
    /// `p`: the platform, always `w`.
    pub platform: char,
    /// `e`: minutes left in the mission (15 by default).
    pub minutes_left: u16,
    /// `f`: `allowedFilePatching` (`netServer+0x7ac`, 0/1/2).
    pub allowed_file_patching: u8,
    /// `c`: `"%d-%d"` of two rounded floats; believed the server's longitude and latitude
    /// (medium confidence in the document), so it is only published when known.
    pub position: Option<(i32, i32)>,
    /// `h`: the content hash string.
    pub content_hash: Option<String>,
    /// `o`: the Steam IP country code.
    pub country: Option<String>,
    /// `j`: `%g` of the first mission parameter float.
    pub mission_parameter_1: Option<f32>,
    /// `k`: `%g` of the second mission parameter float.
    pub mission_parameter_2: Option<f32>,
}

impl Default for GameTags {
    fn default() -> Self {
        Self {
            battleye: false,
            equal_mods_required: false,
            actual_version: crate::handshake::GAME_ACTUAL_VERSION,
            required_build: crate::handshake::GAME_BUILD,
            mission_game_type: String::new(),
            state: 0,
            dedicated: true,
            locked: false,
            verify_signatures: false,
            language: 0,
            difficulty: 0,
            platform: 'w',
            minutes_left: 15,
            allowed_file_patching: 0,
            position: None,
            content_hash: None,
            country: None,
            mission_parameter_1: None,
            mission_parameter_2: None,
        }
    }
}

impl GameTags {
    /// Join the tags in the table's index order, stopping before the 128-character limit.
    pub fn encode(&self) -> String {
        let mut items: Vec<String> = Vec::new();
        items.push(bool_tag('b', self.battleye));
        items.push(bool_tag('m', self.equal_mods_required));
        items.push(format!("r{}", self.actual_version));
        items.push(format!("n{}", self.required_build));
        if !self.mission_game_type.is_empty() {
            let short: String = self.mission_game_type.chars().take(7).collect();
            items.push(format!("t{short}"));
        }
        items.push(format!("s{}", self.state));
        items.push(bool_tag('d', self.dedicated));
        items.push(bool_tag('l', self.locked));
        items.push(bool_tag('v', self.verify_signatures));
        items.push(format!("g{}", self.language));
        items.push(format!("i{}", self.difficulty));
        items.push(format!("p{}", self.platform));
        if let Some((x, y)) = self.position {
            items.push(format!("c{x}-{y}"));
        }
        if let Some(hash) = &self.content_hash {
            items.push(format!("h{hash}"));
        }
        if let Some(country) = &self.country {
            items.push(format!("o{country}"));
        }
        items.push(format!("e{}", self.minutes_left));
        // The document prints these two with `%g`; Rust's `f32` Display is also the shortest form
        // that round-trips, though it never switches to exponent notation.
        if let Some(value) = self.mission_parameter_1 {
            items.push(format!("j{value}"));
        }
        if let Some(value) = self.mission_parameter_2 {
            items.push(format!("k{value}"));
        }
        items.push(format!("f{}", self.allowed_file_patching));

        let mut out = String::new();
        for item in items {
            if out.len() + item.len() + 1 > MAX_TAGS_LEN {
                log::warn!(
                    "a2s tags: dropping `{item}`, the tag string is limited to {MAX_TAGS_LEN} characters"
                );
                break;
            }
            out.push_str(&item);
            out.push(',');
        }
        out
    }
}

/// A boolean tag is `t` or `f` (`docs/re/net-a2s.md`).
fn bool_tag(letter: char, value: bool) -> String {
    format!("{letter}{}", if value { 't' } else { 'f' })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_server_publishes_the_tags_it_knows() {
        let tags = GameTags::default().encode();
        assert_eq!(tags, "bf,mf,r222,n154103,s0,dt,lf,vf,g0,i0,pw,e15,f0,");
        assert!(tags.ends_with(','));
        assert!(tags.len() <= MAX_TAGS_LEN);
    }

    #[test]
    fn optional_tags_appear_only_when_known() {
        let tags = GameTags {
            mission_game_type: "coop_mission_name".into(),
            locked: true,
            position: Some((16, 45)),
            content_hash: Some("abc123".into()),
            country: Some("GB".into()),
            mission_parameter_1: Some(0.5),
            mission_parameter_2: Some(2.0),
            ..GameTags::default()
        }
        .encode();
        assert!(
            tags.contains("tcoop_mi,"),
            "7 characters of the game type: {tags}"
        );
        assert!(tags.contains("lt,"));
        assert!(tags.contains("c16-45,"));
        assert!(tags.contains("habc123,"));
        assert!(tags.contains("oGB,"));
        assert!(tags.contains("j0.5,"));
        assert!(tags.contains("k2,"));
    }

    #[test]
    fn a_tag_that_would_overflow_the_limit_is_dropped() {
        let tags = GameTags {
            content_hash: Some("h".repeat(200)),
            ..GameTags::default()
        }
        .encode();
        assert!(tags.len() <= MAX_TAGS_LEN);
        assert!(!tags.contains('h'));
        // Everything before the oversized tag is still there.
        assert!(tags.starts_with("bf,mf,r222,"));
    }
}
