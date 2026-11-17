#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type SaiApiFieldMap = HashMap<String, SaiApiFieldInfo>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaiApiFieldInfo {
    pub types: Vec<String>,
    pub is_optional: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaiApiVariant {
    pub id: String,
    pub status_code: i64,
    pub request_body_schema: SaiApiFieldMap,
    pub response_schema: SaiApiFieldMap,
    pub request_body_example: String,
    pub response_example: String,
    pub seen_count: i64,
    pub first_seen_ms: f64,
    pub last_seen_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaiApiEndpointDoc {
    pub id: String,
    pub transport: String,
    pub service: Option<String>,
    pub ws_method: Option<String>,
    pub http_method: Option<String>,
    pub path: String,
    pub variants: Vec<SaiApiVariant>,
    pub total_seen: i64,
    pub first_seen_ms: f64,
    pub last_seen_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaiApiDocsLibrary {
    pub endpoints: HashMap<String, SaiApiEndpointDoc>,
    pub schema_version: i64,
}

impl SaiApiDocsLibrary {
    pub fn new() -> Self {
        Self { endpoints: HashMap::new(), schema_version: 1 }
    }
}

impl Default for SaiApiDocsLibrary {
    fn default() -> Self {
        Self::new()
    }
}
