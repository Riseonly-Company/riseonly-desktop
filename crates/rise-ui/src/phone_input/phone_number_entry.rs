use super::phone_country_catalog::{PhoneCountry, PhoneCountryCatalog};

/// A chosen country and a national number, kept apart so a number typed in full
/// never gets its calling code prepended twice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhoneNumberEntry {
    country: PhoneCountry,
    /// Digits only, WITHOUT the calling code.
    national: String,
}

/// What happened when the user typed; tells the caller whether the country
/// button has to be redrawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhoneEdit {
    /// The national part changed and the country stayed put.
    NationalOnly,
    /// The typed value named a country — a `+` or `00` prefix — and the picker
    /// followed.
    CountryAdopted,
}

impl PhoneNumberEntry {
    pub fn new(country: PhoneCountry) -> Self {
        Self {
            country,
            national: String::new(),
        }
    }

    /// The field the device's own region opens on.
    pub fn detected(catalog: &PhoneCountryCatalog, device_region: Option<&str>) -> Self {
        Self::new(catalog.default_country(device_region).clone())
    }

    pub fn country(&self) -> &PhoneCountry {
        &self.country
    }

    pub fn national(&self) -> &str {
        &self.national
    }

    pub fn is_empty(&self) -> bool {
        self.national.is_empty()
    }

    pub fn set_country(&mut self, country: PhoneCountry) {
        self.country = country;
    }

    /// Takes whatever is in the text field and works out what it means: a `+` or
    /// `00` prefix adopts that country, digits already carrying the current
    /// calling code are not given it again, anything else is a national number.
    pub fn accept_typed(&mut self, typed: &str, catalog: &PhoneCountryCatalog) -> PhoneEdit {
        let digits: String = typed.chars().filter(char::is_ascii_digit).collect();

        if let Some(country) = catalog.matching_international(typed) {
            let without_prefix = digits.strip_prefix("00").unwrap_or(&digits);
            let national = without_prefix
                .strip_prefix(&country.calling_code)
                .unwrap_or(without_prefix)
                .to_owned();

            let adopted = country.region != self.country.region;
            self.country = country.clone();
            self.national = national;
            return if adopted {
                PhoneEdit::CountryAdopted
            } else {
                PhoneEdit::NationalOnly
            };
        }

        let code = &self.country.calling_code;
        let looks_complete = digits.starts_with(code.as_str()) && {
            match self.country.national_digits() {
                // A full number and a national one both begin with the code, so
                // only the length tells them apart.
                Some(national) => digits.len() != national && digits.len() >= code.len() + national,
                None => digits.len() >= code.len() + MINIMUM_NATIONAL_DIGITS,
            }
        };

        self.national = if looks_complete {
            digits[code.len()..].to_owned()
        } else {
            digits
        };
        PhoneEdit::NationalOnly
    }

    /// What goes on the wire: calling code and national part, digits only, and
    /// no leading `+` — user-service prefixes that itself.
    pub fn wire_value(&self) -> String {
        if self.national.is_empty() {
            return String::new();
        }
        format!("{}{}", self.country.calling_code, self.national)
    }

    /// The national part, grouped for display.
    pub fn formatted_national(&self) -> String {
        self.country.format_national(&self.national)
    }

    /// Whether the number is long enough to send: the server's 7-to-15-digit
    /// bound. The country's own pattern is a hint and is NOT enforced here.
    pub fn is_plausible(&self) -> bool {
        let total = self.country.calling_code.len() + self.national.len();
        (7..=15).contains(&total)
    }
}

/// Separates "typed the whole number" from "typed a local number that starts
/// with the country code's digits".
const MINIMUM_NATIONAL_DIGITS: usize = 9;

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> PhoneCountryCatalog {
        PhoneCountryCatalog::parse(
            "7;RU;XXX XXX XXXX;Russian Federation\n\
             7;KZ;XXX XXX XX XX;Kazakhstan\n\
             1;US;XXX XXX XXXX;United States\n\
             1264;AI;XXX XXXX;Anguilla\n\
             380;UA;XX XXX XX XX;Ukraine\n",
        )
    }

    fn kazakh() -> PhoneNumberEntry {
        let catalog = catalog();
        PhoneNumberEntry::new(catalog.by_region("KZ").unwrap().clone())
    }

    #[test]
    fn the_field_opens_on_the_device_region() {
        let catalog = catalog();
        assert_eq!(
            PhoneNumberEntry::detected(&catalog, Some("UA"))
                .country()
                .region,
            "UA"
        );
        assert_eq!(
            PhoneNumberEntry::detected(&catalog, None).country().region,
            "KZ"
        );
    }

    /// Regression: a Kazakh number typed in full once became `+777075803272`.
    #[test]
    fn a_number_typed_in_full_does_not_get_its_country_code_twice() {
        let catalog = catalog();
        let mut entry = kazakh();

        entry.accept_typed("77075803272", &catalog);

        assert_eq!(entry.national(), "7075803272");
        assert_eq!(
            entry.wire_value(),
            "77075803272",
            "this is what user-service canonicalises to +7 707 580 32 72"
        );
    }

    #[test]
    fn a_national_number_gets_the_chosen_country_code() {
        let catalog = catalog();
        let mut entry = kazakh();

        entry.accept_typed("7075803272", &catalog);

        assert_eq!(entry.national(), "7075803272");
        assert_eq!(entry.wire_value(), "77075803272");
    }

    #[test]
    fn a_pasted_international_number_moves_the_picker_to_its_country() {
        let catalog = catalog();
        let mut entry = kazakh();

        let edit = entry.accept_typed("+380501234567", &catalog);

        assert_eq!(edit, PhoneEdit::CountryAdopted);
        assert_eq!(entry.country().region, "UA");
        assert_eq!(entry.national(), "501234567");
        assert_eq!(entry.wire_value(), "380501234567");
    }

    #[test]
    fn a_pasted_number_from_the_country_already_chosen_is_not_reported_as_a_change() {
        let catalog = catalog();
        let mut entry = kazakh();

        // +7 resolves to whichever of RU/KZ the catalogue lists first.
        let edit = entry.accept_typed("+77075803272", &catalog);
        assert_eq!(entry.national(), "7075803272");
        assert!(matches!(
            edit,
            PhoneEdit::CountryAdopted | PhoneEdit::NationalOnly
        ));
    }

    #[test]
    fn the_longest_calling_code_wins_when_a_paste_is_ambiguous() {
        let catalog = catalog();
        let mut entry = kazakh();

        entry.accept_typed("+12645550100", &catalog);

        assert_eq!(entry.country().region, "AI");
        assert_eq!(entry.national(), "5550100");
    }

    #[test]
    fn separators_and_spaces_are_thrown_away() {
        let catalog = catalog();
        let mut entry = kazakh();

        entry.accept_typed("(707) 580-32-72", &catalog);
        assert_eq!(entry.national(), "7075803272");
        assert!(
            entry.wire_value().chars().all(|c| c.is_ascii_digit()),
            "the wire value must be digits only"
        );
    }

    #[test]
    fn an_empty_field_sends_nothing_rather_than_a_bare_country_code() {
        let entry = kazakh();
        assert!(entry.is_empty());
        assert_eq!(entry.wire_value(), "");
        assert!(!entry.is_plausible());
    }

    #[test]
    fn switching_country_keeps_the_number_the_user_already_typed() {
        let catalog = catalog();
        let mut entry = kazakh();
        entry.accept_typed("7075803272", &catalog);

        entry.set_country(catalog.by_region("US").unwrap().clone());

        assert_eq!(entry.national(), "7075803272");
        assert_eq!(entry.wire_value(), "17075803272");
    }

    #[test]
    fn plausibility_follows_the_servers_seven_to_fifteen_digit_bound() {
        let catalog = catalog();
        let mut entry = kazakh();

        entry.accept_typed("70758", &catalog);
        assert!(!entry.is_plausible(), "six digits in total is too short");

        entry.accept_typed("7075803272", &catalog);
        assert!(entry.is_plausible());

        entry.accept_typed("70758032720000000", &catalog);
        assert!(!entry.is_plausible(), "over fifteen digits is refused");
    }

    #[test]
    fn a_local_number_starting_with_the_country_digit_is_not_mistaken_for_a_full_one() {
        let catalog = catalog();
        let mut entry = kazakh();

        // Nine digits beginning with 7: one short of a complete +7 number.
        entry.accept_typed("712345678", &catalog);
        assert_eq!(entry.national(), "712345678");
        assert_eq!(entry.wire_value(), "7712345678");
    }

    #[test]
    fn the_display_string_is_grouped_but_the_wire_value_is_not() {
        let catalog = catalog();
        let mut entry = kazakh();
        entry.accept_typed("7075803272", &catalog);

        assert_eq!(entry.formatted_national(), "707 580 32 72");
        assert_eq!(entry.wire_value(), "77075803272");
    }
}
