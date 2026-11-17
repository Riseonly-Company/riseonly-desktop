use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::RiseWireRequestMetadata;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_i18n::language_store::LanguageStore;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::NOTIFY_FAQ_PREFIX;
use serde::{Deserialize, Serialize};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyEndpoint;
use crate::modules::notify::engine::rise_notify_domain::RiseNotifyActionExecutor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseFAQStoreItem {
    pub id: i64,
    pub question: String,
    pub answer: String,
}

pub struct RiseFAQStore {
    items: Vec<RiseFAQStoreItem>,
    is_loading: bool,
    failed: bool,
    shown_language: Option<String>,
    _load: Option<Task<()>>,
}

impl RiseFAQStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { items: Vec::new(), is_loading: false, failed: false, shown_language: None, _load: None }
    }

    pub fn items(&self) -> &[RiseFAQStoreItem] {
        &self.items
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let language = LanguageStore::shared(cx).code().to_owned();
        if self.shown_language.as_deref() != Some(language.as_str()) {
            self.items = Self::cached(&language);
            self.shown_language = Some(language.clone());
            cx.notify();
        }
        if self.is_loading {
            return;
        }
        self.is_loading = true;
        self.failed = false;
        cx.notify();
        let domain = RiseNotifyActionExecutor::require_domain(cx);
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = async {
                let domain = domain.await?;
                let endpoint = RiseNotifyEndpoint::canonical(rise_canon::rise_wire_methods::notify::GET_FAQ, RiseEndpointKind::Read);
                let body = rise_canon::rise_request::notify::GetFaq { locale: Some(language.clone()) };
                let metadata = RiseWireRequestMetadata { auth_required: true, ..RiseWireRequestMetadata::default() };
                let response: rise_canon::rise_response::notify::GetFaq =
                    domain.engine.call_descriptor_response(&endpoint.descriptor(), &body, None, metadata).await?;
                anyhow::Ok(response)
            }
            .await;
            let _ = this.update(cx, |store, cx| store.finish_load(language, result, cx));
        });
        self._load = Some(task);
    }

    fn finish_load(
        &mut self,
        language: String,
        result: anyhow::Result<rise_canon::rise_response::notify::GetFaq>,
        cx: &mut Context<Self>,
    ) {
        self.is_loading = false;
        match result {
            Ok(response) => {
                let fresh: Vec<RiseFAQStoreItem> = response
                    .items
                    .into_iter()
                    .map(|item| RiseFAQStoreItem { id: item.id, question: item.question, answer: item.answer })
                    .collect();
                if self.shown_language.as_deref() == Some(language.as_str()) {
                    Self::store(&fresh, &language);
                    self.items = fresh;
                }
            }
            Err(error) => {
                Sai::log("FAQ", move || format!("load failed: {error}"), SaiLogKind::Network, cx);
                if self.items.is_empty() {
                    self.failed = true;
                }
            }
        }
        cx.notify();
    }

    fn key(language: &str) -> String {
        format!("{NOTIFY_FAQ_PREFIX}{language}")
    }

    fn cached(language: &str) -> Vec<RiseFAQStoreItem> {
        UserDefaults::standard()
            .data(&Self::key(language))
            .and_then(|data| serde_json::from_slice::<Vec<RiseFAQStoreItem>>(&data).ok())
            .unwrap_or_default()
    }

    fn store(items: &[RiseFAQStoreItem], language: &str) {
        let Ok(data) = serde_json::to_vec(items) else { return };
        UserDefaults::standard().set(&Self::key(language), data);
    }
}
