use rise_ui::phone_input::PhoneCountry;

/// Which countries a query leaves, as indices into the full catalogue.
///
/// The order is the catalogue's own, with one exception: an exact region or
/// calling-code hit comes first.
pub fn matches(countries: &[PhoneCountry], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return (0..countries.len()).collect();
    }

    let digits = needle.trim_start_matches('+');
    let searching_digits = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());

    let mut exact = Vec::new();
    let mut rest = Vec::new();

    for (index, country) in countries.iter().enumerate() {
        let region = country.region.to_ascii_lowercase();
        let name = country.english_name.to_lowercase();

        if region == needle || (searching_digits && country.calling_code == digits) {
            exact.push(index);
        } else if name.contains(&needle)
            || (searching_digits && country.calling_code.starts_with(digits))
        {
            rest.push(index);
        }
    }

    exact.extend(rest);
    exact
}

/// Where the highlight lands after a step of `delta` rows. Clamped, not wrapped.
pub fn step_highlight(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    let next = current as isize + delta;
    next.clamp(0, last as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Vec<PhoneCountry> {
        vec![
            country("KZ", "7", "Kazakhstan"),
            country("RU", "7", "Russian Federation"),
            country("UA", "380", "Ukraine"),
            country("US", "1", "United States"),
            country("GB", "44", "United Kingdom"),
        ]
    }

    fn country(region: &str, code: &str, name: &str) -> PhoneCountry {
        PhoneCountry {
            region: region.to_owned(),
            calling_code: code.to_owned(),
            format_pattern: String::new(),
            english_name: name.to_owned(),
        }
    }

    fn named(countries: &[PhoneCountry], indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .map(|index| countries[*index].english_name.clone())
            .collect()
    }

    #[test]
    fn an_empty_query_keeps_the_whole_catalogue_in_its_own_order() {
        let all = catalogue();
        assert_eq!(matches(&all, ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(matches(&all, "   "), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_name_matches_anywhere_in_it() {
        let all = catalogue();
        assert_eq!(
            named(&all, &matches(&all, "united")),
            ["United States", "United Kingdom"]
        );
        assert_eq!(named(&all, &matches(&all, "KAZAK")), ["Kazakhstan"]);
    }

    #[test]
    fn a_region_code_names_one_country_and_puts_it_first() {
        let all = catalogue();
        let found = matches(&all, "ua");
        assert_eq!(named(&all, &found), ["Ukraine"]);
    }

    #[test]
    fn a_calling_code_matches_with_or_without_the_plus() {
        let all = catalogue();
        for query in ["+380", "380"] {
            assert_eq!(named(&all, &matches(&all, query)), ["Ukraine"], "{query}");
        }
    }

    /// `+1` must not be buried under the two dozen codes that start with 1.
    #[test]
    fn an_exact_calling_code_outranks_the_ones_that_only_start_with_it() {
        let mut all = catalogue();
        all.push(country("AI", "1264", "Anguilla"));

        assert_eq!(
            named(&all, &matches(&all, "+1")),
            ["United States", "Anguilla"]
        );
    }

    #[test]
    fn a_query_that_matches_nothing_returns_nothing() {
        let all = catalogue();
        assert!(matches(&all, "zzzz").is_empty());
    }

    #[test]
    fn the_highlight_stops_at_both_ends_instead_of_wrapping() {
        assert_eq!(step_highlight(0, -1, 5), 0);
        assert_eq!(step_highlight(4, 1, 5), 4);
        assert_eq!(step_highlight(2, 1, 5), 3);
        assert_eq!(step_highlight(2, -1, 5), 1);
    }

    #[test]
    fn the_highlight_survives_an_empty_list() {
        assert_eq!(step_highlight(0, 1, 0), 0);
        assert_eq!(step_highlight(7, -1, 0), 0);
    }

    #[test]
    fn a_jump_past_the_end_lands_on_the_end() {
        assert_eq!(step_highlight(0, isize::MAX, 5), 4);
        assert_eq!(step_highlight(4, isize::MIN, 5), 0);
    }
}
