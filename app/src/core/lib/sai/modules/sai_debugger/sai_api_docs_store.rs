#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Task};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::AppPaths;

use super::sai_api_doc_analyzer::SaiApiDocAnalyzer;
use super::sai_api_doc_types::{SaiApiDocsLibrary, SaiApiEndpointDoc, SaiApiFieldMap, SaiApiVariant};
use super::sai_http_trace_entry::{SaiHttpTraceEntry, SaiTraceTransport};
use crate::core::config::app_environment::AppEnvironment;

static WRITE_LOCK: Mutex<()> = Mutex::new(());
static WRITE_GENERATION: AtomicU64 = AtomicU64::new(0);

pub struct SaiApiDocsStore {
    library: SaiApiDocsLibrary,
    processed_ids: HashSet<Uuid>,
    save_task: Option<Task<()>>,
}

struct SaiApiDocsUpsert {
    fingerprint: String,
    status_code: i64,
    request_schema: SaiApiFieldMap,
    response_schema: SaiApiFieldMap,
    request_example: String,
    response_example: String,
    now_ms: f64,
}

impl SaiApiDocsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn docs_file_url() -> Option<PathBuf> {
        AppPaths::resolve(AppEnvironment::current().canon())
            .ok()
            .map(|paths| paths.data_dir().join("sai_api_docs.json"))
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            library: SaiApiDocsLibrary::new(),
            processed_ids: HashSet::new(),
            save_task: None,
        };
        store.load_from_disk();
        store
    }

    pub fn library(&self) -> &SaiApiDocsLibrary {
        &self.library
    }

    pub fn process_entries(&mut self, entries: &[SaiHttpTraceEntry], cx: &mut Context<Self>) {
        let mut changed = false;
        for entry in entries {
            if entry.is_pending || !self.processed_ids.insert(entry.id) {
                continue;
            }

            if entry.is_rise_engine() {
                continue;
            }

            let Some(key) = SaiApiDocAnalyzer::endpoint_key(entry) else {
                continue;
            };

            let request_schema = SaiApiDocAnalyzer::infer_schema(
                &entry.request_body_wire,
                entry.transport == SaiTraceTransport::Websocket,
                SaiApiDocAnalyzer::DEFAULT_MAX_DEPTH,
            );
            let response_schema = SaiApiDocAnalyzer::infer_schema(
                &entry.response_body_preview,
                false,
                SaiApiDocAnalyzer::DEFAULT_MAX_DEPTH,
            );
            let fingerprint =
                SaiApiDocAnalyzer::variant_fingerprint(entry.response_status, &entry.response_body_preview);
            let now_ms = Date::now().time_interval_since_1970() * 1000.0;

            let doc = self.library.endpoints.entry(key.clone()).or_insert_with(|| {
                let (service, ws_method) = Self::endpoint_meta(entry);
                SaiApiEndpointDoc {
                    id: key.clone(),
                    transport: entry.transport.raw_value().to_owned(),
                    service,
                    ws_method,
                    http_method: (entry.transport == SaiTraceTransport::Http)
                        .then(|| entry.method.to_uppercase()),
                    path: Self::endpoint_path(entry),
                    variants: Vec::new(),
                    total_seen: 0,
                    first_seen_ms: now_ms,
                    last_seen_ms: now_ms,
                }
            });

            doc.total_seen += 1;
            doc.last_seen_ms = now_ms;

            Self::upsert_variant(
                doc,
                SaiApiDocsUpsert {
                    fingerprint,
                    status_code: entry.response_status,
                    request_schema,
                    response_schema,
                    request_example: entry.request_body_wire.clone(),
                    response_example: entry.response_body_preview.clone(),
                    now_ms,
                },
            );

            changed = true;
        }
        if changed {
            self.schedule_save(cx);
            cx.notify();
        }
    }

    pub fn clear_all(&mut self, cx: &mut Context<Self>) {
        self.library = SaiApiDocsLibrary::new();
        self.processed_ids.clear();
        drop(self.save_task.take());
        WRITE_GENERATION.fetch_add(1, Ordering::SeqCst);
        if let Some(url) = Self::docs_file_url() {
            let _guard = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = std::fs::remove_file(url);
        }
        cx.notify();
    }

    fn upsert_variant(doc: &mut SaiApiEndpointDoc, upsert: SaiApiDocsUpsert) {
        if let Some(variant) = doc.variants.iter_mut().find(|variant| variant.id == upsert.fingerprint) {
            variant.seen_count += 1;
            variant.last_seen_ms = upsert.now_ms;
            variant.request_body_schema =
                SaiApiDocAnalyzer::merge_schema(&variant.request_body_schema, &upsert.request_schema);
            variant.response_schema =
                SaiApiDocAnalyzer::merge_schema(&variant.response_schema, &upsert.response_schema);
        } else {
            doc.variants.push(SaiApiVariant {
                id: upsert.fingerprint,
                status_code: upsert.status_code,
                request_body_schema: upsert.request_schema,
                response_schema: upsert.response_schema,
                request_body_example: upsert.request_example,
                response_example: upsert.response_example,
                seen_count: 1,
                first_seen_ms: upsert.now_ms,
                last_seen_ms: upsert.now_ms,
            });
        }
    }

    fn load_from_disk(&mut self) {
        let Some(url) = Self::docs_file_url() else {
            return;
        };
        let Ok(data) = std::fs::read(url) else {
            return;
        };
        let Ok(loaded) = serde_json::from_slice::<SaiApiDocsLibrary>(&data) else {
            return;
        };
        self.library = loaded;
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |store, cx| store.write_to_disk(cx));
        }));
    }

    fn write_to_disk(&mut self, cx: &mut Context<Self>) {
        let Some(url) = Self::docs_file_url() else {
            return;
        };
        let library = self.library.clone();
        let generation = WRITE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        cx.background_spawn(async move {
            let Ok(data) = serde_json::to_vec_pretty(&library) else {
                return;
            };
            let _guard = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            if WRITE_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Some(parent) = url.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let staging = url.with_extension("json.tmp");
            if std::fs::write(&staging, data).is_ok() {
                let _ = std::fs::rename(&staging, &url);
            }
        })
        .detach();
    }

    fn endpoint_meta(entry: &SaiHttpTraceEntry) -> (Option<String>, Option<String>) {
        if entry.transport != SaiTraceTransport::Websocket {
            return (None, None);
        }
        match SaiApiDocAnalyzer::ws_service_and_method(&entry.request_body_wire) {
            Some((service, method)) => (Some(service), Some(method)),
            None => (None, None),
        }
    }

    fn endpoint_path(entry: &SaiHttpTraceEntry) -> String {
        match entry.transport {
            SaiTraceTransport::Websocket => {
                match SaiApiDocAnalyzer::ws_service_and_method(&entry.request_body_wire) {
                    Some((service, method)) => format!("{service}/{method}"),
                    None => entry.url.clone(),
                }
            }
            SaiTraceTransport::Http => entry.url.clone(),
        }
    }
}
