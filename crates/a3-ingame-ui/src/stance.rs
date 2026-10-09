//! The stance indicator: `RscInGameUI >> RscStanceInfo`, whose IDC 188 control is the
//! engine's `CStanceIndicator`. Each frame it shows the texture of
//! `CfgInGameUI >> CfgStanceIndicatorTextures >> <state> >> texture<Stance>[Adjust<Dir>]` for
//! the player's stance (`FUN_140aa4ff0`, textures loaded by `FUN_140aa4220`).

use a3_config::ConfigTree;

/// The stance an action map gives (`stance = "ManStance..."`), in the engine's index order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    /// No stance (swimming, ladders, vehicles, ...): the indicator shows nothing.
    Undefined,
    Prone,
    Crouch,
    #[default]
    Stand,
}

/// Weapon resting and deployment, the five texture sets of `CfgStanceIndicatorTextures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StanceState {
    #[default]
    Normal,
    Rested,
    CanDeploy,
    RestedCanDeploy,
    Deployed,
}

/// A stance adjustment the player is making (`Adjust*` textures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StanceAdjust {
    Up,
    Down,
    Left,
    Right,
    #[default]
    None,
}

const STATES: [&str; 5] = [
    "Normal",
    "Rested",
    "CanDeploy",
    "RestedCanDeploy",
    "Deployed",
];
const STANCES: [&str; 3] = ["Prone", "Crouch", "Stand"];
const ADJUSTS: [&str; 5] = ["AdjustUp", "AdjustDown", "AdjustLeft", "AdjustRight", ""];

/// Every texture of `CfgStanceIndicatorTextures`, by state, stance and adjustment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StanceTextures {
    /// `[state][stance][adjust]`; the `Undefined` stance has no textures.
    textures: [[[Option<String>; 5]; 4]; 5],
}

impl StanceTextures {
    /// Reads `CfgInGameUI >> CfgStanceIndicatorTextures`.
    pub fn from_config(config: &ConfigTree) -> Self {
        let mut out = StanceTextures::default();
        let root = config
            .root()
            .get("CfgInGameUI")
            .get("CfgStanceIndicatorTextures");
        for (s, state) in STATES.iter().enumerate() {
            let class = root.get(state);
            if !class.is_class() {
                continue;
            }
            for (p, stance) in STANCES.iter().enumerate() {
                for (a, adjust) in ADJUSTS.iter().enumerate() {
                    let entry = class.get(&format!("texture{stance}{adjust}"));
                    if entry.is_text() {
                        out.textures[s][p + 1][a] = Some(entry.text());
                    }
                }
            }
        }
        out
    }

    /// The texture the indicator shows.
    pub fn texture(
        &self,
        state: StanceState,
        stance: Stance,
        adjust: StanceAdjust,
    ) -> Option<&str> {
        let s = state as usize;
        let p = stance as usize;
        let a = adjust as usize;
        self.textures[s][p][a].as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    #[test]
    fn textures_are_indexed_by_state_stance_and_adjustment() {
        let config = ConfigTree::from_config(
            &parse_text(
                r#"class CfgInGameUI { class CfgStanceIndicatorTextures {
                    class Normal { textureStand = "stand"; textureProneAdjustLeft = "prone_left"; };
                    class Deployed { textureCrouch = "deployed_crouch"; };
                }; };"#,
            )
            .unwrap(),
        );
        let t = StanceTextures::from_config(&config);
        let normal = StanceState::Normal;
        assert_eq!(
            t.texture(normal, Stance::Stand, StanceAdjust::None),
            Some("stand")
        );
        assert_eq!(
            t.texture(normal, Stance::Prone, StanceAdjust::Left),
            Some("prone_left")
        );
        assert_eq!(
            t.texture(StanceState::Deployed, Stance::Crouch, StanceAdjust::None),
            Some("deployed_crouch")
        );
        assert_eq!(t.texture(normal, Stance::Crouch, StanceAdjust::None), None);
        assert_eq!(
            t.texture(normal, Stance::Undefined, StanceAdjust::None),
            None
        );
    }
}
