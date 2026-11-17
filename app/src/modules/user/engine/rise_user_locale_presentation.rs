use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::LanguageStore;

use crate::modules::user::engine::core::rise_user_locale_wire_models::RiseLocaleSource;
use crate::modules::user::engine::rise_user_locale_repository::RiseUserLocaleSnapshot;

/// immutable ui snapshot of the accounts locale prefs,
/// gated on account,
/// epoch and session
pub struct RiseUserLocalePresentation {
    snapshot: RiseUserLocaleSnapshot,
    generation: u64,
    pending_region: Option<String>,
    pending_language: Option<String>,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseUserLocalePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseUserLocaleSnapshot::empty(),
            generation: 0,
            pending_region: None,
            pending_language: None,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseUserLocaleSnapshot {
        &self.snapshot
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pending_region(&self) -> Option<&str> {
        self.pending_region.as_deref()
    }

    pub fn pending_language(&self) -> Option<&str> {
        self.pending_language.as_deref()
    }

    pub fn mark_pending_region(&mut self, region: String, cx: &mut Context<Self>) {
        self.pending_region = Some(region);
        cx.notify();
    }

    pub fn mark_pending_language(&mut self, language: String, cx: &mut Context<Self>) {
        self.pending_language = Some(language);
        cx.notify();
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
        {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseUserLocaleSnapshot::empty();
        self.pending_region = None;
        self.pending_language = None;
        if cx.has_global::<LanguageStore>() {
            LanguageStore::update(cx, |store, cx| store.bind(Some(account_id), cx));
        }
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseUserLocaleSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next == self.snapshot {
            return true;
        }
        if next.has_settings
            && !next.is_mutating
            && next.settings.app_language_source != RiseLocaleSource::Unset
            && !next.settings.app_language.is_empty()
            && cx.has_global::<LanguageStore>()
        {
            let language = next.settings.app_language.clone();
            let server_explicit = next.settings.app_language_source == RiseLocaleSource::Explicit;
            LanguageStore::update(cx, |store, cx| {
                store.apply_account_language(&language, server_explicit, cx);
            });
        }
        if !next.is_mutating {
            if self.pending_region.as_deref() == Some(next.settings.region_code.as_str()) {
                self.pending_region = None;
            }
            if self.pending_language.as_deref() == Some(next.settings.app_language.as_str()) {
                self.pending_language = None;
            }
            if next.error.is_some() {
                self.pending_region = None;
                self.pending_language = None;
            }
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseUserLocaleSnapshot::empty();
        self.pending_region = None;
        self.pending_language = None;
        if cx.has_global::<LanguageStore>() {
            LanguageStore::update(cx, |store, cx| store.bind(None, cx));
        }
        cx.notify();
    }
}
