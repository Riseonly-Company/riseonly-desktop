use rise_canon::RiseWireKey;

/// camelcase spelling of a canon key, the one the old codable wrote
/// for stores that patch rows by camel names and encoders that convert to snake case
/// rise reads either spelling back
pub trait RiseWireKeyCamel {
    fn camel(key: &str) -> String;
}

impl RiseWireKeyCamel for RiseWireKey {
    fn camel(key: &str) -> String {
        RiseWireKey::foundation_camel(key)
    }
}
