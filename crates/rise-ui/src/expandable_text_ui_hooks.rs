use std::sync::OnceLock;

use gpui::{App, Window};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_range::NSRange;

#[derive(Clone, Debug, PartialEq)]
pub enum ExpandableTextLinkTokenKind {
    Mention(Url),
    Hashtag(String),
}

/// a mention or tag of a caption with its range in utf16 units
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandableTextLinkToken {
    pub kind: ExpandableTextLinkTokenKind,
    pub range: NSRange,
}

pub struct ExpandableTextUiHooks {
    pub caption_link_tokens: fn(text: &str) -> Vec<ExpandableTextLinkToken>,
    pub hashtag_url: fn(tag: &str) -> Option<Url>,
    pub hashtag_from_url: fn(url: &Url) -> Option<String>,
    pub route_link: fn(url: &Url, window: &mut Window, cx: &mut App),
}

static EXPANDABLE_TEXT_UI_HOOKS: OnceLock<ExpandableTextUiHooks> = OnceLock::new();

/// installs the hooks once, a second call keeps the first installer hooks
pub fn set_expandable_text_ui_hooks(hooks: ExpandableTextUiHooks) {
    let _ = EXPANDABLE_TEXT_UI_HOOKS.set(hooks);
}

pub fn expandable_text_ui_hooks() -> Option<&'static ExpandableTextUiHooks> {
    EXPANDABLE_TEXT_UI_HOOKS.get()
}
