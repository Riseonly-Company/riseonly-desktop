//! Связь с общим ядром riseonly-tools.
//!
//! Тест сторожит не логику крейта — её сторожат golden-вектора в самом riseonly-tools, —
//! а САМУ СВЯЗЬ: что зависимость объявлена, канон доезжает до десктопа и его значения
//! те же, что видят iOS и Android. Пропавшая зависимость или разъехавшийся путь ломают
//! этот файл раньше, чем странное поведение появится на экране.
//!
//! Правило: значения здесь не дублируются. Проверяются инварианты и связность, потому что
//! копия числа рядом с каноном — ровно то, ради чего общее ядро и заводилось.

use rise_perm::{ALL_PERMISSIONS, BIT_COUNT, BITS, ChatPermissions, DEFAULT_EVERYONE, bit_by_name};

#[test]
fn canon_reaches_the_desktop() {
    assert_eq!(BIT_COUNT, BITS.len(), "таблица бит и её длина разошлись");
    assert!(BIT_COUNT > 0, "канон приехал пустым");
}

/// Продуктовое правило: у роли `@everyone` не может быть права упоминать всех.
///
/// Оно записано в каноне и проверяется его валидатором, но здесь стоит второй раз и
/// намеренно: если однажды подключение подменят локальной копией констант, сломается
/// именно этот тест, а не поведение форума у пользователя.
#[test]
fn default_everyone_cannot_mention_everyone() {
    let mention = bit_by_name("mention_everyone").expect("бит `mention_everyone` есть в каноне");
    let everyone = ChatPermissions::from_bits(DEFAULT_EVERYONE);
    assert!(
        !everyone.contains(ChatPermissions::from_bits(1 << mention.position)),
        "DEFAULT_EVERYONE получил право упоминать всех"
    );
}

#[test]
fn every_bit_is_inside_the_full_mask() {
    let all = ChatPermissions::from_bits(ALL_PERMISSIONS);
    for bit in &BITS {
        let single = ChatPermissions::from_bits(1 << bit.position);
        assert!(
            all.contains(single),
            "бит `{}` не входит в ALL_PERMISSIONS",
            bit.name
        );
    }
}

/// Каждая подключённая зависимость обязана быть достижимой из десктопа.
///
/// Иначе `Cargo.toml` обрастает строками, которые компилируются и ничего не значат:
/// зависимость, которую никто не импортирует, не докажет, что канон доехал.
#[test]
fn every_connected_crate_is_reachable() {
    assert!(rise_moderation::ReportReason::ALL.len() > 1, "moderation");
    assert!(!rise_rtc::ROOM_MODE_RULES.is_empty(), "rtc");
    assert!(rise_music::EFFECT_COUNT > 1, "music");
    assert!(rise_limits::LIMIT_COUNT > 1, "limits");
    assert!(rise_events::WS_EVENT_COUNT > 1, "events");
    assert!(
        !rise_catalog::VACANCY_CURRENCIES_DEFAULT.wire().is_empty(),
        "catalog"
    );
    assert!(!rise_chat::MESSAGE_DECODE_KEYS.is_empty(), "chat");
    assert!(rise_endpoints::TIMEOUT_DEFAULT_MS > 0, "endpoints");
    assert!(!rise_regions::FLAG_FALLBACK.is_empty(), "regions");
}

/// Локали доехали в дерево десктопа и остались разбираемыми.
///
/// Файл читается с диска, а не встраивается: сорок один файл на семь мегабайт незачем
/// класть в двоичный файл теста, а проверяется здесь именно доставка.
#[test]
fn locales_arrived_and_parse() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/locales");
    let mut files = 0;
    for entry in std::fs::read_dir(directory).expect("каталог локалей на месте")
    {
        let path = entry.expect("запись каталога читается").path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("локаль читается");
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: не разбирается: {error}", path.display()));
        assert!(
            value.as_object().is_some_and(|map| !map.is_empty()),
            "{}: пустой словарь",
            path.display()
        );
        files += 1;
    }
    assert!(
        files >= 41,
        "локалей доехало {files}, ожидалось не меньше 41"
    );
}
