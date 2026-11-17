use super::global_hotkey::{Hotkey, KeyCode, Modifiers};
use crate::host_os::HostOs;

pub const SHIFT_MASK: u16 = 0x0001;
pub const LOCK_MASK: u16 = 0x0002;
pub const CONTROL_MASK: u16 = 0x0004;
pub const MODIFIER_BITS: u16 = 0x00FF;

const MODIFIER_ROWS: usize = 8;
const FIRST_MOD_ROW: usize = 3;

pub const XK_NUM_LOCK: u32 = 0xFF7F;
pub const XK_ALT_L: u32 = 0xFFE9;
pub const XK_ALT_R: u32 = 0xFFEA;
pub const XK_SUPER_L: u32 = 0xFFEB;
pub const XK_SUPER_R: u32 = 0xFFEC;

pub const KEYSYM_NOT_ON_KEYBOARD: i32 = -1;
pub const CONNECTION_LOST: i32 = -2;

/// keymap from GetKeyboardMapping, per_keycode keysyms for every keycode from min_keycode on
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeyboardMap {
    min_keycode: u8,
    per_keycode: usize,
    keysyms: Vec<u32>,
}

impl KeyboardMap {
    pub fn new(min_keycode: u8, per_keycode: u8, keysyms: Vec<u32>) -> Self {
        Self {
            min_keycode,
            per_keycode: usize::from(per_keycode),
            keysyms,
        }
    }

    /// empty for a keycode outside the map
    pub fn keysyms(&self, keycode: u8) -> &[u32] {
        if self.per_keycode == 0 || keycode < self.min_keycode {
            return &[];
        }
        let start = usize::from(keycode - self.min_keycode) * self.per_keycode;
        self.keysyms.get(start..start + self.per_keycode).unwrap_or(&[])
    }

    fn keycode_count(&self) -> usize {
        self.keysyms.len().checked_div(self.per_keycode).unwrap_or(0)
    }

    /// same rule as XKeysymToKeycode, column by column
    /// the lowest keycode in the first column that has it
    pub fn keycode_for(&self, keysym: u32) -> Option<u8> {
        if keysym == 0 {
            return None;
        }
        (0..self.per_keycode).find_map(|column| {
            (0..self.keycode_count())
                .find(|index| self.keysyms[index * self.per_keycode + column] == keysym)
                .and_then(|index| u8::try_from(index + usize::from(self.min_keycode)).ok())
        })
    }
}

/// which modifier bit alt, super and num lock hold in the running keymap
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModifierLayout {
    pub alt: u16,
    pub super_key: u16,
    pub num_lock: u16,
}

impl ModifierLayout {
    pub const CONVENTIONAL: Self = Self {
        alt: 0x0008,
        super_key: 0x0040,
        num_lock: 0x0010,
    };

    /// reads GetModifierMapping, eight rows of per_modifier keycodes with zero for an empty slot
    pub fn from_mapping(per_modifier: usize, keycodes: &[u8], keymap: &KeyboardMap) -> Self {
        let mut found = Self {
            alt: 0,
            super_key: 0,
            num_lock: 0,
        };
        if per_modifier == 0 {
            return Self::CONVENTIONAL;
        }

        let rows = keycodes.chunks(per_modifier).enumerate().take(MODIFIER_ROWS).skip(FIRST_MOD_ROW);
        for (row, codes) in rows {
            let bit = 1u16 << row;
            for keysym in codes.iter().filter(|code| **code != 0).flat_map(|code| keymap.keysyms(*code)) {
                let slot = match *keysym {
                    XK_ALT_L | XK_ALT_R => &mut found.alt,
                    XK_SUPER_L | XK_SUPER_R => &mut found.super_key,
                    XK_NUM_LOCK => &mut found.num_lock,
                    _ => continue,
                };
                if *slot == 0 {
                    *slot = bit;
                }
            }
        }

        Self {
            alt: if found.alt == 0 { Self::CONVENTIONAL.alt } else { found.alt },
            super_key: if found.super_key == 0 {
                Self::CONVENTIONAL.super_key
            } else {
                found.super_key
            },
            num_lock: if found.num_lock == 0 {
                Self::CONVENTIONAL.num_lock
            } else {
                found.num_lock
            },
        }
    }

    /// state bits a press of modifiers carries in this keymap
    pub fn mask(self, modifiers: Modifiers) -> u16 {
        let mut mask = 0;
        if modifiers.command {
            mask |= self.super_key;
        }
        if modifiers.control {
            mask |= CONTROL_MASK;
        }
        if modifiers.alt {
            mask |= self.alt;
        }
        if modifiers.shift {
            mask |= SHIFT_MASK;
        }
        mask
    }

    /// caps lock and num lock, bits a press may hold without meaning anything
    /// invariant: never a bit the chord can use or it would match without its modifier
    pub fn ignored(self) -> u16 {
        let chord_bits = SHIFT_MASK | CONTROL_MASK | self.alt | self.super_key;
        (LOCK_MASK | self.num_lock) & !chord_bits
    }

    /// one GrabKey per combination of the ignored bits, the server matches the state exactly
    pub fn grab_masks(self, base: u16) -> Vec<u16> {
        let ignored = self.ignored();
        let mut masks: Vec<u16> = Vec::with_capacity(4);
        let mut subset: u16 = 0;
        loop {
            let mask = base | subset;
            if !masks.contains(&mask) {
                masks.push(mask);
            }
            if subset == ignored {
                break;
            }
            subset = subset.wrapping_sub(ignored) & ignored;
        }
        masks
    }

    pub fn matches(self, state: u16, base: u16) -> bool {
        state & MODIFIER_BITS & !self.ignored() == base
    }
}

/// auto repeat on x11 arrives as a release and a press with one timestamp, only the first press counts
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RepeatFilter {
    last_release: Option<(u8, u32)>,
}

impl RepeatFilter {
    pub fn released(&mut self, keycode: u8, time: u32) {
        self.last_release = Some((keycode, time));
    }

    /// true for a press the user made, false for one the keyboard repeated
    pub fn pressed(&mut self, keycode: u8, time: u32) -> bool {
        let repeated = self.last_release == Some((keycode, time));
        self.last_release = None;
        !repeated
    }
}

pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const GLOBAL_SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
pub const CREATE_SESSION_METHOD: &str = "CreateSession";
pub const BIND_SHORTCUTS_METHOD: &str = "BindShortcuts";
pub const ACTIVATED_SIGNAL: &str = "Activated";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const RESPONSE_SIGNAL: &str = "Response";
pub const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
pub const CLOSE_METHOD: &str = "Close";
pub const VERSION_PROPERTY: &str = "version";

pub const HANDLE_TOKEN_OPTION: &str = "handle_token";
pub const SESSION_HANDLE_TOKEN_OPTION: &str = "session_handle_token";
pub const DESCRIPTION_OPTION: &str = "description";
pub const PREFERRED_TRIGGER_OPTION: &str = "preferred_trigger";
pub const SESSION_HANDLE_RESULT: &str = "session_handle";

const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";

/// handle token of one request, the spec allows letters and digits and underscores only
pub fn request_token(serial: u64) -> String {
    format!("riseonly_hotkey_{serial}")
}

/// session handle token of one session, same alphabet
pub fn session_token(serial: u64) -> String {
    format!("riseonly_hotkeys_{serial}")
}

/// request object path the portal will use
/// known before the call so the response subscription exists first
/// unique name without its colon, dots as underscores and the token
pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.strip_prefix(':').unwrap_or(unique_name).replace('.', "_");
    format!("{REQUEST_PATH_PREFIX}/{sender}/{token}")
}

/// xkb keysym name the shortcuts spec writes after the modifiers
pub fn xkb_key_name(key: KeyCode) -> &'static str {
    match key {
        KeyCode::A => "a",
        KeyCode::B => "b",
        KeyCode::C => "c",
        KeyCode::D => "d",
        KeyCode::E => "e",
        KeyCode::F => "f",
        KeyCode::G => "g",
        KeyCode::H => "h",
        KeyCode::I => "i",
        KeyCode::J => "j",
        KeyCode::K => "k",
        KeyCode::L => "l",
        KeyCode::M => "m",
        KeyCode::N => "n",
        KeyCode::O => "o",
        KeyCode::P => "p",
        KeyCode::Q => "q",
        KeyCode::R => "r",
        KeyCode::S => "s",
        KeyCode::T => "t",
        KeyCode::U => "u",
        KeyCode::V => "v",
        KeyCode::W => "w",
        KeyCode::X => "x",
        KeyCode::Y => "y",
        KeyCode::Z => "z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Space => "space",
        KeyCode::Return => "Return",
        KeyCode::Escape => "Escape",
        KeyCode::Tab => "Tab",
        KeyCode::Backspace => "BackSpace",
        KeyCode::ForwardDelete => "Delete",
        KeyCode::Left => "Left",
        KeyCode::Right => "Right",
        KeyCode::Up => "Up",
        KeyCode::Down => "Down",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "Page_Up",
        KeyCode::PageDown => "Page_Down",
        KeyCode::Comma => "comma",
        KeyCode::Period => "period",
        KeyCode::Slash => "slash",
        KeyCode::Semicolon => "semicolon",
        KeyCode::Quote => "apostrophe",
        KeyCode::Grave => "grave",
        KeyCode::Minus => "minus",
        KeyCode::Equal => "equal",
        KeyCode::LeftBracket => "bracketleft",
        KeyCode::RightBracket => "bracketright",
        KeyCode::Backslash => "backslash",
    }
}

/// preferred trigger in the shortcuts spec format, modifiers first and eg CTRL+ALT+SHIFT+LOGO+a
pub fn preferred_trigger(hotkey: Hotkey) -> String {
    let modifiers = hotkey.modifiers;
    let names = [
        (modifiers.control, "CTRL"),
        (modifiers.alt, "ALT"),
        (modifiers.shift, "SHIFT"),
        (modifiers.command, "LOGO"),
    ];

    let mut trigger = String::new();
    for (_, name) in names.iter().filter(|(held, _)| *held) {
        trigger.push_str(name);
        trigger.push('+');
    }
    trigger.push_str(xkb_key_name(hotkey.key));
    trigger
}

pub fn keysym_of(hotkey: Hotkey) -> u32 {
    hotkey.native_key_code(HostOs::Linux)
}

/// one chord as bind shortcuts sends it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PortalShortcut {
    pub id: String,
    pub description: String,
    pub preferred_trigger: String,
}

impl PortalShortcut {
    pub fn new(action: &str, hotkey: Hotkey) -> Self {
        Self {
            id: action.to_owned(),
            description: action.to_owned(),
            preferred_trigger: preferred_trigger(hotkey),
        }
    }
}

/// how one portal request ended, from the response code
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortalAnswer {
    Accepted,
    Cancelled,
    Ended(u32),
}

pub fn portal_answer(code: u32) -> PortalAnswer {
    match code {
        0 => PortalAnswer::Accepted,
        1 => PortalAnswer::Cancelled,
        other => PortalAnswer::Ended(other),
    }
}

/// action an activated signal stands for
/// none when it belongs to a session this process already closed or names a shortcut thats
/// no longer wanted
pub fn activated_action(
    bound_session: Option<&str>,
    signal_session: &str,
    shortcut_id: &str,
    wanted: &[PortalShortcut],
) -> Option<String> {
    if bound_session != Some(signal_session) {
        return None;
    }
    wanted.iter().find(|shortcut| shortcut.id == shortcut_id).map(|shortcut| shortcut.id.clone())
}

/// appends a press and drops the oldest once limit presses wait, nobody may be collecting them
pub fn push_bounded(queue: &mut Vec<String>, action: String, limit: usize) {
    if limit == 0 {
        return;
    }
    while queue.len() >= limit {
        queue.remove(0);
    }
    queue.push(action);
}
