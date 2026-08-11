use crate::charset::Charset;
use crate::violation::{Edge, Unit, Violation, ViolationKind};

/// One rule, and the i18n key its failure renders from.
///
/// `const`-constructible, so a whole schema can be a `const` beside its field.
#[derive(Clone, Copy, Debug)]
pub struct Check {
    rule: Rule,
    message: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
enum Rule {
    MinChars(usize),
    MaxChars(usize),
    MinBytes(usize),
    MaxBytes(usize),
    MinDigits(usize),
    MaxDigits(usize),
    Charset(Charset),
    NotEdgedWith(char),
    NoRunOf(&'static str),
    Predicate {
        rule: &'static str,
        test: fn(&str) -> bool,
    },
}

impl Check {
    /// Attaches the i18n key this rule's failure renders as.
    pub const fn message(mut self, key: &'static str) -> Self {
        self.message = Some(key);
        self
    }

    pub const fn min_chars(minimum: usize) -> Self {
        Self::of(Rule::MinChars(minimum))
    }

    pub const fn max_chars(maximum: usize) -> Self {
        Self::of(Rule::MaxChars(maximum))
    }

    pub const fn min_bytes(minimum: usize) -> Self {
        Self::of(Rule::MinBytes(minimum))
    }

    pub const fn max_bytes(maximum: usize) -> Self {
        Self::of(Rule::MaxBytes(maximum))
    }

    pub const fn min_digits(minimum: usize) -> Self {
        Self::of(Rule::MinDigits(minimum))
    }

    pub const fn max_digits(maximum: usize) -> Self {
        Self::of(Rule::MaxDigits(maximum))
    }

    pub const fn charset(charset: Charset) -> Self {
        Self::of(Rule::Charset(charset))
    }

    /// Legal inside the value, refused at either end — the tag underscore rule.
    pub const fn not_edged_with(character: char) -> Self {
        Self::of(Rule::NotEdgedWith(character))
    }

    /// Legal once, refused twice in a row.
    pub const fn no_run_of(run: &'static str) -> Self {
        Self::of(Rule::NoRunOf(run))
    }

    /// The escape hatch. `rule` is a stable slug that ends up in the violation, never prose.
    pub const fn predicate(rule: &'static str, test: fn(&str) -> bool) -> Self {
        Self::of(Rule::Predicate { rule, test })
    }

    const fn of(rule: Rule) -> Self {
        Self {
            rule,
            message: None,
        }
    }

    fn evaluate(&self, value: &str) -> Option<ViolationKind> {
        match self.rule {
            Rule::MinChars(minimum) => {
                let actual = value.chars().count();
                (actual < minimum).then_some(ViolationKind::TooShort {
                    minimum,
                    actual,
                    unit: Unit::Characters,
                })
            }
            Rule::MaxChars(maximum) => {
                let actual = value.chars().count();
                (actual > maximum).then_some(ViolationKind::TooLong {
                    maximum,
                    actual,
                    unit: Unit::Characters,
                })
            }
            Rule::MinBytes(minimum) => {
                let actual = value.len();
                (actual < minimum).then_some(ViolationKind::TooShort {
                    minimum,
                    actual,
                    unit: Unit::Bytes,
                })
            }
            Rule::MaxBytes(maximum) => {
                let actual = value.len();
                (actual > maximum).then_some(ViolationKind::TooLong {
                    maximum,
                    actual,
                    unit: Unit::Bytes,
                })
            }
            Rule::MinDigits(minimum) => {
                let actual = digits(value);
                (actual < minimum).then_some(ViolationKind::TooShort {
                    minimum,
                    actual,
                    unit: Unit::Digits,
                })
            }
            Rule::MaxDigits(maximum) => {
                let actual = digits(value);
                (actual > maximum).then_some(ViolationKind::TooLong {
                    maximum,
                    actual,
                    unit: Unit::Digits,
                })
            }
            Rule::Charset(charset) => {
                charset
                    .first_offender(value)
                    .map(|offending| ViolationKind::Charset {
                        charset: charset.name,
                        offending,
                    })
            }
            Rule::NotEdgedWith(character) => {
                if value.starts_with(character) {
                    Some(ViolationKind::Edge {
                        character,
                        edge: Edge::Leading,
                    })
                } else if value.ends_with(character) {
                    Some(ViolationKind::Edge {
                        character,
                        edge: Edge::Trailing,
                    })
                } else {
                    None
                }
            }
            Rule::NoRunOf(run) => {
                (!run.is_empty() && value.contains(run)).then_some(ViolationKind::Run { run })
            }
            Rule::Predicate { rule, test } => {
                (!test(value)).then_some(ViolationKind::Failed { rule })
            }
        }
    }
}

fn digits(value: &str) -> usize {
    value.chars().filter(char::is_ascii_digit).count()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Normalize {
    trim: bool,
    lowercase: bool,
    digits_only: bool,
}

/// A field's rules, in one value: declared once as a `const` next to the field,
/// and read by every store, repository and test that guards it.
///
/// ```
/// use rise_validate::{Charset, Check, TextSchema};
///
/// const TAG: TextSchema = TextSchema::new("tag")
///     .trim()
///     .lowercase()
///     .checks(&[
///         Check::min_chars(3).message("auth_tag_too_short"),
///         Check::max_bytes(32).message("auth_tag_too_long"),
///         Check::charset(Charset::TAG).message("auth_tag_alphabet"),
///         Check::not_edged_with('_').message("auth_tag_underscore_edge"),
///         Check::no_run_of("__").message("auth_tag_double_underscore"),
///     ]);
///
/// // `check` returns the NORMALISED value — there is no second call to forget.
/// assert_eq!(TAG.check("  RiseOnly  ").unwrap(), "riseonly");
/// assert!(TAG.check("ab").is_err());
/// ```
#[derive(Clone, Copy, Debug)]
pub struct TextSchema {
    field: &'static str,
    normalize: Normalize,
    checks: &'static [Check],
    optional: bool,
    missing_message: Option<&'static str>,
    fallback_message: Option<&'static str>,
}

impl TextSchema {
    /// `field` is a stable slug — `tag`, `password` — carried on every violation.
    pub const fn new(field: &'static str) -> Self {
        Self {
            field,
            normalize: Normalize {
                trim: false,
                lowercase: false,
                digits_only: false,
            },
            checks: &[],
            optional: false,
            missing_message: None,
            fallback_message: None,
        }
    }

    pub const fn trim(mut self) -> Self {
        self.normalize.trim = true;
        self
    }

    pub const fn lowercase(mut self) -> Self {
        self.normalize.lowercase = true;
        self
    }

    /// Throws away everything that is not an ASCII digit — what a phone field sends.
    pub const fn digits_only(mut self) -> Self {
        self.normalize.digits_only = true;
        self
    }

    /// An empty value passes and every other rule is skipped. Without it an empty
    /// value is [`ViolationKind::Missing`].
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub const fn missing_message(mut self, key: &'static str) -> Self {
        self.missing_message = Some(key);
        self
    }

    /// The message for any rule that named none.
    pub const fn otherwise(mut self, key: &'static str) -> Self {
        self.fallback_message = Some(key);
        self
    }

    pub const fn checks(mut self, checks: &'static [Check]) -> Self {
        self.checks = checks;
        self
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }

    /// What this schema would send to the server, with no judgement attached — a
    /// phone field can show the digits it kept while the number is still too short.
    pub fn normalize(&self, raw: &str) -> String {
        let mut value = raw.to_owned();
        if self.normalize.digits_only {
            value = value.chars().filter(char::is_ascii_digit).collect();
        }
        if self.normalize.trim {
            value = value.trim().to_owned();
        }
        if self.normalize.lowercase {
            value = value.to_lowercase();
        }
        value
    }

    /// The normalised value, or the FIRST rule it broke.
    ///
    /// First rather than all, because a field shows one message; a schema therefore
    /// orders its rules most-useful-first.
    pub fn check(&self, raw: &str) -> Result<String, Violation> {
        let value = self.normalize(raw);

        if value.is_empty() {
            return if self.optional {
                Ok(value)
            } else {
                Err(Violation::new(self.field, ViolationKind::Missing)
                    .with_message(self.missing_message.or(self.fallback_message)))
            };
        }

        for check in self.checks {
            if let Some(kind) = check.evaluate(&value) {
                return Err(Violation::new(self.field, kind)
                    .with_message(check.message.or(self.fallback_message)));
            }
        }

        Ok(value)
    }

    /// Every rule the value breaks, for a form that lists its problems at once.
    pub fn violations(&self, raw: &str) -> Vec<Violation> {
        let value = self.normalize(raw);

        if value.is_empty() {
            if self.optional {
                return Vec::new();
            }
            return vec![
                Violation::new(self.field, ViolationKind::Missing)
                    .with_message(self.missing_message.or(self.fallback_message)),
            ];
        }

        self.checks
            .iter()
            .filter_map(|check| {
                check.evaluate(&value).map(|kind| {
                    Violation::new(self.field, kind)
                        .with_message(check.message.or(self.fallback_message))
                })
            })
            .collect()
    }

    pub fn is_valid(&self, raw: &str) -> bool {
        self.check(raw).is_ok()
    }
}

/// Two fields that have to agree — a password and its repeat.
///
/// Compared AFTER normalisation by the schema that owns them, so a schema that
/// does not trim compares untrimmed.
pub fn confirm(schema: &TextSchema, value: &str, repeated: &str) -> Result<(), Violation> {
    if schema.normalize(value) == schema.normalize(repeated) {
        return Ok(());
    }
    Err(Violation::new(schema.field(), ViolationKind::Mismatch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: TextSchema = TextSchema::new("tag")
        .trim()
        .lowercase()
        .missing_message("tag_missing")
        .checks(&[
            Check::min_chars(3).message("tag_short"),
            Check::max_bytes(32).message("tag_long"),
            Check::charset(Charset::TAG).message("tag_alphabet"),
            Check::not_edged_with('_').message("tag_edge"),
            Check::no_run_of("__").message("tag_run"),
        ]);

    const PHONE: TextSchema = TextSchema::new("phone").digits_only().checks(&[
        Check::min_digits(7).message("phone_short"),
        Check::max_digits(15).message("phone_long"),
    ]);

    const NICKNAME: TextSchema = TextSchema::new("nickname")
        .trim()
        .optional()
        .checks(&[Check::max_chars(8)]);

    #[test]
    fn checking_returns_the_normalised_value_so_nothing_normalises_twice() {
        assert_eq!(TAG.check("  RiseOnly  ").unwrap(), "riseonly");
    }

    #[test]
    fn a_tag_under_three_characters_is_refused() {
        let violation = TAG.check("ab").unwrap_err();
        assert_eq!(
            violation.kind,
            ViolationKind::TooShort {
                minimum: 3,
                actual: 2,
                unit: Unit::Characters,
            }
        );
        assert_eq!(violation.message_key, Some("tag_short"));
    }

    /// The rule the user asked for by name: Cyrillic is not a tag.
    #[test]
    fn cyrillic_is_refused_and_the_offending_letter_is_named() {
        let violation = TAG.check("тег").unwrap_err();
        assert_eq!(violation.offending_char(), Some('т'));
        assert!(violation.offending_is_non_ascii());
        assert_eq!(violation.message_key, Some("tag_alphabet"));
    }

    #[test]
    fn an_underscore_is_legal_inside_and_refused_at_either_end() {
        assert!(TAG.check("rise_only").is_ok());

        let leading = TAG.check("_rise").unwrap_err();
        assert_eq!(
            leading.kind,
            ViolationKind::Edge {
                character: '_',
                edge: Edge::Leading
            }
        );

        let trailing = TAG.check("rise_").unwrap_err();
        assert_eq!(
            trailing.kind,
            ViolationKind::Edge {
                character: '_',
                edge: Edge::Trailing
            }
        );
    }

    #[test]
    fn a_doubled_underscore_is_refused() {
        let violation = TAG.check("ri__se").unwrap_err();
        assert_eq!(violation.kind, ViolationKind::Run { run: "__" });
    }

    /// tag-service bounds a tag in BYTES; a chars() bound would let the client accept
    /// what the server refuses.
    #[test]
    fn a_tag_is_bounded_in_bytes_because_that_is_what_the_server_counts() {
        assert!(TAG.check(&"a".repeat(32)).is_ok());
        let violation = TAG.check(&"a".repeat(33)).unwrap_err();
        assert!(matches!(
            violation.kind,
            ViolationKind::TooLong {
                maximum: 32,
                unit: Unit::Bytes,
                ..
            }
        ));
    }

    #[test]
    fn an_empty_required_value_is_missing_rather_than_short() {
        let violation = TAG.check("   ").unwrap_err();
        assert_eq!(violation.kind, ViolationKind::Missing);
        assert_eq!(violation.message_key, Some("tag_missing"));
    }

    #[test]
    fn an_optional_field_accepts_nothing_and_skips_every_other_rule() {
        assert_eq!(NICKNAME.check("").unwrap(), "");
        assert!(NICKNAME.check("waytoolongforthis").is_err());
    }

    #[test]
    fn a_phone_is_counted_in_digits_and_keeps_only_them() {
        assert_eq!(PHONE.check("+7 (707) 580-32-72").unwrap(), "77075803272");
        assert!(PHONE.check("123456").is_err());
        assert!(PHONE.check("1234567").is_ok());
        assert!(PHONE.check(&"1".repeat(16)).is_err());
    }

    /// The order of the rules is the order of the messages.
    #[test]
    fn the_first_broken_rule_is_the_one_reported() {
        let violation = TAG.check("_a").unwrap_err();
        assert!(matches!(violation.kind, ViolationKind::TooShort { .. }));
    }

    #[test]
    fn violations_lists_every_broken_rule_for_a_form_summary() {
        let all = TAG.violations("_т_");
        let kinds: Vec<_> = all.iter().map(|violation| &violation.kind).collect();
        assert!(kinds.len() >= 2, "expected several, got {kinds:?}");
        assert!(
            all.iter()
                .any(|v| matches!(v.kind, ViolationKind::Charset { .. }))
        );
        assert!(
            all.iter()
                .any(|v| matches!(v.kind, ViolationKind::Edge { .. }))
        );
    }

    #[test]
    fn a_rule_with_no_message_falls_back_to_the_schemas() {
        const LOOSE: TextSchema = TextSchema::new("loose")
            .otherwise("loose_problem")
            .checks(&[Check::min_chars(4)]);
        assert_eq!(
            LOOSE.check("ab").unwrap_err().message_key,
            Some("loose_problem")
        );
    }

    #[test]
    fn a_predicate_carries_its_slug() {
        const EVEN: TextSchema = TextSchema::new("even")
            .checks(&[Check::predicate("even_length", |value| {
                value.len() % 2 == 0
            })]);
        assert_eq!(
            EVEN.check("abc").unwrap_err().kind,
            ViolationKind::Failed {
                rule: "even_length"
            }
        );
        assert!(EVEN.check("abcd").is_ok());
    }

    #[test]
    fn confirmation_compares_after_normalisation() {
        const TRIMMED: TextSchema = TextSchema::new("password").trim();
        assert!(confirm(&TRIMMED, "secret", "  secret  ").is_ok());

        const RAW: TextSchema = TextSchema::new("password");
        assert!(confirm(&RAW, "secret", "  secret  ").is_err());
    }

    #[test]
    fn a_schema_reports_the_field_it_belongs_to() {
        assert_eq!(TAG.check("ab").unwrap_err().field, "tag");
        assert_eq!(TAG.field(), "tag");
    }
}
