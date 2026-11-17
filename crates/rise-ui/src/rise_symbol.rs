/// symbols that only exist on newer systems
/// desktop draws every symbol from one Lucide table that doesnt change with the host os
/// so each name is the newest glyph
pub struct RiseSymbol;

impl RiseSymbol {
    pub const SORT_NEWEST: &'static str = "chevron.up.2";

    pub const SORT_OLDEST: &'static str = "chevron.down.2";

    pub const DEAFENED: &'static str = "headphones.slash";

    pub const ARTIST: &'static str = "music.microphone";

    pub const STORY_CREATE: &'static str = "plus.circle.dashed";
}
