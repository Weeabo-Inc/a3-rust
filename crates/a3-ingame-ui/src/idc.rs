//! The IDCs and IDDs the engine fills in the in-game displays, named as in the game's
//! `\a3\ui_f\hpp\defineResincl.inc` (`IDC_IGUI_*`). Which control of a unit info display the
//! engine drives is decided by its IDC alone (`DisplayUnitInfo`'s control factory,
//! `FUN_1411b79a0`); see `docs/re/ingame-ui.md`.

/// `IDD_UNITINFO`: every `RscInGameUI` unit info display.
pub const IDD_UNITINFO: i32 = 300;
/// `IDD_HINT`.
pub const IDD_HINT: i32 = 301;
/// `IDD_TASKHINT`.
pub const IDD_TASKHINT: i32 = 302;
/// `IDD_STANCEINFO`.
pub const IDD_STANCEINFO: i32 = 303;

/// Weapon name.
pub const WEAPON: i32 = 118;
/// `"<ammo> | <magazines>"` (`STR_UI_AMMO` / `STR_UI_AMMO_EMPTY`).
pub const AMMO: i32 = 119;
/// The vehicle's display name.
pub const VEHICLE: i32 = 120;
/// The unit info background: shown while the weapon, the gunner weapon or the speed is.
pub const BG: i32 = 124;
/// Fire mode name.
pub const WEAPON_MODE: i32 = 149;
/// The gunner's weapon (vehicles).
pub const WEAPON_GUNNER: i32 = 150;
/// Throwable count (`"x%d"`), countermeasure ammo in vehicles.
pub const COUNTER_MEASURES_AMMO: i32 = 151;
/// Throwable magazine name, countermeasure mode in vehicles.
pub const COUNTER_MEASURES_MODE: i32 = 152;
/// Reload progress bar.
pub const VALUE_RELOAD: i32 = 154;
/// Magazine `displayNameShort`.
pub const WEAPON_AMMO: i32 = 155;
/// Rounds in the loaded magazine (`"%d"`).
pub const AMMOCOUNT: i32 = 184;
/// Further magazines (`"| %d"`, empty without any).
pub const MAGCOUNT: i32 = 185;
/// Fire mode texture (`CfgInGameUI >> CfgWeaponModeTextures`).
pub const WEAPON_MODE_TEXTURE: i32 = 187;
/// The stance indicator picture (`RscStanceInfo`).
pub const STANCE_INDICATOR: i32 = 188;
/// Freefall speed (`STR_UI_SPEED_FREEFALL`).
pub const SPEED_FREEFALL: i32 = 380;
/// Freefall vertical speed (`STR_UI_SPEED_VERTICAL_FREEFALL`).
pub const SPEED_VERTICAL_FREEFALL: i32 = 381;
/// Freefall altitude (`STR_UI_ALT_FREEFALL`).
pub const ALT_FREEFALL: i32 = 382;
/// Always `"0"` (no `defineResincl.inc` name).
pub const ZERO_26006: i32 = 26006;
/// Total capacity of the loaded and spare magazines (no `defineResincl.inc` name).
pub const TOTAL_CAPACITY_26106: i32 = 26106;
/// Capacity of the magazine type (no `defineResincl.inc` name).
pub const CAPACITY_26206: i32 = 26206;
