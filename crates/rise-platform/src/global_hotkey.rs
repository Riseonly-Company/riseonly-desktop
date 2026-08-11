//! System-wide hotkeys — chords that reach the app while another application
//! has focus.
//!
//! Almost all of this is a pure model taking `HostOs` as an argument, so all
//! three platforms' tables are asserted from a Mac; only macOS is bound for
//! real, through Carbon's `RegisterEventHotKey` (which needs no Accessibility
//! permission, unlike an `NSEvent` global monitor).
//!
//! Presses are queued and polled, not delivered by callback: the Carbon handler
//! runs inside gpui's run loop, so `take_pressed` is drained from the caller's
//! own loop instead.

use std::marker::PhantomData;

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HotkeyParseError {
    #[error("an empty string is not a hotkey")]
    Empty,
    #[error("{0:?} names modifiers but no key to press")]
    NoKey(String),
    #[error("{0:?} is not a key this build can bind")]
    UnknownKey(String),
    #[error("{0:?} is not a modifier")]
    UnknownModifier(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("{0} has no modifier; a bare system-wide key is stolen from every app")]
    NoModifier(String),
    #[error("{chord} is already claimed by {owner}")]
    Conflict { chord: String, owner: String },
    #[error("{0} already has a hotkey; release it before binding another")]
    AlreadyBound(String),
    #[error("the system refused {chord} (status {code})")]
    Refused { chord: String, code: i32 },
    #[error("the hotkey event handler could not be installed (status {0})")]
    HandlerUnavailable(i32),
}

/// The four modifiers a portable chord may use. `command` is one flag, not
/// three: Command on macOS, the Windows key on Windows, Super on Linux. Left
/// and right variants are absent — Carbon cannot distinguish them at all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Modifiers {
    pub command: bool,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        command: false,
        control: false,
        alt: false,
        shift: false,
    };

    #[must_use]
    pub fn with_command(self) -> Self {
        Self {
            command: true,
            ..self
        }
    }

    #[must_use]
    pub fn with_control(self) -> Self {
        Self {
            control: true,
            ..self
        }
    }

    #[must_use]
    pub fn with_alt(self) -> Self {
        Self { alt: true, ..self }
    }

    #[must_use]
    pub fn with_shift(self) -> Self {
        Self {
            shift: true,
            ..self
        }
    }

    pub fn is_empty(self) -> bool {
        !(self.command || self.control || self.alt || self.shift)
    }

    /// The bitmask the platform's registration call expects. Linux caveat:
    /// `Mod1Mask` is only conventionally Alt and `Mod4Mask` only conventionally
    /// Super, so a real grab must resolve both from `XGetModifierMapping`.
    pub fn mask(self, host: HostOs) -> u32 {
        let (command, control, alt, shift): (u32, u32, u32, u32) = match host {
            // cmdKey, controlKey, optionKey, shiftKey — HIToolbox/Events.h.
            HostOs::MacOs => (1 << 8, 1 << 12, 1 << 11, 1 << 9),
            // MOD_WIN, MOD_CONTROL, MOD_ALT, MOD_SHIFT.
            HostOs::Windows => (0x0008, 0x0002, 0x0001, 0x0004),
            // Mod4Mask, ControlMask, Mod1Mask, ShiftMask.
            HostOs::Linux => (0x0040, 0x0004, 0x0008, 0x0001),
        };

        let mut mask = 0;
        if self.command {
            mask |= command;
        }
        if self.control {
            mask |= control;
        }
        if self.alt {
            mask |= alt;
        }
        if self.shift {
            mask |= shift;
        }
        mask
    }
}

struct KeyRow {
    key: KeyCode,
    token: &'static str,
    mac: &'static str,
    aliases: &'static [&'static str],
    carbon: u32,
    windows: u32,
    x11: u32,
}

impl KeyRow {
    fn answers_to(&self, token: &str, lowered: &str) -> bool {
        self.token.eq_ignore_ascii_case(token) || self.aliases.contains(&lowered)
    }
}

/// Emits the key vocabulary and its per-OS table from one list, in the same
/// order, so a lookup is `KEYS[key as usize]` rather than a search.
#[rustfmt::skip]
macro_rules! key_table {
    ($($variant:ident, $token:literal, $mac:literal, [$($alias:literal),*],
        $carbon:literal, $windows:literal, $x11:literal;)+) => {
        /// The keys a chord may end in. Wide enough for what a chat client
        /// binds and deliberately no wider: every entry costs three
        /// hand-written platform numbers.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum KeyCode {
            $($variant,)+
        }

        const KEYS: &[KeyRow] = &[
            $(
                KeyRow {
                    key: KeyCode::$variant,
                    token: $token,
                    mac: $mac,
                    aliases: &[$($alias,)*],
                    carbon: $carbon,
                    windows: $windows,
                    x11: $x11,
                },
            )+
        ];
    };
}

// Columns: variant, token, macOS glyph, extra spellings, Carbon kVK_*, Windows
// VK_*, X11 keysym (XK_*, a keysym and not a keycode — see `native_code`).
#[rustfmt::skip]
key_table! {
    A,             "A",         "A",        [],                             0x00, 0x41, 0x0061;
    B,             "B",         "B",        [],                             0x0B, 0x42, 0x0062;
    C,             "C",         "C",        [],                             0x08, 0x43, 0x0063;
    D,             "D",         "D",        [],                             0x02, 0x44, 0x0064;
    E,             "E",         "E",        [],                             0x0E, 0x45, 0x0065;
    F,             "F",         "F",        [],                             0x03, 0x46, 0x0066;
    G,             "G",         "G",        [],                             0x05, 0x47, 0x0067;
    H,             "H",         "H",        [],                             0x04, 0x48, 0x0068;
    I,             "I",         "I",        [],                             0x22, 0x49, 0x0069;
    J,             "J",         "J",        [],                             0x26, 0x4A, 0x006A;
    K,             "K",         "K",        [],                             0x28, 0x4B, 0x006B;
    L,             "L",         "L",        [],                             0x25, 0x4C, 0x006C;
    M,             "M",         "M",        [],                             0x2E, 0x4D, 0x006D;
    N,             "N",         "N",        [],                             0x2D, 0x4E, 0x006E;
    O,             "O",         "O",        [],                             0x1F, 0x4F, 0x006F;
    P,             "P",         "P",        [],                             0x23, 0x50, 0x0070;
    Q,             "Q",         "Q",        [],                             0x0C, 0x51, 0x0071;
    R,             "R",         "R",        [],                             0x0F, 0x52, 0x0072;
    S,             "S",         "S",        [],                             0x01, 0x53, 0x0073;
    T,             "T",         "T",        [],                             0x11, 0x54, 0x0074;
    U,             "U",         "U",        [],                             0x20, 0x55, 0x0075;
    V,             "V",         "V",        [],                             0x09, 0x56, 0x0076;
    W,             "W",         "W",        [],                             0x0D, 0x57, 0x0077;
    X,             "X",         "X",        [],                             0x07, 0x58, 0x0078;
    Y,             "Y",         "Y",        [],                             0x10, 0x59, 0x0079;
    Z,             "Z",         "Z",        [],                             0x06, 0x5A, 0x007A;
    Digit0,        "0",         "0",        [],                             0x1D, 0x30, 0x0030;
    Digit1,        "1",         "1",        [],                             0x12, 0x31, 0x0031;
    Digit2,        "2",         "2",        [],                             0x13, 0x32, 0x0032;
    Digit3,        "3",         "3",        [],                             0x14, 0x33, 0x0033;
    Digit4,        "4",         "4",        [],                             0x15, 0x34, 0x0034;
    Digit5,        "5",         "5",        [],                             0x17, 0x35, 0x0035;
    Digit6,        "6",         "6",        [],                             0x16, 0x36, 0x0036;
    Digit7,        "7",         "7",        [],                             0x1A, 0x37, 0x0037;
    Digit8,        "8",         "8",        [],                             0x1C, 0x38, 0x0038;
    Digit9,        "9",         "9",        [],                             0x19, 0x39, 0x0039;
    F1,            "F1",        "F1",       [],                             0x7A, 0x70, 0xFFBE;
    F2,            "F2",        "F2",       [],                             0x78, 0x71, 0xFFBF;
    F3,            "F3",        "F3",       [],                             0x63, 0x72, 0xFFC0;
    F4,            "F4",        "F4",       [],                             0x76, 0x73, 0xFFC1;
    F5,            "F5",        "F5",       [],                             0x60, 0x74, 0xFFC2;
    F6,            "F6",        "F6",       [],                             0x61, 0x75, 0xFFC3;
    F7,            "F7",        "F7",       [],                             0x62, 0x76, 0xFFC4;
    F8,            "F8",        "F8",       [],                             0x64, 0x77, 0xFFC5;
    F9,            "F9",        "F9",       [],                             0x65, 0x78, 0xFFC6;
    F10,           "F10",       "F10",      [],                             0x6D, 0x79, 0xFFC7;
    F11,           "F11",       "F11",      [],                             0x67, 0x7A, 0xFFC8;
    F12,           "F12",       "F12",      [],                             0x6F, 0x7B, 0xFFC9;
    Space,         "Space",     "Space",    ["spacebar"],                   0x31, 0x20, 0x0020;
    Return,        "Return",    "\u{21A9}", ["enter", "\u{21A9}"],          0x24, 0x0D, 0xFF0D;
    Escape,        "Escape",    "\u{238B}", ["esc", "\u{238B}"],            0x35, 0x1B, 0xFF1B;
    Tab,           "Tab",       "\u{21E5}", ["\u{21E5}"],                   0x30, 0x09, 0xFF09;
    Backspace,     "Backspace", "\u{232B}", ["\u{232B}"],                   0x33, 0x08, 0xFF08;
    ForwardDelete, "Delete",    "\u{2326}", ["del", "\u{2326}"],            0x75, 0x2E, 0xFFFF;
    Left,          "Left",      "\u{2190}", ["leftarrow", "\u{2190}"],      0x7B, 0x25, 0xFF51;
    Right,         "Right",     "\u{2192}", ["rightarrow", "\u{2192}"],     0x7C, 0x27, 0xFF53;
    Up,            "Up",        "\u{2191}", ["uparrow", "\u{2191}"],        0x7E, 0x26, 0xFF52;
    Down,          "Down",      "\u{2193}", ["downarrow", "\u{2193}"],      0x7D, 0x28, 0xFF54;
    Home,          "Home",      "\u{2196}", ["\u{2196}"],                   0x73, 0x24, 0xFF50;
    End,           "End",       "\u{2198}", ["\u{2198}"],                   0x77, 0x23, 0xFF57;
    PageUp,        "PageUp",    "\u{21DE}", ["pgup", "\u{21DE}"],           0x74, 0x21, 0xFF55;
    PageDown,      "PageDown",  "\u{21DF}", ["pgdn", "\u{21DF}"],           0x79, 0x22, 0xFF56;
    Comma,         ",",         ",",        ["comma"],                      0x2B, 0xBC, 0x002C;
    Period,        ".",         ".",        ["period", "dot"],              0x2F, 0xBE, 0x002E;
    Slash,         "/",         "/",        ["slash"],                      0x2C, 0xBF, 0x002F;
    Semicolon,     ";",         ";",        ["semicolon"],                  0x29, 0xBA, 0x003B;
    Quote,         "'",         "'",        ["quote", "apostrophe"],        0x27, 0xDE, 0x0027;
    Grave,         "`",         "`",        ["grave", "backtick"],          0x32, 0xC0, 0x0060;
    Minus,         "-",         "-",        ["minus"],                      0x1B, 0xBD, 0x002D;
    Equal,         "=",         "=",        ["equal", "equals"],            0x18, 0xBB, 0x003D;
    LeftBracket,   "[",         "[",        ["leftbracket"],                0x21, 0xDB, 0x005B;
    RightBracket,  "]",         "]",        ["rightbracket"],               0x1E, 0xDD, 0x005D;
    Backslash,     "\\",        "\\",       ["backslash"],                  0x2A, 0xDC, 0x005C;
}

impl KeyCode {
    pub fn all() -> impl Iterator<Item = Self> {
        KEYS.iter().map(|row| row.key)
    }

    /// Every spelling is matched without regard to case.
    pub fn from_token(token: &str) -> Option<Self> {
        let lowered = token.to_lowercase();
        let row = KEYS.iter().find(|row| row.answers_to(token, &lowered))?;
        Some(row.key)
    }

    /// The number the platform's registration call wants. The macOS number is a
    /// *positional* virtual key code, so a Carbon hotkey binds to a position on
    /// the keyboard rather than to a letter. The Linux number is a *keysym*,
    /// which `XGrabKey` accepts only after `XKeysymToKeycode`.
    pub fn native_code(self, host: HostOs) -> u32 {
        let row = self.row();
        match host {
            HostOs::MacOs => row.carbon,
            HostOs::Windows => row.windows,
            HostOs::Linux => row.x11,
        }
    }

    fn row(self) -> &'static KeyRow {
        &KEYS[self as usize]
    }
}

#[derive(Clone, Copy)]
enum ModifierFlag {
    Command,
    Control,
    Alt,
    Shift,
}

impl ModifierFlag {
    fn set(self, modifiers: &mut Modifiers) {
        match self {
            Self::Command => modifiers.command = true,
            Self::Control => modifiers.control = true,
            Self::Alt => modifiers.alt = true,
            Self::Shift => modifiers.shift = true,
        }
    }
}

fn single_char(token: &str) -> Option<char> {
    let mut characters = token.chars();
    let first = characters.next()?;
    characters.next().is_none().then_some(first)
}

fn modifier_from_glyph(glyph: char) -> Option<ModifierFlag> {
    match glyph {
        '\u{2318}' => Some(ModifierFlag::Command),
        '\u{2303}' => Some(ModifierFlag::Control),
        '\u{2325}' => Some(ModifierFlag::Alt),
        '\u{21E7}' => Some(ModifierFlag::Shift),
        _ => None,
    }
}

fn modifier_from_token(token: &str) -> Option<ModifierFlag> {
    match token.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "super" | "win" | "windows" | "meta" => Some(ModifierFlag::Command),
        "ctrl" | "control" => Some(ModifierFlag::Control),
        "alt" | "opt" | "option" => Some(ModifierFlag::Alt),
        "shift" => Some(ModifierFlag::Shift),
        _ => single_char(token).and_then(modifier_from_glyph),
    }
}

fn is_separator(character: char) -> bool {
    character == '+' || character.is_whitespace()
}

/// One chord: a set of modifiers plus the key that completes it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: KeyCode,
}

impl Hotkey {
    pub fn new(modifiers: Modifiers, key: KeyCode) -> Self {
        Self { modifiers, key }
    }

    /// Accepts what people actually type — `Cmd+Shift+R`, `cmd shift r`,
    /// `Super+M` — and the macOS glyph form, so anything `display` produces
    /// reads back. `-` is not a separator, or `Ctrl+-` could not be named.
    pub fn parse(raw: &str) -> Result<Self, HotkeyParseError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(HotkeyParseError::Empty);
        }

        let mut modifiers = Modifiers::NONE;
        let mut rest = raw;

        // The macOS form has no separators, so glyphs are peeled one at a time.
        while let Some(glyph) = rest.chars().next() {
            match modifier_from_glyph(glyph) {
                Some(flag) => flag.set(&mut modifiers),
                None => break,
            }
            rest = &rest[glyph.len_utf8()..];
        }

        let tokens: Vec<&str> = rest
            .split(is_separator)
            .filter(|token| !token.is_empty())
            .collect();

        let Some((key_token, modifier_tokens)) = tokens.split_last() else {
            return Err(HotkeyParseError::NoKey(raw.to_owned()));
        };

        for token in modifier_tokens {
            match modifier_from_token(token) {
                Some(flag) => flag.set(&mut modifiers),
                None => return Err(HotkeyParseError::UnknownModifier((*token).to_owned())),
            }
        }

        if modifier_from_token(key_token).is_some() {
            return Err(HotkeyParseError::NoKey(raw.to_owned()));
        }

        match KeyCode::from_token(key_token) {
            Some(key) => Ok(Self { modifiers, key }),
            None => Err(HotkeyParseError::UnknownKey((*key_token).to_owned())),
        }
    }

    /// How this chord is written for a human on `host`: macOS renders glyphs
    /// with no separator, in menu-bar order (⌃⌥⇧⌘); Windows and Linux spell
    /// words joined with `+` and differ only in Win versus Super.
    pub fn display(self, host: HostOs) -> String {
        let meta = match host {
            HostOs::MacOs => return self.macos_display(),
            HostOs::Windows => "Win",
            HostOs::Linux => "Super",
        };

        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers.command {
            parts.push(meta);
        }
        if self.modifiers.control {
            parts.push("Ctrl");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        parts.push(self.key.row().token);
        parts.join("+")
    }

    /// The spelling that goes into a settings file — the Linux rendering, never
    /// the macOS glyph form, because settings follow the account. Pinned by a
    /// test so it cannot move out from under files already written.
    pub fn canonical(self) -> String {
        self.display(HostOs::Linux)
    }

    pub fn has_modifier(self) -> bool {
        !self.modifiers.is_empty()
    }

    pub fn modifier_mask(self, host: HostOs) -> u32 {
        self.modifiers.mask(host)
    }

    pub fn native_key_code(self, host: HostOs) -> u32 {
        self.key.native_code(host)
    }

    fn macos_display(self) -> String {
        let mut out = String::new();
        if self.modifiers.control {
            out.push('\u{2303}');
        }
        if self.modifiers.alt {
            out.push('\u{2325}');
        }
        if self.modifiers.shift {
            out.push('\u{21E7}');
        }
        if self.modifiers.command {
            out.push('\u{2318}');
        }
        out.push_str(self.key.row().mac);
        out
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

fn entry_view(entry: &(String, Hotkey)) -> (&str, Hotkey) {
    (entry.0.as_str(), entry.1)
}

/// Which chord belongs to which feature, and the rules about that.
///
/// Pure — it never touches the OS, so a preferences screen can validate a chord
/// as it is typed. A chord with no modifier is refused (registering a bare
/// letter system-wide takes it from every other application), and two features
/// may not claim one chord (the second registration silently never fires).
#[derive(Clone, Default, Debug)]
pub struct HotkeySet {
    bindings: Vec<(String, Hotkey)>,
}

impl HotkeySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, action: impl Into<String>, hotkey: Hotkey) -> Result<(), HotkeyError> {
        let action = action.into();

        if !hotkey.has_modifier() {
            return Err(HotkeyError::NoModifier(hotkey.display(HostOs::current())));
        }

        if self.position_of(&action).is_some() {
            return Err(HotkeyError::AlreadyBound(action));
        }

        if let Some(owner) = self.action_for(hotkey) {
            return Err(HotkeyError::Conflict {
                chord: hotkey.display(HostOs::current()),
                owner: owner.to_owned(),
            });
        }

        self.bindings.push((action, hotkey));
        Ok(())
    }

    pub fn unbind(&mut self, action: &str) -> Option<Hotkey> {
        let index = self.position_of(action)?;
        Some(self.bindings.remove(index).1)
    }

    pub fn hotkey_for(&self, action: &str) -> Option<Hotkey> {
        let index = self.position_of(action)?;
        Some(self.bindings[index].1)
    }

    pub fn action_for(&self, hotkey: Hotkey) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(_, existing)| *existing == hotkey)
            .map(|(action, _)| action.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Hotkey)> {
        self.bindings.iter().map(entry_view)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    fn position_of(&self, action: &str) -> Option<usize> {
        self.bindings.iter().position(|(name, _)| name == action)
    }
}

/// An exported but empty variable is not a running display server.
fn is_present(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.is_empty())
}

/// Which display server a Linux session is running, as far as the environment
/// admits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

impl DisplayServer {
    pub fn detect() -> Self {
        let wayland = std::env::var("WAYLAND_DISPLAY").ok();
        let session = std::env::var("XDG_SESSION_TYPE").ok();
        let x11 = std::env::var("DISPLAY").ok();
        Self::from_environment(wayland.as_deref(), session.as_deref(), x11.as_deref())
    }

    /// `WAYLAND_DISPLAY` outranks `DISPLAY`: under XWayland both are set, and a
    /// grab taken through XWayland never sees a key pressed in a native Wayland
    /// client.
    pub fn from_environment(
        wayland_display: Option<&str>,
        session_type: Option<&str>,
        x11_display: Option<&str>,
    ) -> Self {
        let session = session_type.unwrap_or_default();

        if is_present(wayland_display) || session.eq_ignore_ascii_case("wayland") {
            Self::Wayland
        } else if is_present(x11_display) || session.eq_ignore_ascii_case("x11") {
            Self::X11
        } else {
            Self::Unknown
        }
    }
}

/// Why a platform cannot give us a chord that fires while we are in the
/// background.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnavailableReason {
    /// `RegisterHotKey` delivers `WM_HOTKEY` to a thread's message queue, and
    /// gpui's Windows backend owns the message loop with no hook for a foreign
    /// message — registering without one produces a grab nothing ever reads.
    NoMessageLoop,
    /// X11's `XGrabKey` on the root window would be the implementation; nothing
    /// in this build can reach an X server.
    NoX11Binding,
    /// Wayland has no global shortcut protocol by design — only the compositor
    /// may see keys the focused client did not receive. The one route is the
    /// `org.freedesktop.portal.GlobalShortcuts` portal over D-Bus.
    WaylandHasNoProtocol,
    /// Neither `WAYLAND_DISPLAY` nor `DISPLAY` is set.
    NoDisplayServer,
}

impl std::fmt::Display for UnavailableReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NoMessageLoop => "RegisterHotKey needs a message loop this build does not own",
            Self::NoX11Binding => "no X11 binding is available to call XGrabKey",
            Self::WaylandHasNoProtocol => "Wayland has no global shortcut protocol",
            Self::NoDisplayServer => "no display server is running",
        };
        formatter.write_str(text)
    }
}

/// What a chord will actually do on a given system.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HotkeyAvailability {
    /// The chord reaches us while another application has focus.
    SystemWide,
    /// The chord reaches us only while one of our own windows has focus, so it
    /// must be presented as an in-app shortcut and promise no more.
    WindowOnly(UnavailableReason),
}

impl HotkeyAvailability {
    pub fn for_host(host: HostOs, display_server: DisplayServer) -> Self {
        let reason = match (host, display_server) {
            (HostOs::MacOs, _) => return Self::SystemWide,
            (HostOs::Windows, _) => UnavailableReason::NoMessageLoop,
            (HostOs::Linux, DisplayServer::X11) => UnavailableReason::NoX11Binding,
            (HostOs::Linux, DisplayServer::Wayland) => UnavailableReason::WaylandHasNoProtocol,
            (HostOs::Linux, DisplayServer::Unknown) => UnavailableReason::NoDisplayServer,
        };

        Self::WindowOnly(reason)
    }

    pub fn current() -> Self {
        Self::for_host(HostOs::current(), DisplayServer::detect())
    }

    pub fn is_system_wide(self) -> bool {
        self == Self::SystemWide
    }
}

/// The app's live global hotkeys. Dropping this releases every chord it holds:
/// a chord left registered stays claimed by a process that no longer answers
/// it, and the user's only clue is that the shortcut stopped working.
pub struct GlobalHotkeys {
    bindings: HotkeySet,
    #[cfg(target_os = "macos")]
    live: Vec<macos::Registration>,
    /// Carbon's hotkey calls are not thread safe, so this is `!Send` on every
    /// platform — otherwise code written on this Mac would move it to a worker
    /// and fail to compile only where nobody builds.
    _main_thread_only: PhantomData<*const ()>,
}

impl Default for GlobalHotkeys {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalHotkeys {
    pub fn new() -> Self {
        Self {
            bindings: HotkeySet::new(),
            #[cfg(target_os = "macos")]
            live: Vec::new(),
            _main_thread_only: PhantomData,
        }
    }

    pub fn availability() -> HotkeyAvailability {
        HotkeyAvailability::current()
    }

    /// `Ok(Unsupported)` means the chord was recorded but no OS grab exists, so
    /// it fires only while a window of ours has focus. The binding is kept all
    /// the same, or conflict detection would stop covering it.
    pub fn bind(
        &mut self,
        action: impl Into<String>,
        hotkey: Hotkey,
    ) -> Result<PlatformSupport, HotkeyError> {
        let action = action.into();
        self.bindings.bind(action.clone(), hotkey)?;

        match self.install(&action, hotkey) {
            Ok(support) => Ok(support),
            Err(error) => {
                self.bindings.unbind(&action);
                Err(error)
            }
        }
    }

    /// `Unsupported` means nothing was released — either the action had no
    /// binding, or this platform never took one.
    pub fn unbind(&mut self, action: &str) -> PlatformSupport {
        if self.bindings.unbind(action).is_none() {
            return PlatformSupport::Unsupported;
        }
        self.uninstall(action)
    }

    /// Drains the chords pressed since the last call, oldest first.
    /// Non-blocking: the caller polls it from its own loop.
    pub fn take_pressed(&self) -> Vec<String> {
        #[cfg(target_os = "macos")]
        {
            macos::take_pressed()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }

    pub fn bindings(&self) -> &HotkeySet {
        &self.bindings
    }

    #[cfg(target_os = "macos")]
    fn install(&mut self, action: &str, hotkey: Hotkey) -> Result<PlatformSupport, HotkeyError> {
        self.live.push(macos::register(action, hotkey)?);
        Ok(PlatformSupport::Performed)
    }

    #[cfg(not(target_os = "macos"))]
    fn install(&mut self, action: &str, hotkey: Hotkey) -> Result<PlatformSupport, HotkeyError> {
        let _ = (action, hotkey);
        Ok(PlatformSupport::Unsupported)
    }

    #[cfg(target_os = "macos")]
    fn uninstall(&mut self, action: &str) -> PlatformSupport {
        match self.live.iter().position(|entry| entry.action() == action) {
            Some(index) => {
                // Dropping the Registration is what releases the chord.
                drop(self.live.remove(index));
                PlatformSupport::Performed
            }
            None => PlatformSupport::Unsupported,
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn uninstall(&mut self, action: &str) -> PlatformSupport {
        let _ = action;
        PlatformSupport::Unsupported
    }
}

/// Carbon, declared by hand from `HIToolbox/CarbonEvents.h`: `OSStatus` is
/// `SInt32`, `OSType` is `UInt32`, and `ItemCount` and `ByteCount` are both
/// `unsigned long`, 64-bit on every Mac this ships to — hence `usize`.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU32, Ordering};

    use parking_lot::Mutex;

    use super::{Hotkey, HotkeyError};
    use crate::host_os::HostOs;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {}

    type OsStatus = i32;
    type EventRef = *mut c_void;
    type EventTargetRef = *mut c_void;
    type EventHandlerRef = *mut c_void;
    type EventHandlerCallRef = *mut c_void;
    type EventHotKeyRef = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyId {
        signature: u32,
        id: u32,
    }

    #[repr(C)]
    struct EventTypeSpec {
        event_class: u32,
        event_kind: u32,
    }

    type EventHandlerUpp =
        unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OsStatus;

    unsafe extern "C" {
        #[link_name = "GetApplicationEventTarget"]
        fn get_application_event_target() -> EventTargetRef;

        #[link_name = "InstallEventHandler"]
        fn install_event_handler(
            target: EventTargetRef,
            handler: EventHandlerUpp,
            num_types: usize,
            list: *const EventTypeSpec,
            user_data: *mut c_void,
            out_ref: *mut EventHandlerRef,
        ) -> OsStatus;

        #[link_name = "RegisterEventHotKey"]
        fn register_event_hot_key(
            key_code: u32,
            modifiers: u32,
            hot_key_id: EventHotKeyId,
            target: EventTargetRef,
            options: u32,
            out_ref: *mut EventHotKeyRef,
        ) -> OsStatus;

        #[link_name = "UnregisterEventHotKey"]
        fn unregister_event_hot_key(hot_key: EventHotKeyRef) -> OsStatus;

        #[link_name = "GetEventParameter"]
        fn get_event_parameter(
            event: EventRef,
            name: u32,
            desired_type: u32,
            actual_type: *mut u32,
            buffer_size: usize,
            actual_size: *mut usize,
            data: *mut c_void,
        ) -> OsStatus;
    }

    const fn fourcc(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    const NO_ERR: OsStatus = 0;
    const EVENT_CLASS_KEYBOARD: u32 = fourcc(b"keyb");
    const EVENT_HOT_KEY_PRESSED: u32 = 5;
    const PARAM_DIRECT_OBJECT: u32 = fourcc(b"----");
    const TYPE_EVENT_HOT_KEY_ID: u32 = fourcc(b"hkid");

    /// Stamped into every registration, so another Carbon client's hotkey event
    /// in this process cannot be read as one of ours.
    const SIGNATURE: u32 = fourcc(b"rise");

    /// `kEventHotKeyNoOptions`, not `kEventHotKeyExclusive`: macOS notifies all
    /// registrants, so a successful registration does not prove the chord is
    /// ours alone — but exclusivity would deny it to every other application.
    const NO_OPTIONS: u32 = 0;

    /// A press nobody collects must not accumulate forever.
    const MAX_PENDING: usize = 64;

    static SLOTS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
    static PRESSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static NEXT_SLOT: AtomicU32 = AtomicU32::new(1);
    static HANDLER: OnceLock<OsStatus> = OnceLock::new();

    unsafe extern "C" fn hot_key_pressed(
        _call: EventHandlerCallRef,
        event: EventRef,
        _user_data: *mut c_void,
    ) -> OsStatus {
        let mut id = EventHotKeyId {
            signature: 0,
            id: 0,
        };

        // SAFETY: `event` is the event Carbon is dispatching to this handler,
        // and the out buffer is exactly one EventHotKeyID, the documented type
        // of kEventParamDirectObject on this event kind.
        let status = unsafe {
            get_event_parameter(
                event,
                PARAM_DIRECT_OBJECT,
                TYPE_EVENT_HOT_KEY_ID,
                std::ptr::null_mut(),
                size_of::<EventHotKeyId>(),
                std::ptr::null_mut(),
                (&raw mut id).cast(),
            )
        };

        if status == NO_ERR && id.signature == SIGNATURE {
            let action = SLOTS
                .lock()
                .iter()
                .find(|(slot, _)| *slot == id.id)
                .map(|(_, action)| action.clone());

            if let Some(action) = action {
                let mut pressed = PRESSED.lock();
                if pressed.len() >= MAX_PENDING {
                    pressed.remove(0);
                }
                pressed.push(action);
            }
        }

        NO_ERR
    }

    fn ensure_handler() -> OsStatus {
        *HANDLER.get_or_init(|| {
            let spec = EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOT_KEY_PRESSED,
            };

            // SAFETY: one EventTypeSpec matches the count of 1, and Carbon
            // copies the list during the call. No user data is passed: the
            // callback reaches its state through this module's statics.
            unsafe {
                install_event_handler(
                    get_application_event_target(),
                    hot_key_pressed,
                    1,
                    &raw const spec,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            }
        })
    }

    pub(super) struct Registration {
        action: String,
        slot: u32,
        handle: EventHotKeyRef,
    }

    impl Registration {
        pub(super) fn action(&self) -> &str {
            &self.action
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            // SAFETY: `handle` came from a register_event_hot_key that returned
            // noErr, and this is the only place it is released — Registration is
            // neither Clone nor Copy.
            unsafe {
                unregister_event_hot_key(self.handle);
            }
            SLOTS.lock().retain(|(slot, _)| *slot != self.slot);
        }
    }

    pub(super) fn register(action: &str, hotkey: Hotkey) -> Result<Registration, HotkeyError> {
        let status = ensure_handler();
        if status != NO_ERR {
            return Err(HotkeyError::HandlerUnavailable(status));
        }

        // Slots are never reused: an event in flight when a chord is released
        // cannot be delivered to whatever takes that chord's place.
        let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
        SLOTS.lock().push((slot, action.to_owned()));

        let mut handle: EventHotKeyRef = std::ptr::null_mut();

        // SAFETY: the key code and mask come from this module's tables, and
        // `handle` is a live local Carbon writes exactly one pointer into.
        let status = unsafe {
            register_event_hot_key(
                hotkey.native_key_code(HostOs::MacOs),
                hotkey.modifier_mask(HostOs::MacOs),
                EventHotKeyId {
                    signature: SIGNATURE,
                    id: slot,
                },
                get_application_event_target(),
                NO_OPTIONS,
                &raw mut handle,
            )
        };

        if status != NO_ERR || handle.is_null() {
            SLOTS.lock().retain(|(existing, _)| *existing != slot);
            return Err(HotkeyError::Refused {
                chord: hotkey.display(HostOs::MacOs),
                code: status,
            });
        }

        Ok(Registration {
            action: action.to_owned(),
            slot,
            handle,
        })
    }

    pub(super) fn take_pressed() -> Vec<String> {
        std::mem::take(&mut *PRESSED.lock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // Nothing here registers a real chord: it would claim a system-wide
    // combination on the machine running the suite.

    fn chord(raw: &str) -> Hotkey {
        match Hotkey::parse(raw) {
            Ok(hotkey) => hotkey,
            Err(error) => panic!("{raw:?} should parse: {error}"),
        }
    }

    fn all_modifier_sets() -> Vec<Modifiers> {
        (0..16u8)
            .map(|bits| Modifiers {
                command: bits & 1 != 0,
                control: bits & 2 != 0,
                alt: bits & 4 != 0,
                shift: bits & 8 != 0,
            })
            .collect()
    }

    fn spellings_of(row: &KeyRow) -> Vec<&'static str> {
        let mut spellings = vec![row.token, row.mac];
        spellings.extend_from_slice(row.aliases);
        spellings
    }

    #[test]
    fn the_spellings_people_actually_type_all_parse() {
        assert_eq!(
            chord("Cmd+Shift+R"),
            Hotkey::new(Modifiers::NONE.with_command().with_shift(), KeyCode::R)
        );
        assert_eq!(chord("cmd shift r"), chord("Cmd+Shift+R"));
        assert_eq!(
            chord("Ctrl+Alt+K"),
            Hotkey::new(Modifiers::NONE.with_control().with_alt(), KeyCode::K)
        );
        assert_eq!(
            chord("Super+M"),
            Hotkey::new(Modifiers::NONE.with_command(), KeyCode::M)
        );
    }

    #[test]
    fn one_gesture_answers_to_every_platforms_name_for_it() {
        let meta_spellings = [
            "Cmd+M",
            "Command+M",
            "Super+M",
            "Win+M",
            "Windows+M",
            "meta m",
        ];

        for spelling in meta_spellings {
            assert_eq!(
                chord(spelling),
                Hotkey::new(Modifiers::NONE.with_command(), KeyCode::M),
                "{spelling} should name the meta key"
            );
        }

        for spelling in ["Alt+M", "Opt+M", "Option+M"] {
            assert_eq!(
                chord(spelling),
                Hotkey::new(Modifiers::NONE.with_alt(), KeyCode::M),
                "{spelling} should name the alt key"
            );
        }
    }

    #[test]
    fn extra_whitespace_and_case_do_not_change_the_meaning() {
        assert_eq!(chord("  CTRL + shift + f5  "), chord("Ctrl+Shift+F5"));
    }

    #[test]
    fn every_chord_round_trips_through_every_platforms_display_form() {
        for key in KeyCode::all() {
            for modifiers in all_modifier_sets() {
                let hotkey = Hotkey::new(modifiers, key);
                for host in HostOs::ALL {
                    let rendered = hotkey.display(host);
                    assert_eq!(
                        Hotkey::parse(&rendered),
                        Ok(hotkey),
                        "{rendered:?} on {host:?} did not read back as {hotkey:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn macos_renders_glyphs_in_menu_bar_order_with_no_separator() {
        let everything = Modifiers {
            command: true,
            control: true,
            alt: true,
            shift: true,
        };

        assert_eq!(
            Hotkey::new(everything, KeyCode::K).display(HostOs::MacOs),
            "\u{2303}\u{2325}\u{21E7}\u{2318}K"
        );
        assert_eq!(
            chord("Cmd+Shift+R").display(HostOs::MacOs),
            "\u{21E7}\u{2318}R"
        );
        assert_eq!(
            chord("Cmd+Return").display(HostOs::MacOs),
            "\u{2318}\u{21A9}"
        );
    }

    #[test]
    fn windows_and_linux_spell_words_and_differ_only_in_the_meta_key() {
        let hotkey = chord("Super+Shift+R");
        assert_eq!(hotkey.display(HostOs::Windows), "Win+Shift+R");
        assert_eq!(hotkey.display(HostOs::Linux), "Super+Shift+R");

        let no_meta = chord("Ctrl+Alt+K");
        assert_eq!(no_meta.display(HostOs::Windows), "Ctrl+Alt+K");
        assert_eq!(
            no_meta.display(HostOs::Windows),
            no_meta.display(HostOs::Linux),
            "without the meta key the two renderings must be identical"
        );
    }

    #[test]
    fn the_canonical_spelling_is_pinned_because_settings_files_hold_it() {
        assert_eq!(chord("Cmd+Shift+R").canonical(), "Super+Shift+R");
        assert_eq!(chord("Ctrl+Alt+K").canonical(), "Ctrl+Alt+K");
        assert_eq!(
            chord("Cmd+Shift+PageDown").canonical(),
            "Super+Shift+PageDown"
        );
        assert_eq!(chord("Ctrl+,").canonical(), "Ctrl+,");
        assert_eq!(chord("Cmd+Shift+R").to_string(), "Super+Shift+R");
    }

    #[test]
    fn the_canonical_spelling_never_carries_a_mac_glyph() {
        for key in KeyCode::all() {
            let canonical = Hotkey::new(Modifiers::NONE.with_command(), key).canonical();
            assert!(
                canonical.is_ascii(),
                "{canonical:?} cannot travel to a Linux install in a settings file"
            );
        }
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed() {
        // "R+Cmd" is refused because the key comes last: anything before it has
        // to be a modifier.
        let cases = [
            ("", HotkeyParseError::Empty),
            ("   ", HotkeyParseError::Empty),
            ("Cmd+Shift", HotkeyParseError::NoKey("Cmd+Shift".into())),
            ("Cmd+Nope", HotkeyParseError::UnknownKey("Nope".into())),
            (
                "Frobnicate+R",
                HotkeyParseError::UnknownModifier("Frobnicate".into()),
            ),
            ("R+Cmd", HotkeyParseError::UnknownModifier("R".into())),
        ];

        for (raw, expected) in cases {
            assert_eq!(Hotkey::parse(raw), Err(expected), "{raw:?}");
        }
    }

    #[test]
    fn a_string_of_bare_modifier_glyphs_is_not_a_chord() {
        let raw = "\u{2318}\u{21E7}";
        assert_eq!(Hotkey::parse(raw), Err(HotkeyParseError::NoKey(raw.into())));
    }

    #[test]
    fn the_minus_key_is_still_expressible_because_dash_is_not_a_separator() {
        assert_eq!(
            chord("Ctrl+-"),
            Hotkey::new(Modifiers::NONE.with_control(), KeyCode::Minus)
        );
        assert_eq!(chord("Cmd+="), chord("Cmd+equals"));
    }

    #[test]
    fn the_carbon_masks_are_the_event_manager_bits() {
        let mask = |modifiers: Modifiers| modifiers.mask(HostOs::MacOs);
        assert_eq!(mask(Modifiers::NONE.with_command()), 0x0100);
        assert_eq!(mask(Modifiers::NONE.with_shift()), 0x0200);
        assert_eq!(mask(Modifiers::NONE.with_alt()), 0x0800);
        assert_eq!(mask(Modifiers::NONE.with_control()), 0x1000);
    }

    #[test]
    fn the_windows_masks_are_the_documented_mod_constants() {
        let mask = |modifiers: Modifiers| modifiers.mask(HostOs::Windows);
        assert_eq!(mask(Modifiers::NONE.with_alt()), 0x0001);
        assert_eq!(mask(Modifiers::NONE.with_control()), 0x0002);
        assert_eq!(mask(Modifiers::NONE.with_shift()), 0x0004);
        assert_eq!(mask(Modifiers::NONE.with_command()), 0x0008);
    }

    #[test]
    fn the_x11_masks_are_the_documented_modifier_masks() {
        let mask = |modifiers: Modifiers| modifiers.mask(HostOs::Linux);
        assert_eq!(mask(Modifiers::NONE.with_shift()), 1);
        assert_eq!(mask(Modifiers::NONE.with_control()), 4);
        assert_eq!(mask(Modifiers::NONE.with_alt()), 8);
        assert_eq!(mask(Modifiers::NONE.with_command()), 64);
    }

    #[test]
    fn an_empty_modifier_set_is_an_empty_mask_on_every_platform() {
        for host in HostOs::ALL {
            assert_eq!(Modifiers::NONE.mask(host), 0, "{host:?}");
        }
    }

    #[test]
    fn no_two_modifier_sets_collapse_onto_one_mask() {
        for host in HostOs::ALL {
            let mut seen: HashMap<u32, Modifiers> = HashMap::new();
            for modifiers in all_modifier_sets() {
                let mask = modifiers.mask(host);
                if let Some(other) = seen.insert(mask, modifiers) {
                    panic!("{host:?}: {modifiers:?} and {other:?} both produce {mask:#x}");
                }
            }
        }
    }

    #[test]
    fn the_key_table_is_indexed_by_the_enum_discriminant() {
        for (index, row) in KEYS.iter().enumerate() {
            assert_eq!(
                row.key as usize, index,
                "{:?} is out of place; native_code would report another key's number",
                row.key
            );
        }
    }

    #[test]
    fn no_two_keys_share_a_native_code_on_any_platform() {
        for host in HostOs::ALL {
            let mut seen: HashMap<u32, KeyCode> = HashMap::new();
            for key in KeyCode::all() {
                let code = key.native_code(host);
                if let Some(other) = seen.insert(code, key) {
                    panic!("{host:?}: {key:?} and {other:?} both map to {code:#x}");
                }
            }
        }
    }

    #[test]
    fn every_spelling_resolves_and_no_two_keys_share_one() {
        let mut seen: HashMap<String, KeyCode> = HashMap::new();

        for row in KEYS {
            for spelling in spellings_of(row) {
                assert_eq!(
                    KeyCode::from_token(spelling),
                    Some(row.key),
                    "{spelling:?} should resolve to {:?}",
                    row.key
                );

                let lowered = spelling.to_lowercase();
                if let Some(other) = seen.insert(lowered, row.key) {
                    assert_eq!(other, row.key, "{spelling:?} names two different keys");
                }
            }
        }
    }

    #[test]
    fn no_key_spelling_is_also_a_modifier_name() {
        for row in KEYS {
            for spelling in spellings_of(row) {
                assert!(
                    modifier_from_token(spelling).is_none(),
                    "{spelling:?} would be eaten as a modifier before reaching {:?}",
                    row.key
                );
            }
        }
    }

    #[test]
    fn a_hotkey_without_a_modifier_is_refused() {
        let mut set = HotkeySet::new();
        let bare = Hotkey::new(Modifiers::NONE, KeyCode::R);

        assert!(matches!(
            set.bind("toggle_window", bare),
            Err(HotkeyError::NoModifier(_))
        ));
        assert!(
            set.is_empty(),
            "a refused chord must not be left in the set"
        );
    }

    #[test]
    fn two_features_cannot_claim_the_same_chord() {
        let mut set = HotkeySet::new();
        set.bind("quick_search", chord("Cmd+Shift+R")).unwrap();

        let second = set.bind("toggle_window", chord("Cmd+Shift+R"));
        let Err(HotkeyError::Conflict { owner, .. }) = second else {
            panic!("a chord already claimed by quick_search must be refused");
        };

        assert_eq!(owner, "quick_search");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn one_feature_cannot_hold_two_chords_by_accident() {
        let mut set = HotkeySet::new();
        set.bind("quick_search", chord("Cmd+Shift+R")).unwrap();

        assert!(matches!(
            set.bind("quick_search", chord("Cmd+Shift+K")),
            Err(HotkeyError::AlreadyBound(_))
        ));
    }

    #[test]
    fn releasing_a_chord_frees_it_for_another_feature() {
        let mut set = HotkeySet::new();
        set.bind("quick_search", chord("Cmd+Shift+R")).unwrap();

        assert_eq!(set.unbind("quick_search"), Some(chord("Cmd+Shift+R")));
        assert_eq!(set.unbind("quick_search"), None);
        set.bind("toggle_window", chord("Cmd+Shift+R")).unwrap();

        assert_eq!(set.action_for(chord("Cmd+Shift+R")), Some("toggle_window"));
    }

    #[test]
    fn an_action_and_its_chord_each_find_the_other() {
        let mut set = HotkeySet::new();
        set.bind("quick_search", chord("Cmd+Shift+R")).unwrap();
        set.bind("toggle_window", chord("Ctrl+Alt+K")).unwrap();

        assert_eq!(set.hotkey_for("toggle_window"), Some(chord("Ctrl+Alt+K")));
        assert_eq!(set.action_for(chord("Cmd+Shift+R")), Some("quick_search"));
        assert_eq!(set.hotkey_for("absent"), None);
        assert_eq!(set.action_for(chord("Cmd+F12")), None);
        assert_eq!(set.iter().count(), 2);
    }

    #[test]
    fn only_macos_promises_a_chord_that_fires_in_the_background() {
        let macos = HotkeyAvailability::for_host(HostOs::MacOs, DisplayServer::Unknown);
        assert!(macos.is_system_wide());

        for display_server in [
            DisplayServer::Wayland,
            DisplayServer::X11,
            DisplayServer::Unknown,
        ] {
            assert!(
                !HotkeyAvailability::for_host(HostOs::Windows, display_server).is_system_wide(),
                "the Windows arm has no message loop and must not claim otherwise"
            );
            assert!(
                !HotkeyAvailability::for_host(HostOs::Linux, display_server).is_system_wide(),
                "no Linux display server is implemented here"
            );
        }
    }

    #[test]
    fn each_platform_names_the_reason_it_cannot_do_this() {
        assert_eq!(
            HotkeyAvailability::for_host(HostOs::Windows, DisplayServer::Unknown),
            HotkeyAvailability::WindowOnly(UnavailableReason::NoMessageLoop)
        );
        assert_eq!(
            HotkeyAvailability::for_host(HostOs::Linux, DisplayServer::X11),
            HotkeyAvailability::WindowOnly(UnavailableReason::NoX11Binding)
        );
        assert_eq!(
            HotkeyAvailability::for_host(HostOs::Linux, DisplayServer::Wayland),
            HotkeyAvailability::WindowOnly(UnavailableReason::WaylandHasNoProtocol)
        );
        assert_eq!(
            HotkeyAvailability::for_host(HostOs::Linux, DisplayServer::Unknown),
            HotkeyAvailability::WindowOnly(UnavailableReason::NoDisplayServer)
        );
    }

    #[test]
    fn an_xwayland_session_is_reported_as_wayland_not_x11() {
        assert_eq!(
            DisplayServer::from_environment(Some("wayland-0"), Some("wayland"), Some(":0")),
            DisplayServer::Wayland,
            "a grab through XWayland never sees a native Wayland client"
        );
    }

    #[test]
    fn a_plain_x11_session_is_recognised() {
        let cases = [
            (None, Some("x11"), Some(":0")),
            (None, None, Some(":0")),
            (None, Some("X11"), None),
        ];

        for (wayland, session, x11) in cases {
            assert_eq!(
                DisplayServer::from_environment(wayland, session, x11),
                DisplayServer::X11,
                "{session:?} with {x11:?}"
            );
        }
    }

    #[test]
    fn an_empty_variable_is_not_a_display_server() {
        assert_eq!(
            DisplayServer::from_environment(Some(""), None, Some("")),
            DisplayServer::Unknown,
            "an exported but empty DISPLAY must not look like a running X server"
        );
        assert_eq!(
            DisplayServer::from_environment(None, None, None),
            DisplayServer::Unknown
        );
    }

    #[test]
    fn a_bare_key_is_refused_before_the_os_is_ever_asked() {
        let mut hotkeys = GlobalHotkeys::new();
        let bare = Hotkey::new(Modifiers::NONE, KeyCode::R);

        assert!(matches!(
            hotkeys.bind("toggle_window", bare),
            Err(HotkeyError::NoModifier(_))
        ));
        assert!(hotkeys.bindings().is_empty());
    }

    #[test]
    fn a_fresh_registry_holds_nothing_and_has_pressed_nothing() {
        let hotkeys = GlobalHotkeys::new();
        assert!(hotkeys.bindings().is_empty());
        assert!(hotkeys.take_pressed().is_empty());
    }

    #[test]
    fn releasing_an_action_that_was_never_bound_reports_that_it_did_nothing() {
        let mut hotkeys = GlobalHotkeys::new();
        assert_eq!(
            hotkeys.unbind("never_bound"),
            PlatformSupport::Unsupported,
            "a caller must be able to tell that nothing was released"
        );
    }
}
