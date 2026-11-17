use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_regex::{NSMatchingOptions, NSRegularExpression};
use rise_core::foundation::foundation_text::{is_newline, trimming_whitespaces_and_newlines};
use rise_core::foundation::ns_cache::NSCache;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::Locale;
use rise_regions::{
    ASSET_FIELD_COUNT, ASSET_FIELD_SEPARATOR, ASSET_MAX_SPLITS, ASSET_PLACEHOLDER_REGION,
    ASSET_REGION_CODE_LENGTH_CHARS, DEFAULT_REGION_CHAIN, EXTRA_CALLING_CODE_PREFIXES,
    INTERNATIONAL_PREFIXES, PATTERN_DIGIT_SLOT, PATTERN_DIGIT_SLOT_CASE_INSENSITIVE,
    PHONE_MAX_NATIONAL_LENGTH, PHONE_MIN_NATIONAL_LENGTH, PHONE_TERRITORIES,
    PRIMARY_FOR_SHARED_CALLING_CODE, REGIONAL_INDICATOR_BASE, REGIONS, RegionPhoneDescriptor,
    RegionPhoneTerritory,
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PhoneInputCountry {
    pub id: String,
    pub region_code: String,
    pub calling_code: String,
    pub format_pattern: String,
    pub english_name: String,
}

impl PhoneInputCountry {
    pub fn localized_name(&self) -> String {
        Locale::app_locale()
            .localized_string_for_region_code(&self.region_code)
            .unwrap_or_else(|| self.english_name.clone())
    }

    pub fn flag(&self) -> String {
        self.region_code
            .to_uppercase()
            .chars()
            .filter_map(|scalar| char::from_u32(REGIONAL_INDICATOR_BASE + u32::from(scalar)))
            .collect()
    }
}

pub struct PhoneCountryCatalog;

static ALL: LazyLock<Vec<PhoneInputCountry>> = LazyLock::new(|| {
    REGIONS
        .iter()
        .map(|region| PhoneInputCountry {
            id: region.code.to_owned(),
            region_code: region.code.to_owned(),
            calling_code: format!("+{}", region.calling_code),
            format_pattern: region.format_pattern.to_owned(),
            english_name: region.english_name.to_owned(),
        })
        .collect()
});

static BY_REGION: LazyLock<HashMap<String, PhoneInputCountry>> = LazyLock::new(|| {
    ALL.iter()
        .map(|country| (country.region_code.clone(), country.clone()))
        .collect()
});

static BY_CALLING_CODE: LazyLock<HashMap<String, PhoneInputCountry>> = LazyLock::new(|| {
    let mut out: HashMap<String, PhoneInputCountry> = HashMap::new();
    for region in REGIONS.iter() {
        let Some(country) = BY_REGION.get(region.code) else { continue };
        let codes = std::iter::once(region.calling_code).chain(region.additional_calling_codes.iter().copied());
        for code in codes {
            let primary = PRIMARY_FOR_SHARED_CALLING_CODE
                .iter()
                .find(|(shared, _)| *shared == code)
                .map(|(_, primary)| *primary);
            if let Some(primary) = primary
                && primary != region.code
            {
                continue;
            }
            out.insert(code.to_owned(), country.clone());
        }
    }
    for (prefix, region_code) in EXTRA_CALLING_CODE_PREFIXES.iter() {
        let Some(country) = BY_REGION.get(*region_code) else { continue };
        out.insert((*prefix).to_owned(), country.clone());
    }
    out
});

static LONGEST_CALLING_CODE: LazyLock<usize> = LazyLock::new(|| {
    BY_CALLING_CODE
        .keys()
        .map(|key| key.chars().count())
        .max()
        .unwrap_or(1)
});

static PHONE_TERRITORIES_BY_CODE: LazyLock<HashMap<&'static str, Vec<&'static RegionPhoneTerritory>>> =
    LazyLock::new(|| {
        let ordered = PHONE_TERRITORIES
            .iter()
            .filter(|territory| territory.main)
            .chain(PHONE_TERRITORIES.iter().filter(|territory| !territory.main));
        let mut grouped: HashMap<&'static str, Vec<&'static RegionPhoneTerritory>> = HashMap::new();
        for territory in ordered {
            grouped.entry(territory.calling_code).or_default().push(territory);
        }
        grouped
    });

static MAX_PHONE_CALLING_CODE_LENGTH: LazyLock<usize> = LazyLock::new(|| {
    PHONE_TERRITORIES
        .iter()
        .map(|territory| territory.calling_code.chars().count())
        .max()
        .unwrap_or(0)
});

static PHONE_PATTERN_CACHE: LazyLock<NSCache<String, Arc<NSRegularExpression>>> = LazyLock::new(NSCache::new);

impl PhoneCountryCatalog {
    pub fn all() -> &'static [PhoneInputCountry] {
        &ALL
    }

    pub fn default_country() -> PhoneInputCountry {
        if let Some(preferred) = rise_platform::device_locale::current().region.map(|region| region.to_uppercase())
            && let Some(found) = BY_REGION.get(&preferred)
        {
            return found.clone();
        }
        for fallback in DEFAULT_REGION_CHAIN.iter() {
            if let Some(found) = BY_REGION.get(*fallback) {
                return found.clone();
            }
        }
        ALL[0].clone()
    }

    /// a number without an international marker is a local number
    /// reading it as international would hand it to whichever country its first digits spell
    pub fn country_matching_international_number(value: &str) -> Option<PhoneInputCountry> {
        let trimmed = trimming_whitespaces_and_newlines(value);
        let marker = INTERNATIONAL_PREFIXES.iter().find(|marker| trimmed.starts_with(**marker))?;

        let marker_digits = number_characters(marker).len();
        let digits: Vec<&str> = number_characters(trimmed).into_iter().skip(marker_digits).collect();
        if digits.is_empty() {
            return None;
        }

        let longest = (*LONGEST_CALLING_CODE).min(digits.len());
        for length in (1..=longest).rev() {
            if let Some(found) = BY_CALLING_CODE.get(&digits[..length].concat()) {
                return Some(found.clone());
            }
        }
        None
    }

    /// region a longer canon prefix carves out of a shared code
    /// eg +7 707 is Kazakhstan and not whoever holds +7, none when no longer prefix fits
    pub fn refined_country(calling_code: &str, national_digits: &str) -> Option<PhoneInputCountry> {
        let code = number_characters(calling_code).concat();
        let full = format!("{code}{national_digits}");
        let code_count = code.chars().count();
        EXTRA_CALLING_CODE_PREFIXES
            .iter()
            .filter(|(prefix, _)| {
                prefix.chars().count() > code_count && prefix.starts_with(code.as_str()) && full.starts_with(*prefix)
            })
            .max_by_key(|(prefix, _)| prefix.chars().count())
            .and_then(|(_, region_code)| BY_REGION.get(*region_code).cloned())
    }

    /// digits a full national number has by the region canon pattern, 0 when canon gives none
    pub fn national_digit_count(country: &PhoneInputCountry) -> usize {
        let slot = PATTERN_DIGIT_SLOT.to_string();
        characters(&country.format_pattern)
            .filter(|character| {
                if PATTERN_DIGIT_SLOT_CASE_INSENSITIVE {
                    compare_case_insensitive(character.as_str(), &slot).is_eq()
                } else {
                    character.as_str() == slot
                }
            })
            .count()
    }

    /// true when the server takes international_digits (calling code then national digits) as a real number
    /// otp-service refuses what its phonenumber crate doesnt validate
    /// so this follows the canon phone validity algorithm step by step
    pub fn is_phone_number_valid(international_digits: &str) -> bool {
        let digits: String = international_digits.chars().filter(char::is_ascii_digit).collect();
        let split_limit = (*MAX_PHONE_CALLING_CODE_LENGTH).min(digits.len());
        let Some(territories) = (0..split_limit)
            .map(|length| &digits[..=length])
            .find_map(|calling_code| PHONE_TERRITORIES_BY_CODE.get(calling_code).map(|found| (calling_code, found)))
        else {
            return false;
        };
        let (calling_code, territories) = territories;
        let mut national = digits[calling_code.len()..].to_owned();
        let without_leading_zeros = national.trim_start_matches('0').to_owned();
        if let Some(territory) = Self::phone_source(territories, &without_leading_zeros) {
            let mut stripped = Self::strip_national_prefix(territory, &national);
            if let Some(prefix) = territory.national_prefix
                && !prefix.is_empty()
                && stripped.starts_with(prefix)
            {
                stripped = stripped[prefix.len()..].to_owned();
            }
            if !Self::is_too_short(territory, &stripped) {
                national = stripped;
            }
        }
        let length = national.chars().count();
        if !(PHONE_MIN_NATIONAL_LENGTH..=PHONE_MAX_NATIONAL_LENGTH).contains(&length) {
            return false;
        }
        let Some(source) = Self::phone_source(territories, &national) else { return false };
        Self::is_known(source, &national)
    }

    fn phone_pattern(source: &str) -> Option<Arc<NSRegularExpression>> {
        if let Some(cached) = PHONE_PATTERN_CACHE.object(&source.to_owned()) {
            return Some(cached);
        }
        let pattern = Arc::new(NSRegularExpression::with_pattern(source).ok()?);
        PHONE_PATTERN_CACHE.set_object(pattern.clone(), source.to_owned());
        Some(pattern)
    }

    fn matches_from_start(source: &str, text: &str) -> bool {
        Self::phone_pattern(source)
            .and_then(|pattern| pattern.first_match(text, NSMatchingOptions::ANCHORED, NSRange::full(text)))
            .is_some()
    }

    fn phone_source(
        territories: &[&'static RegionPhoneTerritory],
        national: &str,
    ) -> Option<&'static RegionPhoneTerritory> {
        if territories.len() == 1 {
            return Some(territories[0]);
        }
        territories.iter().copied().find(|territory| match territory.leading_digits {
            Some(leading) => Self::matches_from_start(leading, national),
            None => Self::is_known(territory, national),
        })
    }

    fn matches_descriptor(descriptor: &RegionPhoneDescriptor, national: &str) -> bool {
        (descriptor.lengths.is_empty() || descriptor.lengths.contains(&national.chars().count()))
            && Self::matches_from_start(descriptor.pattern, national)
    }

    fn is_known(territory: &RegionPhoneTerritory, national: &str) -> bool {
        Self::matches_descriptor(&territory.general, national)
            && territory
                .types
                .iter()
                .any(|descriptor| Self::matches_descriptor(descriptor, national))
    }

    fn is_too_short(territory: &RegionPhoneTerritory, national: &str) -> bool {
        let general = &territory.general;
        let length = national.chars().count();
        if general.local_lengths.contains(&length) || general.lengths.is_empty() {
            return false;
        }
        length < general.lengths.iter().copied().min().unwrap_or(0)
    }

    fn strip_national_prefix(territory: &RegionPhoneTerritory, national: &str) -> String {
        let Some(parsing) = territory.national_prefix_for_parsing else {
            if let Some(prefix) = territory.national_prefix
                && !prefix.is_empty()
                && national.starts_with(prefix)
            {
                return national[prefix.len()..].to_owned();
            }
            return national.to_owned();
        };
        let Some(regex) = Self::phone_pattern(parsing) else { return national.to_owned() };
        let Some(found) = regex.first_match(national, NSMatchingOptions::NONE, NSRange::full(national)) else {
            return national.to_owned();
        };
        let whole = found.range();
        if whole.location != 0 {
            return national.to_owned();
        }
        let rest = national.get(whole.length..).unwrap_or_default();
        let Some(rule) = territory.national_prefix_transform_rule else { return rest.to_owned() };
        if found.range_at(found.number_of_ranges().saturating_sub(1)).is_none() {
            return rest.to_owned();
        }
        let transformed = regex.string_by_replacing_matches(national, NSMatchingOptions::ANCHORED, whole, rule);
        let keeps_original = Self::matches_descriptor(&territory.general, national)
            && !Self::matches_descriptor(&territory.general, &transformed);
        if keeps_original { national.to_owned() } else { transformed }
    }

    /// reader for the shared PhoneCountries.txt asset in the shape canon pins
    /// four fields split by semicolon, digits only calling code two letter region
    /// and the FT row is an end marker and not a territory
    pub fn parse(source: &str) -> Vec<PhoneInputCountry> {
        source
            .split(is_newline)
            .filter(|line| !line.is_empty())
            .enumerate()
            .filter_map(|(offset, raw_line)| {
                let columns: Vec<&str> = raw_line.splitn(ASSET_MAX_SPLITS + 1, ASSET_FIELD_SEPARATOR).collect();
                if columns.len() != ASSET_FIELD_COUNT {
                    return None;
                }

                let digits = trimming_whitespaces_and_newlines(columns[0]);
                let region = trimming_whitespaces_and_newlines(columns[1]).to_uppercase();
                let pattern = trimming_whitespaces_and_newlines(columns[2]);
                let name = trimming_whitespaces_and_newlines(columns[3]);
                if digits.is_empty()
                    || !characters(digits).all(|character| character.is_number())
                    || region.chars().count() != ASSET_REGION_CODE_LENGTH_CHARS
                    || region == ASSET_PLACEHOLDER_REGION
                    || name.is_empty()
                {
                    return None;
                }

                Some(PhoneInputCountry {
                    id: format!("{region}-{digits}-{offset}"),
                    region_code: region,
                    calling_code: format!("+{digits}"),
                    format_pattern: pattern.to_owned(),
                    english_name: name.to_owned(),
                })
            })
            .collect()
    }
}

/// the number characters of the text, in order
fn number_characters(text: &str) -> Vec<&str> {
    characters(text)
        .filter(Character::is_number)
        .map(|character| character.as_str())
        .collect()
}
