//! The chat area: the engine's global chat list (`DAT_142221660`), configured from
//! `RscChatListDefault` / `RscChatListMission` (`FUN_141398090`), filled by `systemChat`,
//! `sideChat`, `globalChat`, radio protocol and multiplayer chat (`FUN_1413956a0`) and drawn
//! bottom-up (`FUN_141398e40`). See `docs/re/ingame-ui.md`.

use a3_config::ConfigRef;
use a3_ui::draw::{Align, TextStyle, draw_text};
use a3_ui::{DrawList, Eval, Fonts, Rgba, UiMetrics, read_color, read_number, read_text};

/// The chat channels with their own config colours (`colorGlobalChannel`, ...), in engine
/// order.
pub const CHANNELS: [&str; 6] = [
    "GlobalChannel",
    "SideChannel",
    "CommandChannel",
    "GroupChannel",
    "VehicleChannel",
    "DirectChannel",
];

/// The system channel (`systemChat`).
pub const SYSTEM: usize = 0x10;
/// The BattlEye channel.
pub const BATTLEYE: usize = 0x11;
/// Channel slots the list keeps colours for.
pub const SLOTS: usize = 0x42;

/// Seconds after which a message starts fading; it is gone 5 s later.
pub const FADE_START: f32 = 25.0;
pub const FADE_END: f32 = 30.0;

/// The colours of one channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelColors {
    /// The sender's name of others' messages.
    pub text: Rgba,
    /// The background behind the player's own messages.
    pub player_background: Rgba,
    /// The sender's name of the player's own messages.
    pub player_text: Rgba,
}

/// The configuration of the chat list.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatConfig {
    /// Left, bottom-row reference and width of the area, height of one row (UI units).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rows: i32,
    pub font: String,
    /// Text height (UI units).
    pub size: f32,
    pub background: Rgba,
    pub channels: Vec<ChannelColors>,
    pub color_message: Rgba,
    pub color_message_protocol: Rgba,
    pub shadow: i32,
    pub shadow_player: i32,
    pub shadow_color: Rgba,
    /// Horizontal padding (0.008, set by the constructor).
    pub border: f32,
}

impl Default for ChatConfig {
    /// The constructor's values (`FUN_141394bc0`) before a config is loaded.
    fn default() -> Self {
        let channel = |text: Rgba, bg: Rgba| ChannelColors {
            text,
            player_background: bg,
            player_text: [0.0, 0.0, 0.0, 1.0],
        };
        let mut channels = vec![channel([0.8, 0.8, 0.8, 1.0], [0.8, 0.8, 0.8, 0.5]); SLOTS];
        channels[1] = channel([0.0, 0.9, 0.9, 1.0], [0.0, 0.9, 0.9, 0.5]);
        channels[2] = channel([0.9, 0.9, 0.0, 1.0], [0.9, 0.9, 0.0, 0.5]);
        channels[3] = channel([0.1, 0.9, 0.2, 1.0], [0.1, 0.9, 0.2, 0.5]);
        channels[4] = channel([0.9, 0.8, 0.0, 1.0], [0.9, 0.8, 0.0, 0.5]);
        channels[5] = channel([0.9, 0.0, 0.8, 1.0], [0.9, 0.0, 0.8, 0.5]);
        channels[SYSTEM] = channel([1.0, 0.1, 0.1, 1.0], [1.0, 0.1, 0.1, 0.5]);
        channels[BATTLEYE] = channel([1.0, 0.1, 0.1, 1.0], [1.0, 0.1, 0.1, 0.5]);
        ChatConfig {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            rows: 0,
            font: String::new(),
            size: 0.0,
            background: [0.0, 0.0, 0.0, 0.0],
            channels,
            color_message: [1.0, 1.0, 1.0, 1.0],
            color_message_protocol: [0.8, 0.8, 0.8, 1.0],
            shadow: 0,
            shadow_player: 0,
            shadow_color: [0.0, 0.0, 0.0, 0.0],
            border: 0.008,
        }
    }
}

impl ChatConfig {
    /// Loads a `RscChatList*` class (`FUN_141398090`); expressions are evaluated by `eval`.
    pub fn from_class(class: &ConfigRef<'_>, eval: &mut dyn Eval) -> Self {
        let mut c = ChatConfig::default();
        let num = |name: &str, eval: &mut dyn Eval| read_number(class, name, eval);
        c.x = num("x", eval).unwrap_or(0.0);
        c.y = num("y", eval).unwrap_or(0.0);
        c.w = num("w", eval).unwrap_or(0.0);
        c.h = num("h", eval).unwrap_or(0.0);
        // An integer entry: the expression's value rounded _(medium: the engine's rounding)_.
        c.rows = num("rows", eval).map_or(0, |r| r.round() as i32);
        c.font = read_text(class, "font", eval).unwrap_or_default();
        c.size = num("size", eval).unwrap_or(0.0);
        if let Some(v) = read_color(class, "colorBackground", eval) {
            c.background = v;
        }
        let mut load = |slot: usize, name: &str, eval: &mut dyn Eval| {
            let channel = &mut c.channels[slot];
            if let Some(v) = read_color(class, &format!("color{name}"), eval) {
                channel.text = v;
            }
            if let Some(v) = read_color(class, &format!("color{name}PlayerBackground"), eval) {
                channel.player_background = v;
            }
            if let Some(v) = read_color(class, &format!("color{name}PlayerText"), eval) {
                channel.player_text = v;
            }
        };
        for (slot, name) in CHANNELS.iter().enumerate() {
            load(slot, name, eval);
        }
        if class.get("colorSystemChannel").is_array() {
            load(SYSTEM, "SystemChannel", eval);
        }
        if class.get("colorBattlEyeChannel").is_array() {
            load(BATTLEYE, "BattlEyeChannel", eval);
        }
        // Custom radio channels and the other slots take the global channel's colours.
        let global = c.channels[0];
        for slot in (6..=0xf).chain(0x1a..=0x41) {
            c.channels[slot] = global;
        }
        if let Some(v) = read_color(class, "colorMessage", eval) {
            c.color_message = v;
        }
        if let Some(v) = read_color(class, "colorMessageProtocol", eval) {
            c.color_message_protocol = v;
        }
        c.shadow = num("shadow", eval).unwrap_or(0.0) as i32;
        c.shadow_player = num("shadowPlayer", eval).unwrap_or(0.0) as i32;
        if let Some(v) = read_color(class, "shadowColor", eval) {
            c.shadow_color = v;
        }
        c
    }
}

/// One message of the chat list.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    /// Channel slot: 0 global, 1 side, 2 command, 3 group, 4 vehicle, 5 direct, 6.. custom,
    /// [`SYSTEM`].
    pub channel: usize,
    /// The sender's name as shown before the text (`"Name: "`); none for system messages.
    pub sender: Option<String>,
    pub text: String,
    /// Sent by the player himself (player colours).
    pub player: bool,
    /// Radio protocol / system text: drawn in `colorMessageProtocol` and not quoted.
    pub protocol: bool,
    /// UI time it arrived (seconds).
    pub time: f64,
}

impl ChatMessage {
    /// A `systemChat` line _(medium: the flags `systemChat` passes)_.
    pub fn system(text: &str, time: f64) -> Self {
        ChatMessage {
            channel: SYSTEM,
            sender: None,
            text: text.to_owned(),
            player: false,
            protocol: true,
            time,
        }
    }
}

/// Splits `text` into lines no wider than `width` the way the chat list does
/// (`FUN_1413974b0`): characters are measured one by one; a line that overflows breaks after
/// its last space (or before the overflowing character when it has none), and the width
/// count starts again at zero. Returns the byte offsets of the line starts plus the end.
pub fn wrap_offsets(text: &str, width: f32, measure: &mut dyn FnMut(&str) -> f32) -> Vec<usize> {
    let mut offsets = vec![0];
    if width > 0.0 && measure(text) > width {
        let mut acc = 0.0;
        let mut after_space: Option<usize> = None;
        let mut iter = text.char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            let next = iter.peek().map_or(text.len(), |&(j, _)| j);
            if u32::from(c) <= 0x20 {
                after_space = Some(next);
            }
            acc += measure(&text[i..next]);
            if acc > width {
                let brk = after_space.take().unwrap_or(i);
                acc = 0.0;
                offsets.push(brk);
                if brk == 0 {
                    break;
                }
            }
        }
    }
    offsets.push(text.len());
    offsets
}

fn faded(mut c: Rgba, alpha: f32) -> Rgba {
    c[3] *= alpha;
    c
}

/// The chat list: configuration and messages, newest first.
#[derive(Debug, Clone, Default)]
pub struct ChatList {
    pub config: ChatConfig,
    messages: Vec<ChatMessage>,
}

impl ChatList {
    pub fn new(config: ChatConfig) -> Self {
        ChatList {
            config,
            messages: Vec::new(),
        }
    }

    /// Adds a message as the newest.
    pub fn add(&mut self, message: ChatMessage) {
        self.messages.insert(0, message);
    }

    /// The messages, newest first.
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Draws the visible messages at UI time `now` into `list` (`FUN_141398e40`).
    pub fn draw(&self, list: &mut DrawList, fonts: &mut Fonts, m: &UiMetrics, now: f64) {
        let c = &self.config;
        if c.rows <= 0 || c.font.is_empty() {
            return;
        }
        let px = |x: f32| m.x_to_px(x);
        let py = |y: f32| m.rect_to_px([0.0, y, 0.0, 0.0])[1];
        let sx = |w: f32| w * m.viewport_w;
        let sy = |h: f32| h * m.viewport_h;
        let pixels = sy(c.size);
        // Measured in UI units of width.
        let width_of =
            |fonts: &mut Fonts, s: &str| fonts.measure(&c.font, pixels, s) / m.viewport_w;
        let inner = c.w - 2.0 * c.border;
        let mut rows_left = c.rows - 1;
        let mut y = (c.rows - 1) as f32 * c.h + c.y;

        let line = |list: &mut DrawList,
                    fonts: &mut Fonts,
                    text: &str,
                    rect: [f32; 4],
                    text_x: f32,
                    text_y: f32,
                    color: Rgba,
                    background: Rgba,
                    shadow: Option<Rgba>| {
            let r = [px(rect[0]), py(rect[1]), sx(rect[2]), sy(rect[3])];
            list.solid(r, background, None);
            let ty = text_y + (c.h - c.size) * 0.5;
            let mut put = |dx: f32, dy: f32, color: Rgba, list: &mut DrawList| {
                let style = TextStyle {
                    font: &c.font,
                    pixels,
                    color,
                    align: Align::Left,
                    vcenter: false,
                    shadow: 0,
                    shadow_color: color,
                    multiline: false,
                    line_spacing: 1.0,
                };
                let x = px(text_x + dx);
                draw_text(
                    list,
                    fonts,
                    text,
                    [x, py(ty + dy), m.screen.width as f32 - x, pixels],
                    &style,
                    None,
                );
            };
            if let Some(shadow) = shadow {
                put(c.h * 0.075, c.h * 0.1, shadow, list);
            }
            put(0.0, 0.0, color, list);
        };

        for msg in &self.messages {
            if msg.text.is_empty() {
                continue;
            }
            let age = (now - msg.time) as f32;
            let alpha = if age > FADE_END {
                continue;
            } else if age > FADE_START {
                (FADE_END - age) * 0.2
            } else {
                1.0
            };
            let channel = c
                .channels
                .get(msg.channel)
                .copied()
                .unwrap_or(c.channels[0]);
            let (name_color, name_background) = if msg.player {
                (channel.player_text, channel.player_background)
            } else {
                (channel.text, c.background)
            };
            let name_shadow =
                (c.shadow_player == 1 || !msg.player).then(|| faded(c.shadow_color, alpha));
            let body_shadow = (c.shadow == 1).then(|| faded(c.shadow_color, alpha));
            let body = if msg.protocol {
                msg.text.clone()
            } else {
                format!("\"{}\"", msg.text)
            };
            let prefix = match &msg.sender {
                Some(s) if !s.is_empty() => format!("{s}: "),
                _ => String::new(),
            };
            let mut prefix_w = width_of(fonts, &prefix);
            let mut avail = inner - prefix_w;
            if avail < inner * 0.3 {
                prefix_w = inner - inner * 0.3;
                avail = inner * 0.3;
            }
            let offsets = wrap_offsets(&body, avail, &mut |s| width_of(fonts, s));
            let last = offsets.len() as i32 - 2;
            let shown_above = last.min(rows_left);
            line(
                list,
                fonts,
                &prefix,
                [c.x, y - shown_above as f32 * c.h, prefix_w, c.h],
                c.x + c.border,
                y - shown_above as f32 * c.h,
                faded(name_color, alpha),
                faded(name_background, alpha),
                name_shadow,
            );
            let body_color = faded(
                if msg.protocol {
                    c.color_message_protocol
                } else {
                    c.color_message
                },
                alpha,
            );
            let mut index = last;
            while index >= 0 {
                let i = index as usize;
                let mut text = String::new();
                if rows_left == 0 && index != 0 {
                    text.push_str("...");
                }
                text.push_str(&body[offsets[i]..offsets[i + 1]]);
                let text_w = width_of(fonts, &text);
                line(
                    list,
                    fonts,
                    &text,
                    [prefix_w + c.x, y, 2.0 * c.border + text_w, c.h],
                    prefix_w + c.x + c.border,
                    y,
                    body_color,
                    faded(c.background, alpha),
                    body_shadow,
                );
                rows_left -= 1;
                if rows_left < 0 {
                    return;
                }
                y -= c.h;
                index -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is one unit wide.
    fn chars(s: &str) -> f32 {
        s.chars().count() as f32
    }

    #[test]
    fn wrapping_breaks_after_spaces_and_restarts_the_count() {
        assert_eq!(wrap_offsets("short", 10.0, &mut chars), [0, 5]);
        // "hello world again": overflows at 'r' (11th char); breaks after "hello ".
        let t = "hello world again";
        let o = wrap_offsets(t, 10.0, &mut chars);
        assert_eq!(o.first(), Some(&0));
        assert_eq!(o[1], 6);
        assert_eq!(*o.last().unwrap(), t.len());
        // No space: break before the overflowing character.
        assert_eq!(wrap_offsets("abcdefghij", 4.0, &mut chars)[1], 4);
    }

    #[test]
    fn system_messages_are_protocol_text_without_a_sender() {
        let m = ChatMessage::system("Saved", 1.0);
        assert_eq!(m.channel, SYSTEM);
        assert!(m.protocol && !m.player && m.sender.is_none());
    }
}
