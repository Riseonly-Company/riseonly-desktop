use rise_core::foundation::foundation_url::Url;
use rise_ui::expandable_text_ui_hooks::{
    ExpandableTextLinkToken, ExpandableTextLinkTokenKind, ExpandableTextUiHooks, set_expandable_text_ui_hooks,
};

use gpui::{App, Window};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextCaptionLinkTokenKind};

fn caption_link_tokens(text: &str) -> Vec<ExpandableTextLinkToken> {
    FormattedText::caption_link_tokens(text)
        .into_iter()
        .map(|token| ExpandableTextLinkToken {
            kind: match token.kind {
                FormattedTextCaptionLinkTokenKind::Mention(url) => ExpandableTextLinkTokenKind::Mention(url),
                FormattedTextCaptionLinkTokenKind::Hashtag(tag) => ExpandableTextLinkTokenKind::Hashtag(tag),
            },
            range: token.range,
        })
        .collect()
}

fn route_link(url: &Url, _window: &mut Window, cx: &mut App) {
    InAppLinkBrowser::route(url, cx);
}

pub fn install_expandable_text_ui_hooks() {
    set_expandable_text_ui_hooks(ExpandableTextUiHooks {
        caption_link_tokens,
        hashtag_url: FormattedText::hashtag_url,
        hashtag_from_url: FormattedText::hashtag,
        route_link,
    });
}
