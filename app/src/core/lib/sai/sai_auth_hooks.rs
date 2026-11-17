use futures::future::LocalBoxFuture;
use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct SaiAuthHooks {
    pub access_token: Box<dyn Fn(&App) -> Option<String> + Send + Sync>,
    pub is_expired: Box<dyn Fn(&str, f64) -> bool + Send + Sync>,
    pub extract_user_id: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
    pub refresh_access_token:
        Box<dyn Fn(&mut App) -> LocalBoxFuture<'static, anyhow::Result<()>> + Send + Sync>,
    pub ensure_valid_access_token:
        Box<dyn Fn(&mut App) -> LocalBoxFuture<'static, anyhow::Result<()>> + Send + Sync>,
    pub handle_auth_error: Box<dyn Fn(&mut App) + Send + Sync>,
    pub is_auth_error: Box<dyn Fn(Option<&str>, Option<&str>) -> bool + Send + Sync>,
    pub note_authorized_reply: Box<dyn Fn(&mut App) + Send + Sync>,
    pub handle_account_blocked: Box<dyn Fn(&mut App) + Send + Sync>,
    pub is_premium: Box<dyn Fn(&App) -> bool + Send + Sync>,
    pub effective_user_id_for_wire: Box<dyn Fn(&App) -> Option<String> + Send + Sync>,
    pub debug_first_account: Box<dyn Fn(&str, &App) -> Option<String> + Send + Sync>,
    pub debug_detach_active_account: Box<dyn Fn(&mut App) + Send + Sync>,
    pub active_account_id: Box<dyn Fn(&App) -> Option<String> + Send + Sync>,
    pub switch_account: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub load_account_backend_id: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

pub(crate) static SAI_AUTH_HOOKS: FeatureHookSlot<SaiAuthHooks> = FeatureHookSlot::new("SAI_AUTH_HOOKS");

/// installed once, a second install keeps the first
pub fn set_sai_auth_hooks(hooks: SaiAuthHooks) {
    let _ = SAI_AUTH_HOOKS.set(hooks);
}

pub fn sai_auth_hooks() -> Option<&'static SaiAuthHooks> {
    SAI_AUTH_HOOKS.get()
}

/// stored access token trimmed, none when signed out or auth isnt installed
pub(crate) fn sai_auth_access_token(cx: &App) -> Option<String> {
    let hooks = sai_auth_hooks()?;
    let token = (hooks.access_token)(cx)?;
    let trimmed = trimming_whitespaces_and_newlines(&token);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// true if the token is expired, without auth nothing is ever expired
pub fn sai_auth_is_expired(token: &str, leeway_seconds: f64) -> bool {
    sai_auth_hooks().is_some_and(|hooks| (hooks.is_expired)(token, leeway_seconds))
}

/// refreshes the access token, fails like a signed out refresh when auth is missing
pub fn sai_auth_refresh_access_token(
    cx: &mut App,
) -> LocalBoxFuture<'static, anyhow::Result<()>> {
    match sai_auth_hooks() {
        Some(hooks) => (hooks.refresh_access_token)(cx),
        None => Box::pin(async { Err(anyhow::Error::new(SaiAuthNotSignedIn)) }),
    }
}

/// handles an auth error, nothing to sign out without auth
pub(crate) fn sai_auth_handle_auth_error(cx: &mut App) {
    if let Some(hooks) = sai_auth_hooks() {
        (hooks.handle_auth_error)(cx);
    }
}

/// confirms the active session after a reply the server accepted, nothing without auth
pub(crate) fn sai_auth_note_authorized_reply(cx: &mut App) {
    if let Some(hooks) = sai_auth_hooks() {
        (hooks.note_authorized_reply)(cx);
    }
}

/// refresh answer when auth is missing, no session to refresh
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("no signed-in session")]
pub(crate) struct SaiAuthNotSignedIn;
