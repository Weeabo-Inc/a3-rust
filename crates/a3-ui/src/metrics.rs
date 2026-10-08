//! The UI coordinate system: the 4:3 viewport, the safe zone and the pixel grid.
//!
//! UI positions are in units of the 4:3 *viewport*: `(0, 0)` is its top-left corner and
//! `(1, 1)` its bottom-right. The viewport is centred on the screen and its height is the
//! screen height times the interface size (`uiScale`), so `[0.5, 0.5]` is always the screen
//! centre. The *safe zone* (`safeZoneX/Y/W/H`) is the whole screen in viewport units. See
//! `docs/re/ui.md`.

/// Interface sizes from the game options, as `getResolution select 5` reports them.
pub mod ui_scale {
    pub const VERY_SMALL: f32 = 0.47;
    pub const SMALL: f32 = 0.55;
    pub const NORMAL: f32 = 0.7;
    pub const LARGE: f32 = 0.85;
    pub const VERY_LARGE: f32 = 1.0;
}

/// Screen and interface settings the UI metrics derive from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Interface size, see [`ui_scale`].
    pub ui_scale: f32,
    /// `configFile >> "uiScaleMaxGrids"` (60 in 2.22).
    pub max_grids: f32,
    /// `configFile >> "uiScaleFactor"` (4 in 2.22).
    pub grid_factor: f32,
}

impl Screen {
    /// A screen of `width` x `height` with the 2.22 default config grid values and the
    /// "Normal" interface size.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ui_scale: ui_scale::NORMAL,
            max_grids: 60.0,
            grid_factor: 4.0,
        }
    }

    /// A screen of `width` x `height` with the grid settings of `config`'s root
    /// (`uiScaleMaxGrids`, `uiScaleFactor`; `bin\config.bin` in the shipped game) and the
    /// "Normal" interface size.
    pub fn from_config(width: u32, height: u32, config: &a3_config::ConfigTree) -> Self {
        let mut screen = Self::new(width, height);
        let root = config.root();
        for (name, slot) in [
            ("uiScaleMaxGrids", &mut screen.max_grids),
            ("uiScaleFactor", &mut screen.grid_factor),
        ] {
            let entry = root.get(name);
            if entry.is_number() {
                *slot = entry.number();
            }
        }
        screen
    }
}

/// Everything derived from a [`Screen`]: the values of `safeZone*`, `pixelW/H`, `pixelGrid*`
/// and `getResolution`, and conversions between UI units and pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMetrics {
    pub screen: Screen,
    /// Width of the 4:3 viewport in pixels.
    pub viewport_w: f32,
    /// Height of the 4:3 viewport in pixels.
    pub viewport_h: f32,
    pub safe_zone_x: f32,
    pub safe_zone_y: f32,
    pub safe_zone_w: f32,
    pub safe_zone_h: f32,
    /// One pixel in UI units horizontally (`pixelW`).
    pub pixel_w: f32,
    /// One pixel in UI units vertically (`pixelH`).
    pub pixel_h: f32,
    /// `pixelGridBase`: screen height / `uiScaleMaxGrids`.
    pub pixel_grid_base: f32,
    /// `pixelGridNoUIScale`: the base rounded to a multiple of `uiScaleFactor`.
    pub pixel_grid_no_ui_scale: f32,
    /// `pixelGrid`: base times interface size, rounded to a multiple of `uiScaleFactor`.
    pub pixel_grid: f32,
}

impl UiMetrics {
    /// Derives the metrics of `screen`.
    pub fn new(screen: Screen) -> Self {
        let width = screen.width.max(1) as f32;
        let height = screen.height.max(1) as f32;
        // The 4:3 area that fits the screen, scaled by the interface size.
        let fit_h = height.min(width * 3.0 / 4.0);
        let viewport_h = fit_h * screen.ui_scale;
        let viewport_w = viewport_h * 4.0 / 3.0;
        let safe_zone_w = width / viewport_w;
        let safe_zone_h = height / viewport_h;
        let pixel_grid_base = height / screen.max_grids;
        let factor = screen.grid_factor.max(1.0);
        // The engine rounds with the FPU's default mode (to nearest, ties to even).
        let round = |v: f32| (v / factor).round_ties_even() * factor;
        Self {
            screen,
            viewport_w,
            viewport_h,
            safe_zone_x: -(safe_zone_w - 1.0) / 2.0,
            safe_zone_y: -(safe_zone_h - 1.0) / 2.0,
            safe_zone_w,
            safe_zone_h,
            pixel_w: 1.0 / viewport_w,
            pixel_h: 1.0 / viewport_h,
            pixel_grid_base,
            pixel_grid_no_ui_scale: round(pixel_grid_base),
            pixel_grid: round(pixel_grid_base * screen.ui_scale),
        }
    }

    /// Screen aspect ratio (`getResolution select 4`).
    pub fn aspect(&self) -> f32 {
        self.screen.width.max(1) as f32 / self.screen.height.max(1) as f32
    }

    /// UI x coordinate to screen pixels.
    pub fn x_to_px(&self, x: f32) -> f32 {
        (x - self.safe_zone_x) * self.viewport_w
    }

    /// UI y coordinate to screen pixels.
    pub fn y_to_px(&self, y: f32) -> f32 {
        (y - self.safe_zone_y) * self.viewport_h
    }

    /// A UI rectangle `[x, y, w, h]` to screen pixels.
    pub fn rect_to_px(&self, r: [f32; 4]) -> [f32; 4] {
        [
            self.x_to_px(r[0]),
            self.y_to_px(r[1]),
            r[2] * self.viewport_w,
            r[3] * self.viewport_h,
        ]
    }

    /// Screen pixels to UI coordinates.
    pub fn px_to_ui(&self, px: f32, py: f32) -> (f32, f32) {
        (
            px / self.viewport_w + self.safe_zone_x,
            py / self.viewport_h + self.safe_zone_y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn wiki_values_for_1080p_normal() {
        // BI wiki "Arma 3: Pixel Grid System": 1080p / 16:9 / Normal.
        let m = UiMetrics::new(Screen::new(1920, 1080));
        assert!(close(m.safe_zone_x, -0.452381));
        assert!(close(m.safe_zone_y, -0.214286));
        assert!(close(m.safe_zone_w, 1.90476));
        assert!(close(m.safe_zone_h, 1.42857));
        assert!(close(m.viewport_h, 756.0));
        assert!(close(m.viewport_w, 1008.0));
        assert!(close(m.pixel_h, 1.0 / 756.0));
    }

    #[test]
    fn centre_maps_to_screen_centre() {
        let m = UiMetrics::new(Screen::new(2560, 1080));
        assert!(close(m.x_to_px(0.5), 1280.0));
        assert!(close(m.y_to_px(0.5), 540.0));
        let (x, y) = m.px_to_ui(1280.0, 540.0);
        assert!(close(x, 0.5) && close(y, 0.5));
    }

    #[test]
    fn safe_zone_covers_the_screen() {
        let m = UiMetrics::new(Screen::new(1280, 720));
        let r = m.rect_to_px([m.safe_zone_x, m.safe_zone_y, m.safe_zone_w, m.safe_zone_h]);
        assert!(close(r[0], 0.0) && close(r[1], 0.0));
        assert!(close(r[2], 1280.0) && close(r[3], 720.0));
    }

    #[test]
    fn pixel_grid_rounds_to_the_factor() {
        // 1080 / 60 = 18; 18 / 4 = 4.5 rounds to 4 (ties to even); 18 * 0.7 / 4 = 3.15 -> 3.
        let m = UiMetrics::new(Screen::new(1920, 1080));
        assert!(close(m.pixel_grid_base, 18.0));
        assert!(close(m.pixel_grid_no_ui_scale, 16.0));
        assert!(close(m.pixel_grid, 12.0));
    }

    #[test]
    fn grid_values_come_from_the_config() {
        let config = a3_config::parse_text("uiScaleMaxGrids = 64; uiScaleFactor = 8;").unwrap();
        let s = Screen::from_config(1920, 1080, &a3_config::ConfigTree::from_config(&config));
        assert_eq!(s.max_grids, 64.0);
        assert_eq!(s.grid_factor, 8.0);
        // 1080 / 64 = 16.875 -> 16 (ties to even); 16.875 * 0.7 / 8 = 1.476... -> 8.
        let m = UiMetrics::new(s);
        assert!(close(m.pixel_grid_no_ui_scale, 16.0));
        assert!(close(m.pixel_grid, 8.0));

        // A config without the entries keeps the 2.22 values (`bin\config.bin`).
        let config = a3_config::parse_text("class CfgPatches {};").unwrap();
        let s = Screen::from_config(1920, 1080, &a3_config::ConfigTree::from_config(&config));
        assert_eq!(s.max_grids, 60.0);
        assert_eq!(s.grid_factor, 4.0);
        assert_eq!(s.ui_scale, ui_scale::NORMAL);
    }

    #[test]
    fn narrow_screens_fit_the_width() {
        // 5:4: the 4:3 area is limited by the width.
        let m = UiMetrics::new(Screen {
            ui_scale: ui_scale::VERY_LARGE,
            ..Screen::new(1280, 1024)
        });
        assert!(close(m.viewport_w, 1280.0));
        assert!(close(m.safe_zone_x, 0.0));
        assert!(m.safe_zone_h > 1.0);
    }
}
