use super::measurement_formatter::LocaleMeasurementSystem;

/// a unit with a linear converter to its base unit
pub trait Dimension: Copy + PartialEq + std::fmt::Debug {
    /// coefficient of the linear converter
    fn coefficient(self) -> f64;

    fn format_index(self) -> usize;

    fn preferred_unit(system: LocaleMeasurementSystem) -> Option<Self>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitLength {
    Meters,
    Kilometers,
    Feet,
    Miles,
}

impl Dimension for UnitLength {
    fn coefficient(self) -> f64 {
        match self {
            UnitLength::Meters => 1.0,
            UnitLength::Kilometers => 1000.0,
            UnitLength::Feet => 0.3048,
            UnitLength::Miles => 1609.344,
        }
    }

    fn format_index(self) -> usize {
        match self {
            UnitLength::Meters => 0,
            UnitLength::Kilometers => 1,
            UnitLength::Feet => 2,
            UnitLength::Miles => 3,
        }
    }

    fn preferred_unit(_system: LocaleMeasurementSystem) -> Option<Self> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitSpeed {
    KilometersPerHour,
    MilesPerHour,
}

impl Dimension for UnitSpeed {
    fn coefficient(self) -> f64 {
        match self {
            UnitSpeed::KilometersPerHour => 0.277778,
            UnitSpeed::MilesPerHour => 0.44704,
        }
    }

    fn format_index(self) -> usize {
        match self {
            UnitSpeed::KilometersPerHour => 4,
            UnitSpeed::MilesPerHour => 5,
        }
    }

    fn preferred_unit(system: LocaleMeasurementSystem) -> Option<Self> {
        Some(match system {
            LocaleMeasurementSystem::Metric => UnitSpeed::KilometersPerHour,
            LocaleMeasurementSystem::Us | LocaleMeasurementSystem::Uk => UnitSpeed::MilesPerHour,
        })
    }
}

/// a value with a unit
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement<U: Dimension> {
    pub value: f64,
    pub unit: U,
}

impl<U: Dimension> Measurement<U> {
    pub fn new(value: f64, unit: U) -> Self {
        Self { value, unit }
    }

    /// copy converted to another unit
    pub fn converted(&self, to: U) -> Self {
        if to == self.unit {
            return *self;
        }
        let base = self.value * self.unit.coefficient();
        Self { value: base / to.coefficient(), unit: to }
    }

    /// converts in place to another unit
    pub fn convert(&mut self, to: U) {
        *self = self.converted(to);
    }
}
