use std::sync::OnceLock;

/// One country in the phone field's picker. `calling_code` is digits only — no
/// `+`, which the server prefixes itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhoneCountry {
    pub region: String,
    pub calling_code: String,
    /// `XXX XXX XX XX` — where the digits of a NATIONAL number go. Empty when
    /// the country has no pattern.
    pub format_pattern: String,
    pub english_name: String,
}

impl PhoneCountry {
    /// The regional-indicator pair for the region code, for accessibility labels
    /// and fallback — the picker's artwork comes from `FlagUi`.
    pub fn flag_emoji(&self) -> String {
        self.region
            .chars()
            .filter(char::is_ascii_alphabetic)
            .filter_map(|c| char::from_u32(127_397 + u32::from(c.to_ascii_uppercase())))
            .collect()
    }

    pub fn display_code(&self) -> String {
        format!("+{}", self.calling_code)
    }

    /// How many digits a national number has here, read off the pattern. `None`
    /// when there is no pattern, in which case nothing is enforced.
    pub fn national_digits(&self) -> Option<usize> {
        let count = self
            .format_pattern
            .chars()
            .filter(|c| c.eq_ignore_ascii_case(&'X'))
            .count();
        (count > 0).then_some(count)
    }

    /// Groups digits the way the pattern says, for display only.
    pub fn format_national(&self, digits: &str) -> String {
        let digits: Vec<char> = digits.chars().filter(char::is_ascii_digit).collect();
        if self.format_pattern.is_empty() {
            return digits.into_iter().collect();
        }

        let mut out = String::new();
        let mut next = digits.iter().peekable();
        for slot in self.format_pattern.chars() {
            // A separator only when a digit follows, or a half-typed number ends
            // in a trailing space and the caret sits past the typing.
            if next.peek().is_none() {
                break;
            }
            if slot.eq_ignore_ascii_case(&'X') {
                if let Some(digit) = next.next() {
                    out.push(*digit);
                }
            } else if !out.is_empty() {
                out.push(slot);
            }
        }
        // Digits past the pattern still belong to the user.
        out.extend(next);
        out
    }
}

/// The countries the product knows, parsed from a shipped data file in
/// `callingCode;region;pattern;englishName` format.
pub struct PhoneCountryCatalog {
    countries: Vec<PhoneCountry>,
}

/// Installed once at startup; this crate must not decide where the asset lives.
static CATALOG: OnceLock<PhoneCountryCatalog> = OnceLock::new();

pub fn install_countries(source: &str) {
    let _ = CATALOG.set(PhoneCountryCatalog::parse(source));
}

pub fn countries() -> &'static PhoneCountryCatalog {
    CATALOG.get_or_init(|| PhoneCountryCatalog::parse(""))
}

impl PhoneCountryCatalog {
    pub fn parse(source: &str) -> Self {
        let mut countries: Vec<PhoneCountry> = source
            .lines()
            .filter_map(|line| {
                let mut columns = line.splitn(4, ';');
                let calling_code = columns.next()?.trim();
                let region = columns.next()?.trim().to_ascii_uppercase();
                let pattern = columns.next()?.trim();
                let name = columns.next()?.trim();

                // `FT` is a placeholder row rather than a country.
                if calling_code.is_empty()
                    || !calling_code.chars().all(|c| c.is_ascii_digit())
                    || region.len() != 2
                    || region == "FT"
                    || name.is_empty()
                {
                    return None;
                }

                Some(PhoneCountry {
                    region,
                    calling_code: calling_code.to_owned(),
                    format_pattern: pattern.to_owned(),
                    english_name: name.to_owned(),
                })
            })
            .collect();

        if countries.is_empty() {
            countries = Self::fallback();
        }

        countries.sort_by(|left, right| left.english_name.cmp(&right.english_name));
        Self { countries }
    }

    /// What the picker shows when the data file is missing; a phone field with
    /// no countries cannot be used at all.
    fn fallback() -> Vec<PhoneCountry> {
        vec![
            PhoneCountry {
                region: "KZ".into(),
                calling_code: "7".into(),
                format_pattern: "XXX XXX XX XX".into(),
                english_name: "Kazakhstan".into(),
            },
            PhoneCountry {
                region: "US".into(),
                calling_code: "1".into(),
                format_pattern: "XXX XXX XXXX".into(),
                english_name: "United States".into(),
            },
        ]
    }

    pub fn all(&self) -> &[PhoneCountry] {
        &self.countries
    }

    pub fn by_region(&self, region: &str) -> Option<&PhoneCountry> {
        let region = region.to_ascii_uppercase();
        self.countries
            .iter()
            .find(|country| country.region == region)
    }

    /// The country the field opens on: the device's own region, then Kazakhstan,
    /// then the United States.
    pub fn default_country(&self, device_region: Option<&str>) -> &PhoneCountry {
        device_region
            .and_then(|region| self.by_region(region))
            .or_else(|| self.by_region("KZ"))
            .or_else(|| self.by_region("US"))
            .or_else(|| self.countries.first())
            .expect("the catalogue is never empty")
    }

    /// Which country a number in international form belongs to. The longest
    /// calling code wins, so `+1264` is Anguilla rather than the United States.
    pub fn matching_international(&self, value: &str) -> Option<&PhoneCountry> {
        let trimmed = value.trim();
        if !trimmed.starts_with('+') && !trimmed.starts_with("00") {
            return None;
        }

        let all_digits: String = trimmed.chars().filter(char::is_ascii_digit).collect();
        let digits = all_digits.strip_prefix("00").unwrap_or(&all_digits);

        self.countries
            .iter()
            .filter(|country| digits.starts_with(&country.calling_code))
            .max_by_key(|country| country.calling_code.len())
    }

    pub fn search(&self, query: &str) -> Vec<&PhoneCountry> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return self.countries.iter().collect();
        }

        let digits = needle.trim_start_matches('+');
        self.countries
            .iter()
            .filter(|country| {
                country.english_name.to_lowercase().contains(&needle)
                    || country.region.to_lowercase() == needle
                    || country.calling_code.starts_with(digits)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn shipped() -> PhoneCountryCatalog {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/phone_countries.txt");
        let source = std::fs::read_to_string(path).unwrap_or_default();
        PhoneCountryCatalog::parse(&source)
    }

    #[test]
    fn the_shipped_file_parses_into_a_real_catalogue() {
        let catalog = shipped();
        assert!(
            catalog.all().len() > 200,
            "only {} countries parsed; the field would be missing most of the world",
            catalog.all().len()
        );

        let kz = catalog.by_region("KZ").expect("Kazakhstan");
        assert_eq!(kz.calling_code, "7");
        assert_eq!(kz.display_code(), "+7");
    }

    #[test]
    fn a_calling_code_never_carries_a_plus() {
        for country in shipped().all() {
            assert!(
                country.calling_code.chars().all(|c| c.is_ascii_digit()),
                "{} has a non-digit calling code {}; the server prefixes the + itself",
                country.region,
                country.calling_code
            );
        }
    }

    #[test]
    fn the_device_region_wins_and_the_reference_fallbacks_follow_it() {
        let catalog = shipped();

        assert_eq!(catalog.default_country(Some("KZ")).region, "KZ");
        assert_eq!(catalog.default_country(Some("ru")).region, "RU");
        assert_eq!(
            catalog.default_country(Some("ZZ")).region,
            "KZ",
            "an unknown region falls back to the reference's first choice"
        );
        assert_eq!(catalog.default_country(None).region, "KZ");
    }

    #[test]
    fn two_countries_can_share_a_calling_code_and_both_survive() {
        let catalog = shipped();
        let sharing: Vec<&str> = catalog
            .all()
            .iter()
            .filter(|country| country.calling_code == "7")
            .map(|country| country.region.as_str())
            .collect();

        assert!(sharing.contains(&"RU"));
        assert!(sharing.contains(&"KZ"));
    }

    #[test]
    fn a_pasted_international_number_finds_its_country_by_the_longest_code() {
        let catalog = shipped();

        assert_eq!(
            catalog
                .matching_international("+77075803272")
                .map(|c| c.calling_code.clone()),
            Some("7".to_owned())
        );
        assert_eq!(
            catalog
                .matching_international("+1264 555 0100")
                .map(|c| c.region.clone()),
            Some("AI".to_owned()),
            "the longest matching code wins, or every +1 country reads as the US"
        );
        assert!(
            catalog.matching_international("77075803272").is_none(),
            "without a + there is nothing to say the leading digits are a country code"
        );
    }

    #[test]
    fn a_double_zero_prefix_is_an_international_number_too() {
        let catalog = shipped();
        assert_eq!(
            catalog
                .matching_international("0077075803272")
                .map(|c| c.calling_code.clone()),
            Some("7".to_owned())
        );
    }

    #[test]
    fn the_pattern_says_how_many_national_digits_there_are() {
        let kazakhstan = PhoneCountry {
            region: "KZ".into(),
            calling_code: "7".into(),
            format_pattern: "XXX XXX XX XX".into(),
            english_name: "Kazakhstan".into(),
        };
        assert_eq!(kazakhstan.national_digits(), Some(10));

        let unknown = PhoneCountry {
            format_pattern: String::new(),
            ..kazakhstan
        };
        assert_eq!(unknown.national_digits(), None);
    }

    #[test]
    fn formatting_groups_the_digits_and_never_invents_any() {
        let kazakhstan = PhoneCountry {
            region: "KZ".into(),
            calling_code: "7".into(),
            format_pattern: "XXX XXX XX XX".into(),
            english_name: "Kazakhstan".into(),
        };

        assert_eq!(kazakhstan.format_national("7075803272"), "707 580 32 72");
        assert_eq!(kazakhstan.format_national("707"), "707");
        assert_eq!(
            kazakhstan.format_national(""),
            "",
            "an empty field must not show separators"
        );
        assert_eq!(
            kazakhstan
                .format_national("70758032721234")
                .chars()
                .filter(char::is_ascii_digit)
                .count(),
            14,
            "digits past the pattern still belong to the user"
        );
    }

    #[test]
    fn search_finds_a_country_by_name_by_region_and_by_code() {
        let catalog = shipped();

        assert!(catalog.search("kazakh").iter().any(|c| c.region == "KZ"));
        assert!(catalog.search("kz").iter().any(|c| c.region == "KZ"));
        assert!(catalog.search("+7").iter().any(|c| c.calling_code == "7"));
        assert_eq!(catalog.search("").len(), catalog.all().len());
    }

    #[test]
    fn a_missing_data_file_still_leaves_a_usable_field() {
        let empty = PhoneCountryCatalog::parse("");
        assert!(!empty.all().is_empty());
        assert_eq!(empty.default_country(None).region, "KZ");
    }

    #[test]
    fn a_malformed_row_is_skipped_rather_than_poisoning_the_list() {
        let catalog = PhoneCountryCatalog::parse(
            "7;KZ;XXX XXX XX XX;Kazakhstan\n\
             nonsense\n\
             ;;;\n\
             999;FT;;Placeholder\n\
             1;US;XXX XXX XXXX;United States\n",
        );

        let regions: Vec<&str> = catalog.all().iter().map(|c| c.region.as_str()).collect();
        assert_eq!(regions, vec!["KZ", "US"]);
    }

    #[test]
    fn the_list_is_alphabetical_so_the_picker_is_scannable() {
        let catalog = shipped();
        let names: Vec<&str> = catalog
            .all()
            .iter()
            .map(|c| c.english_name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
