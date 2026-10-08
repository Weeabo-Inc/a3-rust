//! Control `type` and `style` values (the engine's `CT_*` and `ST_*` constants, as in
//! `\a3\ui_f\hpp\defineResincl.inc`).

/// A control's `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlType {
    Static,
    Button,
    Edit,
    Slider,
    Combo,
    ListBox,
    Toolbox,
    CheckBoxes,
    Progress,
    Html,
    StaticSkew,
    ActiveText,
    Tree,
    StructuredText,
    ContextMenu,
    ControlsGroup,
    ShortcutButton,
    HitZones,
    VehicleToggles,
    ControlsTable,
    XKeyDesc,
    XButton,
    XListBox,
    XSlider,
    XCombo,
    AnimatedTexture,
    Menu,
    MenuStrip,
    CheckBox,
    Object,
    ObjectZoom,
    ObjectContainer,
    ObjectContainerAnim,
    LineBreak,
    User,
    Map,
    MapMain,
    ListNBox,
    ItemSlot,
    ListNBoxCheckable,
    VehicleDirection,
    /// A value the table does not know.
    Other(i32),
}

impl ControlType {
    /// The type for a config `type` value.
    pub fn from_code(code: i32) -> Self {
        use ControlType::*;
        match code {
            0 => Static,
            1 => Button,
            2 => Edit,
            3 => Slider,
            4 => Combo,
            5 => ListBox,
            6 => Toolbox,
            7 => CheckBoxes,
            8 => Progress,
            9 => Html,
            10 => StaticSkew,
            11 => ActiveText,
            12 => Tree,
            13 => StructuredText,
            14 => ContextMenu,
            15 => ControlsGroup,
            16 => ShortcutButton,
            17 => HitZones,
            18 => VehicleToggles,
            19 => ControlsTable,
            40 => XKeyDesc,
            41 => XButton,
            42 => XListBox,
            43 => XSlider,
            44 => XCombo,
            45 => AnimatedTexture,
            46 => Menu,
            47 => MenuStrip,
            77 => CheckBox,
            80 => Object,
            81 => ObjectZoom,
            82 => ObjectContainer,
            83 => ObjectContainerAnim,
            98 => LineBreak,
            99 => User,
            100 => Map,
            101 => MapMain,
            102 => ListNBox,
            103 => ItemSlot,
            104 => ListNBoxCheckable,
            105 => VehicleDirection,
            other => Other(other),
        }
    }

    /// The config `type` value (`ctrlType`).
    pub fn code(self) -> i32 {
        use ControlType::*;
        match self {
            Static => 0,
            Button => 1,
            Edit => 2,
            Slider => 3,
            Combo => 4,
            ListBox => 5,
            Toolbox => 6,
            CheckBoxes => 7,
            Progress => 8,
            Html => 9,
            StaticSkew => 10,
            ActiveText => 11,
            Tree => 12,
            StructuredText => 13,
            ContextMenu => 14,
            ControlsGroup => 15,
            ShortcutButton => 16,
            HitZones => 17,
            VehicleToggles => 18,
            ControlsTable => 19,
            XKeyDesc => 40,
            XButton => 41,
            XListBox => 42,
            XSlider => 43,
            XCombo => 44,
            AnimatedTexture => 45,
            Menu => 46,
            MenuStrip => 47,
            CheckBox => 77,
            Object => 80,
            ObjectZoom => 81,
            ObjectContainer => 82,
            ObjectContainerAnim => 83,
            LineBreak => 98,
            User => 99,
            Map => 100,
            MapMain => 101,
            ListNBox => 102,
            ItemSlot => 103,
            ListNBoxCheckable => 104,
            VehicleDirection => 105,
            Other(code) => code,
        }
    }

    /// Whether the control holds child controls (`class Controls`).
    pub fn is_group(self) -> bool {
        matches!(
            self,
            ControlType::ControlsGroup | ControlType::ControlsTable
        )
    }

    /// Whether the control reacts to clicks like a button.
    pub fn is_button(self) -> bool {
        matches!(
            self,
            ControlType::Button
                | ControlType::ShortcutButton
                | ControlType::ActiveText
                | ControlType::XButton
        )
    }

    /// Whether the control holds a list of rows (`lbAdd`).
    pub fn is_list(self) -> bool {
        matches!(
            self,
            ControlType::ListBox
                | ControlType::Combo
                | ControlType::XListBox
                | ControlType::XCombo
                | ControlType::ListNBox
                | ControlType::ListNBoxCheckable
                | ControlType::Toolbox
        )
    }
}

/// `style` bits.
pub mod style {
    /// Horizontal alignment mask.
    pub const POS: u32 = 0x0F;
    pub const HPOS: u32 = 0x03;
    pub const VPOS: u32 = 0x0C;
    pub const LEFT: u32 = 0x00;
    pub const RIGHT: u32 = 0x01;
    pub const CENTER: u32 = 0x02;
    pub const DOWN: u32 = 0x04;
    pub const UP: u32 = 0x08;
    pub const VCENTER: u32 = 0x0C;
    /// Type mask.
    pub const TYPE: u32 = 0xF0;
    pub const SINGLE: u32 = 0x00;
    pub const MULTI: u32 = 0x10;
    pub const TITLE_BAR: u32 = 0x20;
    pub const PICTURE: u32 = 0x30;
    pub const FRAME: u32 = 0x40;
    pub const BACKGROUND: u32 = 0x50;
    pub const GROUP_BOX: u32 = 0x60;
    pub const GROUP_BOX2: u32 = 0x70;
    pub const HUD_BACKGROUND: u32 = 0x80;
    pub const TILE_PICTURE: u32 = 0x90;
    pub const WITH_RECT: u32 = 0xA0;
    pub const LINE: u32 = 0xB0;
    pub const UPPERCASE: u32 = 0xC0;
    pub const LOWERCASE: u32 = 0xD0;
    pub const SHADOW: u32 = 0x100;
    pub const NO_RECT: u32 = 0x200;
    pub const KEEP_ASPECT_RATIO: u32 = 0x800;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for code in [0, 1, 5, 13, 15, 16, 43, 77, 100, 102, 105, 999] {
            assert_eq!(ControlType::from_code(code).code(), code);
        }
        assert_eq!(ControlType::from_code(15), ControlType::ControlsGroup);
        assert!(ControlType::ShortcutButton.is_button());
        assert!(ControlType::Combo.is_list());
    }
}
