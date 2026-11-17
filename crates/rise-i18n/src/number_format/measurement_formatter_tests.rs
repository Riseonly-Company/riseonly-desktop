use super::measurement::{Measurement, UnitLength, UnitSpeed};
use super::measurement_formatter::{
    FormatterUnitStyle, LocaleMeasurementSystem, MeasurementFormatter, MeasurementFormatterUnitOptions, NumberFormatter,
};
use crate::language_store::Locale;

fn formatter(identifier: &str, options: MeasurementFormatterUnitOptions, style: FormatterUnitStyle, digits: u32) -> MeasurementFormatter {
    MeasurementFormatter {
        locale: Locale::new(identifier),
        unit_options: options,
        unit_style: style,
        number_formatter: NumberFormatter { maximum_fraction_digits: digits },
    }
}

fn distance(identifier: &str) -> MeasurementFormatter {
    formatter(identifier, MeasurementFormatterUnitOptions::PROVIDED_UNIT, FormatterUnitStyle::Medium, 1)
}

fn speed(identifier: &str) -> MeasurementFormatter {
    formatter(identifier, MeasurementFormatterUnitOptions::NATURAL_SCALE, FormatterUnitStyle::Short, 0)
}

#[test]
fn provided_unit_medium_matches_foundation() {
    assert_eq!(distance("ru_RU").string_from(&Measurement::new(1.5, UnitLength::Kilometers)), "1,5 \u{43a}\u{43c}");
    assert_eq!(distance("en_US").string_from(&Measurement::new(0.3, UnitLength::Miles)), "0.3 mi");
    assert_eq!(distance("en_US").string_from(&Measurement::new(800.0, UnitLength::Feet)), "800 ft");
    assert_eq!(distance("pl_PL").string_from(&Measurement::new(1234.5, UnitLength::Kilometers)), "1234,5 km");
}

#[test]
fn plural_form_without_number_matches_foundation() {
    assert_eq!(distance("ar_SA").string_from(&Measurement::new(2.0, UnitLength::Meters)), "\u{645}\u{62a}\u{631}\u{627}\u{646}");
}

#[test]
fn long_style_follows_fraction_digits() {
    let long = |identifier: &str| formatter(identifier, MeasurementFormatterUnitOptions::PROVIDED_UNIT, FormatterUnitStyle::Long, 1);
    assert_eq!(
        long("ru_RU").string_from(&Measurement::new(5.0, UnitLength::Kilometers)),
        "5 \u{43a}\u{438}\u{43b}\u{43e}\u{43c}\u{435}\u{442}\u{440}\u{43e}\u{432}"
    );
    assert_eq!(long("sr_RS").string_from(&Measurement::new(0.2, UnitLength::Meters)), "0,2 \u{43c}\u{435}\u{442}\u{440}\u{430}");
}

#[test]
fn natural_scale_speed_follows_measurement_system() {
    assert_eq!(speed("en_US").string_from(&Measurement::new(100.0, UnitSpeed::KilometersPerHour)), "62mph");
    assert_eq!(speed("ru_RU").string_from(&Measurement::new(21.0, UnitSpeed::KilometersPerHour)), "21 \u{43a}\u{43c}/\u{447}");
}

#[test]
fn measurement_system_by_region() {
    assert_eq!(Locale::new("en_US").measurement_system(), LocaleMeasurementSystem::Us);
    assert_eq!(Locale::new("en_GB").measurement_system(), LocaleMeasurementSystem::Uk);
    assert_eq!(Locale::new("ru_US").measurement_system(), LocaleMeasurementSystem::Us);
    assert_eq!(Locale::new("zh_Hans_CN").measurement_system(), LocaleMeasurementSystem::Metric);
    assert_eq!(Locale::new("es_419").measurement_system(), LocaleMeasurementSystem::Metric);
    assert_eq!(Locale::new("ru").measurement_system(), LocaleMeasurementSystem::Metric);
}

#[test]
fn converted_uses_foundation_coefficients() {
    let miles = Measurement::new(1609.344, UnitLength::Meters).converted(UnitLength::Miles);
    assert_eq!(miles.unit, UnitLength::Miles);
    assert!((miles.value - 1.0).abs() < 1e-12);
}
