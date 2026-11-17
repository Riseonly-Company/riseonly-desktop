use std::collections::HashMap;
use std::panic::Location;
use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use rise_core::foundation::foundation_character::characters;
use rise_endpoints::{ENDPOINTS, EndpointKind, SERVICES, TIMEOUT_DEFAULT_MS, TimeoutSource};

pub type RiseEndpoint = rise_endpoints::Endpoint;
pub type RiseEndpointService = rise_endpoints::Service;

/// backend method catalog from canon, service and method and kind and deadline
/// descriptors take all but the method name from here, timeout and retry never typed by hand
/// unknown name traps in debug
pub enum RiseCanonEndpoints {}

impl RiseCanonEndpoints {
    pub fn is_known(service: &str, method: &str) -> bool {
        INDEX.contains_key(&Self::key(service, method))
    }

    pub fn is_known_on_wire(service: &str, method: &str) -> bool {
        if Self::is_known(service, method) {
            return true;
        }
        if !SERVICES_BY_NAME.get(service).is_some_and(|entry| entry.accepts_alternate_case) {
            return false;
        }
        Self::is_known(service, &Self::snake_cased(method))
            || Self::is_known(service, &Self::camel_cased(method))
    }

    fn snake_cased(method: &str) -> String {
        let mut result = String::new();
        for character in characters(method) {
            if character.is_uppercase() {
                result.push('_');
                result.push_str(&character.lowercased());
            } else {
                result.push_str(character.as_str());
            }
        }
        result
    }

    fn camel_cased(method: &str) -> String {
        let mut parts = method.split('_').filter(|part| !part.is_empty());
        let Some(first) = parts.next() else {
            return method.to_owned();
        };
        parts.fold(first.to_owned(), |mut result, part| {
            result.push_str(&Self::capitalized(part));
            result
        })
    }

    fn capitalized(word: &str) -> String {
        let mut result = String::new();
        for (position, character) in characters(word).enumerate() {
            if position == 0 {
                result.push_str(&character.uppercased());
            } else {
                result.push_str(&character.lowercased());
            }
        }
        result
    }

    /// wire method name of a catalog constant
    /// returns the canon spelling and not the argument
    /// a case mismatch is the typo these constants catch
    #[track_caller]
    pub fn method(service: &str, method: &str) -> &'static str {
        Self::required(service, method).method
    }

    /// wire service name, checked on its own since a
    /// misspelled service reaches the gateway just as silently
    #[track_caller]
    pub fn service(name: &str) -> String {
        let Some(entry) = SERVICES_BY_NAME.get(name) else {
            let location = Location::caller();
            debug_assert!(false, "service {name} is missing from the canon catalog ({location})");
            return name.to_owned();
        };
        entry.name.to_owned()
    }

    #[track_caller]
    pub fn required(service: &str, method: &str) -> &'static RiseEndpoint {
        let key = Self::key(service, method);
        if let Some(entry) = INDEX.get(&key) {
            return entry;
        }
        let location = Location::caller();
        debug_assert!(
            false,
            "method {service}.{method} is missing from the canon catalog ({location})"
        );
        let mut fallbacks = FALLBACKS.lock();
        if let Some(entry) = fallbacks.get(&key) {
            return entry;
        }
        let entry: &'static RiseEndpoint = Box::leak(Box::new(RiseEndpoint {
            name: Box::leak(format!("{service}_{method}").into_boxed_str()),
            service: Box::leak(service.to_owned().into_boxed_str()),
            method: Box::leak(method.to_owned().into_boxed_str()),
            kind: EndpointKind::Mutation,
            timeout_ms: TIMEOUT_DEFAULT_MS,
            timeout_source: TimeoutSource::Catalog,
            idempotency_field: None,
            catalog: None,
            server_route: false,
            http_route: false,
            request_policy: None,
        }));
        fallbacks.insert(key, entry);
        entry
    }

    fn key(service: &str, method: &str) -> String {
        format!("{service}.{method}")
    }
}

static INDEX: LazyLock<HashMap<String, &'static RiseEndpoint>> = LazyLock::new(|| {
    let mut index = HashMap::with_capacity(ENDPOINTS.len());
    for entry in ENDPOINTS.iter() {
        index.entry(RiseCanonEndpoints::key(entry.service, entry.method)).or_insert(entry);
    }
    index
});

static SERVICES_BY_NAME: LazyLock<HashMap<&'static str, &'static RiseEndpointService>> =
    LazyLock::new(|| {
        let mut services = HashMap::with_capacity(SERVICES.len());
        for entry in SERVICES.iter() {
            services.entry(entry.name).or_insert(entry);
        }
        services
    });

static FALLBACKS: LazyLock<Mutex<HashMap<String, &'static RiseEndpoint>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub trait RiseEndpointClientTimeout {
    /// client response deadline in the form the engine expects
    fn client_timeout(&self) -> Duration;
}

impl RiseEndpointClientTimeout for RiseEndpoint {
    fn client_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.timeout_ms))
    }
}
