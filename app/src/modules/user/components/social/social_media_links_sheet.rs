use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, Point, RenderOnce,
    prelude::FluentBuilder,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::{hstack, spacer_min_h, spacer_min_w, vstack};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::modules::user::components::social::social_icon_badge::SocialIconBadge;
use crate::modules::user::shared::social_platform::SocialLink;

const ACTION_OPEN: &str = "open";
const ACTION_COPY: &str = "copy";

struct SocialMediaLinksSheetState {
    _hold_context_menu: Subscription,
}

impl SocialMediaLinksSheetState {
    fn new(cx: &mut Context<Self>) -> Self {
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self { _hold_context_menu: cx.observe(&hold_context_menu, |_, _, cx| cx.notify()) }
    }
}

#[derive(IntoElement)]
pub struct SocialMediaLinksSheet {
    id: ElementId,
    pub links: Vec<SocialLink>,
    hugs_content: bool,
}

impl SocialMediaLinksSheet {
    pub fn new(id: impl Into<ElementId>, links: Vec<SocialLink>) -> Self {
        Self { id: id.into(), links, hugs_content: false }
    }

    /// desktop modal fits this column so it keeps natural height,
    /// shrinks and scrolls past the window ceiling
    pub fn hugs_content(mut self, hugs: bool) -> Self {
        self.hugs_content = hugs;
        self
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name.to_owned()))
    }

    fn row(&self, link: &SocialLink, theme: &AppTheme) -> HoldContextMenuContainer {
        let content_link = link.clone();
        let content_theme = theme.clone();
        let action_link = link.clone();
        let tap_link = link.clone();
        let actions = vec![
            HoldContextMenuAction::new(ACTION_OPEN, tr("social_link_open")).system_image("safari"),
            HoldContextMenuAction::new(ACTION_COPY, tr("copy")).system_image("doc.on.doc"),
        ];
        HoldContextMenuContainer::new(
            self.child_id(&format!("link-{}", link.id())),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                Self::row_content(&content_link, &content_theme).into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| match action.as_str() {
            ACTION_OPEN => Self::open(&action_link, cx),
            ACTION_COPY => Self::copy(&action_link.url, cx),
            _ => {}
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            Self::open(&tap_link, cx);
        }))
    }

    fn row_content(link: &SocialLink, theme: &AppTheme) -> Div {
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(11.0))
            .bg(theme.bg._200)
            .rounded(px(14.0))
            .overflow_hidden()
            .child(SocialIconBadge::new(link.platform).size(40.0))
            .child(
                vstack(2.0)
                    .items_start()
                    .min_w_0()
                    .child(
                        MainText::new(link.title.clone())
                            .px(15.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(link.display_url())
                            .px(13.0)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
            .child(spacer_min_w(8.0))
            .children(IconUi::sized("arrow.up.right", px(13.0), theme.text.secondary, false))
    }

    fn open(link: &SocialLink, cx: &mut App) {
        let Some(url) = Url::from_string(&link.url) else {
            return;
        };
        InAppLinkBrowser::route(&url, cx);
    }

    fn copy(value: &str, cx: &mut App) {
        UIPasteboard::general().set_string(value, cx);
        Notifier::copied(cx);
    }
}

impl RenderOnce for SocialMediaLinksSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let _state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| SocialMediaLinksSheetState::new(cx));
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let menu_is_open = hold_context_menu.read(cx).active_request().is_some();
        let rows: Vec<_> = self.links.iter().map(|link| div().w_full().child(self.row(link, &theme))).collect();

        let hugs_content = self.hugs_content;
        let list = vstack(8.0).w_full().px(px(12.0)).children(rows);
        let scroll = if hugs_content {
            let column = div()
                .id(self.child_id("scroll"))
                .flex()
                .flex_col()
                .w_full()
                .h_auto()
                .min_h_0()
                .flex_shrink(1.0);
            let column = if menu_is_open { column.overflow_y_hidden() } else { column.overflow_y_scroll() };
            column.child(list).child(spacer_min_h(28.0)).into_any_element()
        } else {
            scroll_view(self.child_id("scroll"))
                .w_full()
                .flex_1()
                .scroll_disabled(menu_is_open)
                .shows_indicators(false)
                .child(list)
                .child(spacer_min_h(28.0))
                .into_any_element()
        };

        vstack(0.0)
            .map(|column| if hugs_content { column.w_full().min_h_0() } else { column.size_full() })
            .bg(theme.bg._100)
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .px(px(16.0))
                    .py(px(14.0))
                    .child(MainText::new(tr("social_links_title")).px(17.0).font_weight(FontWeight::SEMIBOLD))
                    .child(spacer_min_w(0.0)),
            )
            .child(scroll)
    }
}
