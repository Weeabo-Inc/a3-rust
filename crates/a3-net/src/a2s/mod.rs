//! Steam server queries (A2S) answered directly, without the Steamworks SDK.
//!
//! The official dedicated server hands its data to Steamworks and lets the library answer on the
//! query port (game port + 1, 2303 by default). `docs/re/net-a2s.md` and ADR 0004 have our server
//! speak the standard Source query protocol itself with the same values, which is what this module
//! does: [`query`] holds the request/answer codec and the challenge table, [`rules`] the binary
//! mod/DLC block Arma packs into A2S_RULES, and [`tags`] the game-tag string of A2S_INFO.

pub mod query;
pub mod rules;
pub mod tags;

pub use query::{
    APP_ID, APP_ID_SHORT, CHALLENGE_LIFETIME, CHALLENGE_UNSET, ChallengeTable, Info, PlayerEntry,
    Query, RESPONSE_CHALLENGE, RESPONSE_INFO, RESPONSE_PLAYER, RESPONSE_RULES, Responder, Rule,
    ServerState, challenge_response, parse_query, player_response, rules_response,
};
pub use rules::{Mod, RulesBlock};
pub use tags::GameTags;
