use std::collections::HashSet;
use std::sync::LazyLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_canon::EmojiSearch;

use crate::modules::auth::stores::auth_actions::selectable_emoji_hooks;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;

pub struct EmojiPickerCatalog;

/// grid identity for one unicode emoji
/// built once per catalog section and reused across renders
/// rebuilding about 1100 of these every render costs more than all the rest of the picker in that frame
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmojiPickerCatalogCell {
    pub id: String,
    pub emoji: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiPickerCatalogCategory {
    pub id: String,
    pub title_key: String,
    pub symbol: String,
    pub emojis: Vec<String>,
    pub cells: Vec<EmojiPickerCatalogCell>,
}

impl EmojiPickerCatalogCategory {
    pub fn new(id: &str, title_key: &str, symbol: &str, emojis: Vec<String>) -> Self {
        let cells = EmojiPickerCatalog::make_cells(&emojis, id);
        Self {
            id: id.to_owned(),
            title_key: title_key.to_owned(),
            symbol: symbol.to_owned(),
            emojis,
            cells,
        }
    }

    pub fn title(&self) -> String {
        tr(&self.title_key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiPickerCatalogQuickFilter {
    pub id: String,
    pub symbol: String,
    pub query: String,
}

const REACTIONS: &str = "❤️ 🤝 👍 🥰 🤯 🔥 👎 👏 😁 🤔 😱 🤬 😔 🎉 🤩 🤮";

const CATEGORIES: [(&str, &str, &str, &str); 9] = [
    (
        "smileys",
        "emoji_smileys",
        "face.smiling",
        "😀 😃 😄 😁 😆 😅 😂 🤣 🥲 ☺️ 😊 😇 🙂 🙃 😉 😌 😍 🥰 😘 😗 😙 😚 😋 😛 😝 😜 🤪 🤨 🧐 🤓 😎 🥸 🤩 🥳 😏 😒 😞 😔 😟 😕 🙁 ☹️ 😣 😖 😫 😩 🥺 😢 😭 😤 😠 😡 🤬 🤯 😳 🥵 🥶 😱 😨 😰 😥 😓 🤗 🤔 🫣 🤭 🫢 🤫 🤥 😶 😐 😑 😬 🫠 🙄 😯 😦 😧 😮 😲 🥱 😴 🤤 😪 😵 🤐 🥴 🤢 🤮 🤧 😷 🤒 🤕 🤑 🤠 😈 👿 🤡 💩 👻 💀 ☠️ 👽 👾 🤖 🎃 😺 😸 😹 😻 😼 😽 🙀 😿 😾",
    ),
    (
        "gestures",
        "emoji_gestures",
        "hand.raised",
        "👋 🤚 🖐 ✋ 🖖 👌 🤌 🤏 ✌️ 🤞 🫰 🤟 🤘 🤙 👈 👉 👆 🖕 👇 ☝️ 🫵 👍 👎 ✊ 👊 🤛 🤜 👏 🙌 🫶 👐 🤲 🤝 🙏 ✍️ 💅 🤳 💪 🦾 🦵 🦶 👂 👃 🧠 👀 👁 👅 👄 🫦 👶 🧒 👦 👧 🧑 👱 👨 🧔 👩 🧓 👴 👵 🙍 🙎 🙅 🙆 💁 🙋 🧏 🙇 🤦 🤷 👮 🕵️ 💂 🥷 👷 🤴 👸 👳 👲 🧕 🤵 👰 🤰 🤱 👼 🎅 🤶 🦸 🦹 🧙 🧚 🧛 🧜 🧝 🧞 🧟 💆 💇 🚶 🧍 🧎 🏃 💃 🕺 🕴 👯 🧖 🧗",
    ),
    (
        "hearts",
        "emoji_hearts",
        "heart",
        "❤️ 🧡 💛 💚 💙 💜 🖤 🤍 🤎 💔 ❤️‍🔥 ❤️‍🩹 ❣️ 💕 💞 💓 💗 💖 💘 💝 💟 💌 💋 💍 💎 💐 🌹 🥀",
    ),
    (
        "animals",
        "emoji_animals",
        "pawprint",
        "🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐻‍❄️ 🐨 🐯 🦁 🐮 🐷 🐽 🐸 🐵 🙈 🙉 🙊 🐒 🐔 🐧 🐦 🐤 🐣 🐥 🦆 🦅 🦉 🦇 🐺 🐗 🐴 🦄 🐝 🪱 🐛 🦋 🐌 🐞 🐜 🦟 🦗 🕷 🦂 🐢 🐍 🦎 🦖 🦕 🐙 🦑 🦐 🦞 🦀 🐡 🐠 🐟 🐬 🐳 🐋 🦈 🐊 🐅 🐆 🦓 🦍 🐘 🦛 🐪 🦒 🦘 🐃 🐂 🐄 🐎 🐖 🐏 🐑 🦙 🐐 🦌 🐕 🐩 🦮 🐈 🐈‍⬛ 🐓 🦃 🦚 🦜 🦢 🦩 🕊 🐇 🦝 🦨 🦦 🦥 🐁 🐀 🦔 🌵 🎄 🌲 🌳 🌴 🌱 🌿 ☘️ 🍀 🪴 🍃 🍂 🍁 🍄 🐚 🌾 🌷 🌺 🌸 🌼 🌻 🌞 🌝 🌛 🌜 🌚 🌕 🌙 ⭐️ 🌟 💫 ✨ ☄️ 🪐 🌍 ⚡️ ☀️ 🌤 ⛅️ ☁️ 🌧 ⛈ 🌨 ❄️ ☃️ ⛄️ 💨 💧 💦 ☔️ 🌊 🌈",
    ),
    (
        "food",
        "emoji_food",
        "fork.knife",
        "🍏 🍎 🍐 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍈 🍒 🍑 🥭 🍍 🥥 🥝 🍅 🍆 🥑 🥦 🥬 🥒 🌶 🫑 🌽 🥕 🫒 🧄 🧅 🥔 🍠 🥐 🥯 🍞 🥖 🥨 🧀 🥚 🍳 🧈 🥞 🧇 🥓 🥩 🍗 🍖 🦴 🌭 🍔 🍟 🍕 🫓 🥪 🥙 🧆 🌮 🌯 🫔 🥗 🥘 🫕 🥫 🍝 🍜 🍲 🍛 🍣 🍱 🥟 🦪 🍤 🍙 🍚 🍘 🍥 🥠 🥮 🍢 🍡 🍧 🍨 🍦 🥧 🧁 🍰 🎂 🍮 🍭 🍬 🍫 🍿 🍩 🍪 🌰 🥜 🍯 🥛 🍼 ☕️ 🫖 🍵 🧃 🥤 🧋 🍶 🍺 🍻 🥂 🍷 🥃 🍸 🍹 🧉 🍾 🧊",
    ),
    (
        "activity",
        "emoji_activity",
        "figure.run",
        "⚽️ 🏀 🏈 ⚾️ 🥎 🎾 🏐 🏉 🥏 🎱 🪀 🏓 🏸 🏒 🏑 🥍 🏏 🥅 ⛳️ 🪁 🏹 🎣 🤿 🥊 🥋 🎽 🛹 🛼 🛷 ⛸ 🥌 🎿 ⛷ 🏂 🪂 🏋️ 🤼 🤸 ⛹️ 🤺 🤾 🏌️ 🏇 🧘 🏄 🏊 🤽 🚣 🧗 🚵 🚴 🏆 🥇 🥈 🥉 🏅 🎖 🎫 🎟 🎪 🤹 🎭 🩰 🎨 🎬 🎤 🎧 🎼 🎹 🥁 🪘 🎷 🎺 🪗 🎸 🪕 🎻 🎲 ♟ 🎯 🎳 🎮 🎰 🧩",
    ),
    (
        "travel",
        "emoji_travel",
        "car",
        "🚗 🚕 🚙 🚌 🚎 🏎 🚓 🚑 🚒 🚐 🛻 🚚 🚛 🚜 🛴 🚲 🛵 🏍 🛺 🚨 🚔 🚍 🚘 🚖 🚡 🚠 🚟 🚃 🚋 🚝 🚄 🚅 🚈 🚂 🚆 🚇 🚊 🚉 ✈️ 🛫 🛬 🛩 💺 🛰 🚀 🛸 🚁 🛶 ⛵️ 🚤 🛥 🛳 ⛴ 🚢 ⚓️ ⛽️ 🚧 🚦 🚥 🚏 🗺 🗿 🗽 🗼 🏰 🏯 🏟 🎡 🎢 🎠 ⛲️ ⛱ 🏖 🏝 🏜 🌋 ⛰ 🏔 🗻 🏕 ⛺️ 🛖 🏠 🏡 🏘 🏚 🏗 🏭 🏢 🏬 🏣 🏥 🏦 🏨 🏪 🏫 🏩 💒 🏛 ⛪️ 🕌 🕍 🛕 🕋 ⛩",
    ),
    (
        "objects",
        "emoji_objects",
        "lightbulb",
        "⌚️ 📱 📲 💻 ⌨️ 🖥 🖨 🖱 🕹 💽 💾 💿 📀 📼 📷 📸 📹 🎥 📽 🎞 📞 ☎️ 📟 📠 📺 📻 🎙 🧭 ⏱ ⏲ ⏰ 🕰 ⌛️ ⏳ 📡 🔋 🪫 🔌 💡 🔦 🕯 🪔 🧯 💸 💵 💴 💶 💷 🪙 💰 💳 ⚖️ 🪜 🧰 🪛 🔧 🔨 ⚒ 🛠 ⛏ 🔩 ⚙️ 🧱 ⛓ 🧲 🔫 💣 🧨 🪓 🔪 🗡 ⚔️ 🛡 🚬 ⚰️ 🪦 ⚱️ 🏺 🔮 📿 🧿 💈 ⚗️ 🔭 🔬 🩹 🩺 💊 💉 🩸 🧬 🦠 🧫 🧪 🌡 🧹 🧺 🧻 🚽 🚰 🚿 🛁 🛀 🧼 🪥 🪒 🧽 🪣 🧴 🛎 🔑 🗝 🚪 🪑 🛋 🛏 🛌 🧸 🪆 🖼 🪞 🪟 🛍 🛒 🎁 🎈 🎏 🎀 🪄 🪅 🎊 🎉 🏮 🎐 ✉️ 📩 📨 📧 💌 📥 📤 📦 🏷 📪 📫 📬 📭 📮 📯 📜 📃 📄 📑 🧾 📊 📈 📉 🗒 🗓 📆 📅 🗑 📇 🗃 🗳 🗄 📋 📁 📂 🗂 🗞 📰 📓 📔 📒 📕 📗 📘 📙 📚 📖 🔖 🧷 🔗 📎 🖇 📐 📏 🧮 📌 📍 ✂️ 🖊 🖋 ✒️ 🖌 🖍 📝 ✏️ 🔍 🔎 🔏 🔐 🔒 🔓",
    ),
    (
        "symbols",
        "emoji_symbols",
        "number",
        "✅ ❌ ❓ ❗️ ‼️ ⁉️ 💯 🔞 🚭 🚫 ⛔️ 🛑 💢 ♨️ 💤 🆗 🆒 🆕 🆓 🆙 ➕ ➖ ➗ ✖️ ♾ 💲 💱 ™️ ©️ ®️ 〰️ ➰ ➿ 🔚 🔙 🔛 🔝 🔜 ✔️ ☑️ 🔘 🔴 🟠 🟡 🟢 🔵 🟣 ⚫️ ⚪️ 🟤 🔺 🔻 🔸 🔹 🔶 🔷 🔳 🔲 ▪️ ▫️ 🟥 🟧 🟨 🟩 🟦 🟪 ⬛️ ⬜️ 🟫 🔈 🔇 🔉 🔊 🔔 🔕 📣 📢 💬 💭 🗯 ♠️ ♣️ ♥️ ♦️ 🃏 🎴 🀄️ 🕐 ⚠️ 🚸 🔱 ⚜️ 🔰 ♻️ 🈯️ 💹 ❇️ ✳️ ❎ 🌐 💠 Ⓜ️ 🌀 💤 🏧 🚾 ♿️ 🅿️ 🛗 🈳 🈂️ 🛂 🛃 🛄 🛅 🚹 🚺 🚼 ⚧ 🚻 🚮 🎦 📶 🈁 🔣 ℹ️ 🔤 🔡 🔠 🆖 🆑 🆘 🆔 🅰️ 🅱️ 🆎 🅾️",
    ),
];

const QUICK_FILTER_SYMBOLS: [(&str, &str); 6] = [
    ("love", "heart"),
    ("like", "hand.thumbsup"),
    ("dislike", "hand.thumbsdown"),
    ("party", "party.popper"),
    ("happy", "face.smiling"),
    ("sad", "cloud.rain"),
];

const POPULAR_SECTION_ID: &str = "emoji_popular";

static REACTION_LIST: LazyLock<Vec<String>> = LazyLock::new(|| EmojiPickerCatalog::blob(REACTIONS));

static CATEGORY_LIST: LazyLock<Vec<EmojiPickerCatalogCategory>> = LazyLock::new(|| {
    CATEGORIES
        .iter()
        .map(|(id, title_key, symbol, emojis)| {
            EmojiPickerCatalogCategory::new(id, title_key, symbol, EmojiPickerCatalog::blob(emojis))
        })
        .collect()
});

static REACTION_CELL_LIST: LazyLock<Vec<EmojiPickerCatalogCell>> =
    LazyLock::new(|| EmojiPickerCatalog::make_cells(&REACTION_LIST, POPULAR_SECTION_ID));

static QUICK_FILTER_LIST: LazyLock<Vec<EmojiPickerCatalogQuickFilter>> = LazyLock::new(|| {
    QUICK_FILTER_SYMBOLS
        .iter()
        .map(|(id, symbol)| {
            let Some(canon) = EmojiSearch::QUICK_FILTERS.iter().find(|filter| filter.id == *id) else {
                panic!("Canon emoji quick filter {id} is missing");
            };
            EmojiPickerCatalogQuickFilter {
                id: (*id).to_owned(),
                symbol: (*symbol).to_owned(),
                query: canon.query.to_owned(),
            }
        })
        .collect()
});

impl EmojiPickerCatalog {
    pub fn make_cells(emojis: &[String], section_id: &str) -> Vec<EmojiPickerCatalogCell> {
        emojis
            .iter()
            .enumerate()
            .map(|(index, emoji)| EmojiPickerCatalogCell {
                id: format!("unicode:{section_id}:{index}:{emoji}"),
                emoji: emoji.clone(),
            })
            .collect()
    }

    pub fn reactions() -> &'static [String] {
        &REACTION_LIST
    }

    pub fn categories() -> &'static [EmojiPickerCatalogCategory] {
        &CATEGORY_LIST
    }

    pub fn reaction_cells() -> &'static [EmojiPickerCatalogCell] {
        &REACTION_CELL_LIST
    }

    /// stored icon or reaction as the product allows it
    /// one unicode emoji grapheme or a ce custom emoji ref
    /// decoding with normalized_selectable_emoji alone would rewrite every custom emoji
    pub fn normalized_selectable_emoji_or_custom_ref(value: Option<&str>) -> Option<String> {
        selectable_emoji_hooks::normalized_selectable_emoji_or_custom_ref(value)
    }

    /// builds ce:sticker_item_id the way the server accepts it
    /// it fits in 64 bytes and the id alphabet is what stickers service mints
    /// same bounds as is_single_emoji_or_custom_ref
    pub fn custom_emoji_ref(item_id: &str) -> Option<String> {
        let reference = format!(
            "{}{}",
            MessageReaction::CUSTOM_EMOJI_PREFIX,
            trimming_whitespaces_and_newlines(item_id)
        );
        if Self::is_valid_custom_emoji_ref(&reference) {
            Some(reference)
        } else {
            None
        }
    }

    pub fn is_valid_custom_emoji_ref(value: &str) -> bool {
        selectable_emoji_hooks::is_valid_custom_emoji_ref(value)
    }

    pub fn normalized_selectable_emoji(value: Option<&str>) -> Option<String> {
        selectable_emoji_hooks::normalized_selectable_emoji(value)
    }

    pub fn search(query: &str) -> Vec<String> {
        let lowered = query.to_lowercase();
        let q = trimming_whitespaces_and_newlines(&lowered);
        if q.is_empty() {
            return Vec::new();
        }

        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for entry in EmojiSearch::KEYWORD_INDEX.iter().filter(|entry| entry.keys.contains(q)) {
            for emoji in entry.emojis.iter() {
                if seen.insert((*emoji).to_owned()) {
                    out.push((*emoji).to_owned());
                }
            }
        }
        for category in Self::categories()
            .iter()
            .filter(|category| category.title().to_lowercase().contains(q))
        {
            for emoji in &category.emojis {
                if seen.insert(emoji.clone()) {
                    out.push(emoji.clone());
                }
            }
        }
        out
    }

    pub fn quick_filters() -> &'static [EmojiPickerCatalogQuickFilter] {
        &QUICK_FILTER_LIST
    }

    fn blob(s: &str) -> Vec<String> {
        s.split([' ', '\n'])
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    }
}
