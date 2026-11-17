use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, SharedString, Window};
use rise_canon::RiseValidation;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::lib::canon_rules::RiseValidationCanonRules;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::components::social::social_icon_badge::SocialIconBadge;
use crate::modules::user::shared::profile_field_rules::ProfileFieldRules;
use crate::modules::user::shared::social_platform::{SocialLink, SocialPlatform};
use crate::modules::user::stores::user::user_actions::user_types::ProfileSocialLink;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

struct EditableLink {
    id: String,
    title: String,
    url: String,
    presence: Presence<()>,
    is_removed: bool,
}

struct SocialLinksEditorState {
    items: Vec<EditableLink>,
    initial_normalized: Vec<ProfileSocialLink>,
    did_hydrate: bool,
}

impl SocialLinksEditorState {
    fn new() -> Self {
        Self { items: Vec::new(), initial_normalized: Vec::new(), did_hydrate: false }
    }

    fn live_items(&self) -> impl Iterator<Item = &EditableLink> {
        self.items.iter().filter(|item| !item.is_removed)
    }

    fn profile_links(&self) -> Vec<ProfileSocialLink> {
        self.live_items().map(|item| ProfileSocialLink { title: item.title.clone(), url: item.url.clone() }).collect()
    }

    fn can_commit(&self) -> bool {
        let has_incomplete_item = self.live_items().any(|item| {
            trimming_whitespaces_and_newlines(&item.title).is_empty()
                || trimming_whitespaces_and_newlines(&item.url).is_empty()
        });
        !has_incomplete_item && SocialLink::normalize(&self.profile_links()).len() == self.live_items().count()
    }

    fn hydrate(&mut self, existing: &[ProfileSocialLink]) {
        if self.did_hydrate {
            return;
        }
        self.did_hydrate = true;
        self.items = existing
            .iter()
            .map(|link| {
                let title = trimming_whitespaces_and_newlines(&link.title);
                let platform = SocialPlatform::detect(&link.url);
                EditableLink {
                    id: Uuid::new_v4().uuid_string(),
                    title: if title.is_empty() { platform.display_name() } else { title.to_owned() },
                    url: link.url.clone(),
                    presence: Presence::new(Some(())).with_transition(Transition::opacity()),
                    is_removed: false,
                }
            })
            .collect();
        self.initial_normalized = SocialLink::normalize(existing);
    }

    fn prune_finished_removals(&mut self) {
        let now = RiseClock::monotonic();
        self.items.retain(|item| !item.is_removed || item.presence.is_running(now));
    }

    fn append(&mut self, cx: &mut Context<Self>) {
        self.prune_finished_removals();
        let mut presence = Presence::new(None).with_transition(Transition::opacity());
        presence.set_in(Some(()), SocialLinksEditor::row_animation(), RiseClock::monotonic(), cx);
        self.items.push(EditableLink {
            id: Uuid::new_v4().uuid_string(),
            title: String::new(),
            url: String::new(),
            presence,
            is_removed: false,
        });
        cx.notify();
    }

    fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.prune_finished_removals();
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.is_removed = true;
            item.presence.set_in(None, SocialLinksEditor::row_animation(), RiseClock::monotonic(), cx);
        }
        cx.notify();
    }

    fn item(&self, id: &str) -> Option<&EditableLink> {
        self.items.iter().find(|item| item.id == id)
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut EditableLink> {
        self.items.iter_mut().find(|item| item.id == id)
    }
}

#[derive(IntoElement)]
pub struct SocialLinksEditor;

impl SocialLinksEditor {
    pub fn new() -> Self {
        Self
    }

    fn row_animation() -> MotionSpec {
        spring(0.34, 0.9)
    }

    fn max_links() -> usize {
        RiseValidation::collection_limit(rise_canon::rise_key::user::SOCIAL_LINKS).maximum
    }

    fn max_title_length() -> usize {
        ProfileFieldRules::social_link_title().maximum().unwrap_or(0)
    }

    fn title_length(title: &str) -> usize {
        let max_title_length = Self::max_title_length();
        max_title_length - ProfileFieldRules::social_link_title().remaining(title).unwrap_or(max_title_length)
    }

    fn title_binding(state: &Entity<SocialLinksEditorState>, id: &str) -> Binding<String> {
        let reader = state.clone();
        let read_id = id.to_owned();
        let writer = state.clone();
        let write_id = id.to_owned();
        Binding::new(
            move |cx: &App| reader.read(cx).item(&read_id).map(|item| item.title.clone()).unwrap_or_default(),
            move |title, _, cx| {
                writer.update(cx, |state, cx| {
                    if let Some(item) = state.item_mut(&write_id) {
                        item.title = title;
                    }
                    cx.notify();
                });
            },
        )
    }

    fn url_binding(state: &Entity<SocialLinksEditorState>, id: &str) -> Binding<String> {
        let reader = state.clone();
        let read_id = id.to_owned();
        let writer = state.clone();
        let write_id = id.to_owned();
        Binding::new(
            move |cx: &App| reader.read(cx).item(&read_id).map(|item| item.url.clone()).unwrap_or_default(),
            move |url, _, cx| {
                writer.update(cx, |state, cx| {
                    if let Some(item) = state.item_mut(&write_id) {
                        item.url = url;
                    }
                    cx.notify();
                });
            },
        )
    }

    /// tab_order keeps the stops of one row together,
    /// remove button gets a higher index so it comes after both fields
    fn row(state: &Entity<SocialLinksEditorState>, item: &EditableLink, tab_order: isize, theme: &AppTheme) -> Div {
        let id = item.id.as_str();
        let platform = SocialPlatform::detect(&item.url);
        let remove_state = state.clone();
        let remove_id = id.to_owned();
        let remove_action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            remove_state.update(cx, |state, cx| state.remove(&remove_id, cx));
        });

        let title_row = hstack(12.0)
            .w_full()
            .px(px(16.0))
            .min_h(px(58.0))
            .child(
                div()
                    .flex_none()
                    .size(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("textformat", px(17.0), theme.text.secondary, false)),
            )
            .child(
                div().flex_1().min_w_0().child(
                    ShapedTextField::new(
                        SharedString::from(format!("social-link-title-{id}")),
                        tr("social_link_title_placeholder"),
                        ProfileFieldRules::social_link_title().binding(Self::title_binding(state, id)),
                    )
                    .font_size(16.0)
                    .text_color(theme.text.primary)
                    .on_submit(|window, cx| window.focus_next(cx))
                    .on_cancel(|window, cx| ScreenFocus::release_text_field(window, cx)),
                ),
            )
            .child(
                MainText::new(format!("{}/{}", Self::title_length(&item.title), Self::max_title_length()))
                    .px(11.0)
                    .color(theme.text.secondary),
            )
            .child(
                div()
                    .id(SharedString::from(format!("social-link-remove-hint-{id}")))
                    .aria_label(tr("social_link_remove"))
                    .tooltip_label(tr("social_link_remove"))
                    .child(
                        button_chrome(
                            div().id(SharedString::from(format!("social-link-remove-{id}"))),
                            false,
                            Some(remove_action),
                        )
                        .tab_index(1)
                        .children(IconUi::sized(
                            "xmark.circle.fill",
                            px(20.0),
                            theme.text.secondary.opacity(0.72),
                            true,
                        )),
                    ),
            );

        let url_row = hstack(12.0)
            .w_full()
            .px(px(16.0))
            .min_h(px(58.0))
            .child(SocialIconBadge::new(platform).size(30.0))
            .child(
                div().flex_1().min_w_0().child(
                    ShapedTextField::new(
                        SharedString::from(format!("social-link-url-{id}")),
                        tr("social_link_placeholder"),
                        ProfileFieldRules::social_link_url().binding(Self::url_binding(state, id)),
                    )
                    .font_size(16.0)
                    .text_color(theme.text.primary)
                    .on_submit(|window, cx| ScreenFocus::release_text_field(window, cx))
                    .on_cancel(|window, cx| ScreenFocus::release_text_field(window, cx)),
                ),
            );

        div().w_full().tab_index(tab_order).tab_stop(false).tab_group().child(
            GroupedBtns::new(
                SharedString::from(format!("social-link-row-{id}")),
                vec![
                    GroupedBtn::new(item.id.clone(), "").content_slot(
                        vstack(0.0)
                            .w_full()
                            .child(title_row)
                            .child(
                                div()
                                    .w_full()
                                    .pl(px(58.0))
                                    .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.5))),
                            )
                            .child(url_row),
                    ),
                ],
            )
            .corner_radius(30.0)
            .horizontal_padding(0.0)
            .separator_leading_padding(0.0),
        )
    }

    fn add_button(state: &Entity<SocialLinksEditorState>, theme: &AppTheme) -> PlainButton {
        let state = state.clone();
        plain("social-links-add")
            .w_full()
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| state.append(cx));
            })
            .child(
                hstack(8.0)
                    .w_full()
                    .min_h(px(48.0))
                    .px(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_full()
                    .children(IconUi::sized("plus.circle.fill", px(18.0), theme.primary._100, true))
                    .child(
                        MainText::new(tr("social_links_add"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    )
                    .child(spacer_min_w(0.0)),
            )
    }

    fn pop(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn commit_and_pop(state: &Entity<SocialLinksEditorState>, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        let state = state.read(cx);
        if !state.can_commit() {
            return;
        }
        let normalized = SocialLink::normalize(&state.profile_links());
        if normalized != state.initial_normalized {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.save_social_links(&normalized, cx));
        }
        Self::pop(nav_store, cx);
    }
}

impl RenderOnce for SocialLinksEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("social-links-editor-state", cx, |_, _| SocialLinksEditorState::new());
        let nav_store = env::<NavigationStoreKey>(cx);
        let now = RiseClock::monotonic();
        let max_links = Self::max_links();

        let (can_commit, items_count, rows) = {
            let editor = state.read(cx);
            let count = editor.items.len() as isize;
            let rows: Vec<_> = editor
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let tab_order = index as isize - count;
                    item.presence.render_transitioned(now, window, |_| Self::row(&state, item, tab_order, &theme))
                })
                .collect();
            (editor.can_commit(), editor.live_items().count(), rows)
        };

        let content = vstack(14.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .tab_index(1)
            .tab_stop(false)
            .tab_group()
            .px(px(14.0))
            .pt(px(12.0))
            .child(
                div().px(px(4.0)).child(
                    MainText::new(tr("social_links_editor_hint"))
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .color(theme.text.secondary),
                ),
            )
            .child(vstack(10.0).w_full().children(rows))
            .when(items_count < max_links, |column| column.child(Self::add_button(&state, &theme)))
            .child(
                hstack(None).w_full().px(px(4.0)).child(spacer()).child(
                    MainText::new(format!("{items_count}/{max_links}")).px(12.0).color(theme.text.secondary),
                ),
            )
            .child(div().flex_none().h(px(80.0)));

        let commit_state = state.clone();
        let commit_nav_store = nav_store.clone();
        let hydrate_state = state.clone();
        SettingsWrapperUi::new(tr("social_links_title"), move |_, cx| Self::pop(&nav_store, cx))
            .right_jsx(div().when(!can_commit, |button| button.opacity(0.45)).child(
                PageHeaderIconButton::new("social-links-commit", "checkmark", move |_, cx| {
                    Self::commit_and_pop(&commit_state, &commit_nav_store, cx);
                })
                .disabled(!can_commit),
            ))
            .content(
                scroll_view("social-links-editor-scroll")
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(content),
            )
            .on_appear("social-links-editor-appear", move |_, cx| {
                let auth = AuthServicesStore::shared(cx);
                let existing = auth.read(cx).profile().map(|profile| profile.social_links.clone()).unwrap_or_default();
                hydrate_state.update(cx, |state, cx| {
                    state.hydrate(&existing);
                    cx.notify();
                });
            })
    }
}
