//! CLDR cardinal plural selection; must agree with `backend/common/src/i18n/plural.rs`.
//!
//! Only integer counts are pluralised, so rules are evaluated against `i` with
//! `v = 0`; fractional operands are not modelled.

use crate::app_language_catalog::PluralRules;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }
}

impl PluralRules {
    pub fn category(self, count: i64) -> PluralCategory {
        let n = count.unsigned_abs();
        let mod10 = n % 10;
        let mod100 = n % 100;

        match self {
            Self::OtherOnly => PluralCategory::Other,

            Self::OneOther => {
                if n == 1 {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }

            Self::OneZeroOther => {
                if n <= 1 {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }

            Self::Slavic => {
                if mod10 == 1 && mod100 != 11 {
                    PluralCategory::One
                } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                    PluralCategory::Few
                } else {
                    PluralCategory::Many
                }
            }

            Self::Polish => {
                if n == 1 {
                    PluralCategory::One
                } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                    PluralCategory::Few
                } else {
                    PluralCategory::Many
                }
            }

            Self::Czech => {
                if n == 1 {
                    PluralCategory::One
                } else if (2..=4).contains(&n) {
                    PluralCategory::Few
                } else {
                    PluralCategory::Other
                }
            }

            Self::Serbian => {
                if mod10 == 1 && mod100 != 11 {
                    PluralCategory::One
                } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                    PluralCategory::Few
                } else {
                    PluralCategory::Other
                }
            }

            Self::Romanian => {
                if n == 1 {
                    PluralCategory::One
                } else if n == 0 || (1..=19).contains(&mod100) {
                    PluralCategory::Few
                } else {
                    PluralCategory::Other
                }
            }

            Self::Hebrew => match n {
                1 => PluralCategory::One,
                2 => PluralCategory::Two,
                _ => PluralCategory::Other,
            },

            Self::Arabic => {
                if n == 0 {
                    PluralCategory::Zero
                } else if n == 1 {
                    PluralCategory::One
                } else if n == 2 {
                    PluralCategory::Two
                } else if (3..=10).contains(&mod100) {
                    PluralCategory::Few
                } else if (11..=99).contains(&mod100) {
                    PluralCategory::Many
                } else {
                    PluralCategory::Other
                }
            }

            Self::Filipino => {
                if (1..=3).contains(&n) || !matches!(mod10, 4 | 6 | 9) {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }
        }
    }

    /// Candidate key suffixes in preference order: the exact category, then `other`.
    pub fn key_suffixes(self, count: i64) -> [&'static str; 2] {
        [
            self.category(count).as_str(),
            PluralCategory::Other.as_str(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(rules: PluralRules, counts: &[i64]) -> Vec<&'static str> {
        counts.iter().map(|n| rules.category(*n).as_str()).collect()
    }

    #[test]
    fn russian_matches_cldr() {
        assert_eq!(
            cats(
                PluralRules::Slavic,
                &[0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111]
            ),
            vec![
                "many", "one", "few", "few", "many", "many", "many", "many", "one", "few", "many",
                "one", "many"
            ]
        );
    }

    #[test]
    fn ukrainian_shares_the_slavic_family() {
        assert_eq!(PluralRules::Slavic.category(21), PluralCategory::One);
    }

    #[test]
    fn polish_differs_from_russian_at_one_hundred_and_one() {
        assert_eq!(PluralRules::Slavic.category(101), PluralCategory::One);
        assert_eq!(PluralRules::Polish.category(101), PluralCategory::Many);
        assert_eq!(
            cats(PluralRules::Polish, &[1, 2, 5, 12, 22, 25]),
            vec!["one", "few", "many", "many", "few", "many"]
        );
    }

    #[test]
    fn czech_matches_cldr() {
        assert_eq!(
            cats(PluralRules::Czech, &[0, 1, 2, 4, 5, 100]),
            vec!["other", "one", "few", "few", "other", "other"]
        );
    }

    #[test]
    fn serbian_matches_cldr() {
        assert_eq!(
            cats(PluralRules::Serbian, &[1, 2, 5, 11, 21, 22]),
            vec!["one", "few", "other", "other", "one", "few"]
        );
    }

    #[test]
    fn romanian_matches_cldr() {
        assert_eq!(
            cats(PluralRules::Romanian, &[0, 1, 2, 19, 20, 101, 119, 120]),
            vec!["few", "one", "few", "few", "other", "few", "few", "other"]
        );
    }

    #[test]
    fn arabic_uses_all_six_categories() {
        assert_eq!(
            cats(PluralRules::Arabic, &[0, 1, 2, 3, 10, 11, 99, 100, 103]),
            vec![
                "zero", "one", "two", "few", "few", "many", "many", "other", "few"
            ]
        );
    }

    #[test]
    fn hebrew_has_a_dual() {
        assert_eq!(
            cats(PluralRules::Hebrew, &[1, 2, 3, 10, 20]),
            vec!["one", "two", "other", "other", "other"]
        );
    }

    #[test]
    fn zero_and_one_share_a_form_in_french_family() {
        assert_eq!(
            cats(PluralRules::OneZeroOther, &[0, 1, 2]),
            vec!["one", "one", "other"]
        );
        assert_eq!(
            cats(PluralRules::OneOther, &[0, 1, 2]),
            vec!["other", "one", "other"]
        );
    }

    #[test]
    fn other_only_never_varies() {
        for n in [0, 1, 2, 5, 11, 100] {
            assert_eq!(PluralRules::OtherOnly.category(n), PluralCategory::Other);
        }
    }

    #[test]
    fn filipino_treats_most_numbers_as_one() {
        assert_eq!(
            cats(PluralRules::Filipino, &[1, 2, 3, 4, 5, 6, 9, 10, 14]),
            vec![
                "one", "one", "one", "other", "one", "other", "other", "one", "other"
            ]
        );
    }

    #[test]
    fn negative_counts_use_the_absolute_value() {
        assert_eq!(PluralRules::Slavic.category(-1), PluralCategory::One);
        assert_eq!(PluralRules::Slavic.category(-22), PluralCategory::Few);
        assert_eq!(PluralRules::Arabic.category(-2), PluralCategory::Two);
    }

    #[test]
    fn suffixes_always_fall_back_to_other() {
        assert_eq!(PluralRules::Slavic.key_suffixes(2), ["few", "other"]);
        assert_eq!(PluralRules::Slavic.key_suffixes(1), ["one", "other"]);
        assert_eq!(PluralRules::OtherOnly.key_suffixes(7), ["other", "other"]);
    }

    #[test]
    fn i64_extremes_do_not_panic() {
        for rules in ALL_FAMILIES {
            let _ = rules.category(i64::MIN);
            let _ = rules.category(i64::MAX);
        }
    }

    /// Every family, against the same counts the server's own suite asserts.
    #[test]
    fn every_family_agrees_with_the_server_table() {
        let counts: [i64; 16] = [0, 1, 2, 3, 4, 5, 10, 11, 12, 14, 19, 20, 21, 22, 25, 101];
        let expected: [(PluralRules, [&str; 16]); 11] = [
            (
                PluralRules::OtherOnly,
                [
                    "other", "other", "other", "other", "other", "other", "other", "other",
                    "other", "other", "other", "other", "other", "other", "other", "other",
                ],
            ),
            (
                PluralRules::OneOther,
                [
                    "other", "one", "other", "other", "other", "other", "other", "other", "other",
                    "other", "other", "other", "other", "other", "other", "other",
                ],
            ),
            (
                PluralRules::OneZeroOther,
                [
                    "one", "one", "other", "other", "other", "other", "other", "other", "other",
                    "other", "other", "other", "other", "other", "other", "other",
                ],
            ),
            (
                PluralRules::Slavic,
                [
                    "many", "one", "few", "few", "few", "many", "many", "many", "many", "many",
                    "many", "many", "one", "few", "many", "one",
                ],
            ),
            (
                PluralRules::Polish,
                [
                    "many", "one", "few", "few", "few", "many", "many", "many", "many", "many",
                    "many", "many", "many", "few", "many", "many",
                ],
            ),
            (
                PluralRules::Czech,
                [
                    "other", "one", "few", "few", "few", "other", "other", "other", "other",
                    "other", "other", "other", "other", "other", "other", "other",
                ],
            ),
            (
                PluralRules::Serbian,
                [
                    "other", "one", "few", "few", "few", "other", "other", "other", "other",
                    "other", "other", "other", "one", "few", "other", "one",
                ],
            ),
            (
                PluralRules::Romanian,
                [
                    "few", "one", "few", "few", "few", "few", "few", "few", "few", "few", "few",
                    "other", "other", "other", "other", "few",
                ],
            ),
            (
                PluralRules::Hebrew,
                [
                    "other", "one", "two", "other", "other", "other", "other", "other", "other",
                    "other", "other", "other", "other", "other", "other", "other",
                ],
            ),
            (
                PluralRules::Arabic,
                [
                    "zero", "one", "two", "few", "few", "few", "few", "many", "many", "many",
                    "many", "many", "many", "many", "many", "other",
                ],
            ),
            (
                PluralRules::Filipino,
                [
                    "one", "one", "one", "one", "other", "one", "one", "one", "one", "other",
                    "other", "one", "one", "one", "one", "one",
                ],
            ),
        ];

        assert_eq!(expected.len(), ALL_FAMILIES.len());
        for (rules, want) in expected {
            assert_eq!(cats(rules, &counts), want.to_vec(), "family {rules:?}");
        }
    }

    const ALL_FAMILIES: [PluralRules; 11] = [
        PluralRules::OtherOnly,
        PluralRules::OneOther,
        PluralRules::OneZeroOther,
        PluralRules::Slavic,
        PluralRules::Polish,
        PluralRules::Czech,
        PluralRules::Serbian,
        PluralRules::Romanian,
        PluralRules::Hebrew,
        PluralRules::Arabic,
        PluralRules::Filipino,
    ];
}
