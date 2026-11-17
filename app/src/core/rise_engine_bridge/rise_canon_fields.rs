use std::collections::HashMap;
use std::panic::Location;
use std::sync::LazyLock;

use rise_canon::RiseRequestFieldAddress;
use rise_core::foundation::foundation_character::characters;
use rise_validate::TextSchema;

use crate::core::rise_engine_bridge::rise_canon_endpoints::RiseCanonEndpoints;

/// the canon text rule
pub type ValidationSchema = TextSchema;

/// canon request fields addressed by endpoint and pointer
/// each field has required flag and rejection codes and closed list and text rule
/// expanded to a dictionary once, no linear scan on the input path
pub enum RiseCanonFields {}

impl RiseCanonFields {
    #[track_caller]
    pub fn field_service_method(
        service: &str,
        method: &str,
        pointer: &str,
    ) -> Option<RiseRequestFieldAddress> {
        let endpoint = RiseCanonEndpoints::required(service, method).name;
        Self::field(endpoint, pointer)
    }

    /// field by canonical endpoint name
    /// pointer is normalized to the contract spelling
    /// a server rejection may carry an index where the contract has a star
    pub fn field(endpoint: &str, pointer: &str) -> Option<RiseRequestFieldAddress> {
        INDEX.get(&Self::key(endpoint, &Self::canonical_pointer(pointer))).copied()
    }

    /// a field that must exist, absence is a typo in the call and not a data state
    #[track_caller]
    pub fn required(endpoint: &str, pointer: &str) -> Option<RiseRequestFieldAddress> {
        let Some(address) = Self::field(endpoint, pointer) else {
            let location = Location::caller();
            debug_assert!(
                false,
                "field {pointer} of endpoint {endpoint} is missing from the riseonly-tools canon \
                 ({location})"
            );
            return None;
        };
        Some(address)
    }

    /// text validation rule for the field
    /// none means canon declared no bound so dont invent one
    pub fn schema_for(address: &RiseRequestFieldAddress) -> Option<ValidationSchema> {
        let profile = address.profile?;
        SCHEMAS_BY_FIELD.get(profile).copied()
    }

    pub fn schema_named(field: &str) -> Option<ValidationSchema> {
        SCHEMAS_BY_FIELD.get(field).copied()
    }

    pub fn schema(endpoint: &str, pointer: &str) -> Option<ValidationSchema> {
        Self::field(endpoint, pointer).and_then(|address| Self::schema_for(&address))
    }

    /// pointer spelling the contract uses, a resolved index becomes a star
    /// field keys are never numeric so an all digit segment is an index
    pub fn canonical_pointer(pointer: &str) -> String {
        if !characters(pointer).any(|character| character.is_number()) {
            return pointer.to_owned();
        }
        pointer
            .split('/')
            .map(|segment| {
                if segment.is_empty() || !characters(segment).all(|character| character.is_number())
                {
                    segment
                } else {
                    "*"
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    fn key(endpoint: &str, pointer: &str) -> String {
        format!("{endpoint}#{pointer}")
    }
}

static INDEX: LazyLock<HashMap<String, RiseRequestFieldAddress>> = LazyLock::new(|| {
    let mut index = HashMap::with_capacity(rise_canon::rise_request_fields::ALL.len());
    for address in rise_canon::rise_request_fields::ALL.iter() {
        index.entry(RiseCanonFields::key(address.endpoint, address.pointer)).or_insert(*address);
    }
    index
});

static SCHEMAS_BY_FIELD: LazyLock<HashMap<&'static str, ValidationSchema>> = LazyLock::new(|| {
    let mut schemas = HashMap::with_capacity(rise_validate::ALL.len());
    for schema in rise_validate::ALL.iter() {
        schemas.entry(schema.field()).or_insert(*schema);
    }
    schemas
});

pub trait RiseRequestFieldAddressCanon {
    fn schema(&self) -> Option<ValidationSchema>;

    fn is_closed_list(&self) -> bool;
}

impl RiseRequestFieldAddressCanon for RiseRequestFieldAddress {
    fn schema(&self) -> Option<ValidationSchema> {
        RiseCanonFields::schema_for(self)
    }

    fn is_closed_list(&self) -> bool {
        !self.allowed_values.is_empty()
    }
}
