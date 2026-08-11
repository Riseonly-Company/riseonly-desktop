use crate::violation::Violation;

/// What a whole form was refused for, in the order the caller checked: the first
/// entry is the field a wizard should send the user back to.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    violations: Vec<Violation>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the outcome of one field and forgets the value: takes the `Result` a
    /// schema returned, so a caller writes `report.field(TAG.check(tag))`.
    pub fn field<T>(mut self, outcome: Result<T, Violation>) -> Self {
        if let Err(violation) = outcome {
            self.violations.push(violation);
        }
        self
    }

    pub fn push(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }

    /// The one to act on: the first field that failed.
    pub fn first(&self) -> Option<&Violation> {
        self.violations.first()
    }

    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    pub fn for_field(&self, field: &str) -> Option<&Violation> {
        self.violations
            .iter()
            .find(|violation| violation.field == field)
    }

    pub fn into_result(self) -> Result<(), Violation> {
        match self.violations.into_iter().next() {
            Some(violation) => Err(violation),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Check, TextSchema};

    const NAME: TextSchema = TextSchema::new("name")
        .trim()
        .checks(&[Check::min_chars(2), Check::max_chars(64)]);
    const CODE: TextSchema = TextSchema::new("code").trim().checks(&[
        Check::min_chars(4),
        Check::max_chars(4),
        Check::charset(crate::Charset::DIGITS),
    ]);

    #[test]
    fn a_form_with_nothing_wrong_reports_nothing() {
        let report = Report::new()
            .field(NAME.check("Ян"))
            .field(CODE.check("1234"));
        assert!(report.is_ok());
        assert!(report.into_result().is_ok());
    }

    /// The order the caller checked in has to survive; a map keyed by field would lose it.
    #[test]
    fn the_first_failure_is_the_first_field_checked() {
        let report = Report::new().field(NAME.check("")).field(CODE.check("12"));

        assert!(!report.is_ok());
        assert_eq!(
            report.first().map(|violation| violation.field),
            Some("name")
        );
        assert_eq!(report.violations().len(), 2);
    }

    #[test]
    fn a_field_can_be_looked_up_by_name() {
        let report = Report::new()
            .field(NAME.check("Ян"))
            .field(CODE.check("abcd"));
        assert!(report.for_field("name").is_none());
        assert!(report.for_field("code").is_some());
    }
}
