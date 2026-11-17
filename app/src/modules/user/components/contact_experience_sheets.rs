use std::rc::Rc;

use gpui::{App, ElementId, Empty, Entity, FontWeight, Hsla, SharedString, Subscription, Window, white};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::UserContactPhotoSuggestionWireDTO;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

type ContactSyncDecision = Rc<dyn Fn(bool, &mut Window, &mut App)>;

struct ContactSyncOnboardingState {
    is_submitting: bool,
}

#[derive(IntoElement, Default)]
pub struct ContactSyncOnboardingSheet {
    on_decision: Option<ContactSyncDecision>,
}

impl ContactSyncOnboardingSheet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_decision(mut self, on_decision: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_decision = Some(Rc::new(on_decision));
        self
    }

    /// presents the contact sync onboarding sheet, the kit host owns the presentation modifiers on desktop
    pub fn presented(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        cx: &App,
    ) -> NativeBottomSheetPresentationHost {
        let background = rise_ui::theme(cx).bg._100;
        let on_decision = self.on_decision;
        NativeBottomSheetPresentationHost::new(id, is_presented, move |_, _| ContactSyncOnboardingSheet {
            on_decision: on_decision.clone(),
        })
        .presentation_detents([PresentationDetent::Height(430.0)])
        .presentation_drag_indicator(true)
        .presentation_background(background)
        .presentation_corner_radius(30.0)
        .interactive_dismiss_disabled(true)
    }
}

impl RenderOnce for ContactSyncOnboardingSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("contact-sync-onboarding-state", cx, |_, _| ContactSyncOnboardingState {
            is_submitting: false,
        });
        let is_submitting = state.read(cx).is_submitting;

        let continue_decision = self.on_decision.clone();
        let continue_state = state.clone();
        let decline_decision = self.on_decision;

        let mut continue_label = hstack(None);
        if is_submitting {
            continue_label = continue_label.child(progress_view().tint(theme.content_on(theme.primary._100)));
        }
        continue_label = continue_label.child(
            MainText::new(tr("contact_onboarding_continue"))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.content_on(theme.primary._100)),
        );

        vstack(22.0)
            .w_full()
            .px(px(24.0))
            .pt(px(34.0))
            .pb(px(22.0))
            .bg(theme.bg._100)
            .child(
                div()
                    .size(px(82.0))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.primary._100.opacity(0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("person.2.crop.square.stack.fill", px(34.0), theme.primary._100, true)),
            )
            .child(
                vstack(9.0)
                    .child(
                        MainText::new(tr("contact_onboarding_title"))
                            .px(22.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary)
                            .number_of_lines(2),
                    )
                    .child(
                        MainText::new(tr("contact_onboarding_description"))
                            .px(15.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
            )
            .child(
                vstack(10.0)
                    .w_full()
                    .child(
                        plain("contact-sync-onboarding-continue")
                            .w_full()
                            .disabled(is_submitting)
                            .on_press(move |window, cx| {
                                if let Some(on_decision) = &continue_decision {
                                    on_decision(true, window, cx);
                                    return;
                                }
                                continue_state.update(cx, |state, cx| {
                                    state.is_submitting = true;
                                    cx.notify();
                                });
                                UserInteractionsStore::shared(cx)
                                    .update(cx, |interactions, cx| interactions.accept_contact_sync_onboarding(cx));
                            })
                            .child(
                                div()
                                    .w_full()
                                    .h(px(52.0))
                                    .rounded_full()
                                    .bg(theme.primary._100)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(continue_label),
                            ),
                    )
                    .child(
                        plain("contact-sync-onboarding-not-now")
                            .w_full()
                            .disabled(is_submitting)
                            .on_press(move |window, cx| {
                                if let Some(on_decision) = &decline_decision {
                                    on_decision(false, window, cx);
                                    return;
                                }
                                UserInteractionsStore::shared(cx)
                                    .update(cx, |interactions, cx| interactions.decline_contact_sync_onboarding(cx));
                            })
                            .child(
                                div().w_full().h(px(48.0)).flex().items_center().justify_center().child(
                                    MainText::new(tr("contact_onboarding_not_now"))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.secondary),
                                ),
                            ),
                    ),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResolutionAction {
    Reject,
    Accept,
}

/// state of the view plus the user store binding
struct ContactPhotoSuggestionState {
    user_store: Entity<UserServicesStore>,
    resolving_action: Option<ResolutionAction>,
    error_text: Option<String>,
    observed_suggestion: Option<UserContactPhotoSuggestionWireDTO>,
    _user_store_observation: Subscription,
}

impl ContactPhotoSuggestionState {
    fn new(cx: &mut Context<Self>) -> Self {
        let user_store = UserServicesStore::shared(cx);
        let observed_suggestion = user_store.read(cx).active_contact_photo_suggestion.clone();
        let observation = cx.observe(&user_store, |this, store, cx| {
            let suggestion = store.read(cx).active_contact_photo_suggestion.clone();
            if suggestion != this.observed_suggestion {
                this.observed_suggestion = suggestion;
                cx.notify();
            }
        });
        Self {
            user_store,
            resolving_action: None,
            error_text: None,
            observed_suggestion,
            _user_store_observation: observation,
        }
    }
}

#[derive(IntoElement, Default)]
pub struct ContactPhotoSuggestionSheet;

impl ContactPhotoSuggestionSheet {
    pub fn new() -> Self {
        Self
    }

    fn suggester_name(suggestion: &UserContactPhotoSuggestionWireDTO) -> String {
        let value = suggestion
            .suggester
            .as_ref()
            .map(|suggester| trimming_whitespaces_and_newlines(&suggester.name))
            .unwrap_or_default();
        if value.is_empty() { tr("contact_unknown_name") } else { value.to_owned() }
    }

    fn suggestion(state: &Entity<ContactPhotoSuggestionState>, cx: &App) -> Option<UserContactPhotoSuggestionWireDTO> {
        state.read(cx).user_store.read(cx).active_contact_photo_suggestion.clone()
    }

    fn suggestion_content(
        suggestion: &UserContactPhotoSuggestionWireDTO,
        state: &Entity<ContactPhotoSuggestionState>,
        theme: &AppTheme,
        cx: &App,
    ) -> Div {
        let name = Self::suggester_name(suggestion);
        let error_text = state.read(cx).error_text.clone();

        vstack(20.0)
            .w_full()
            .px(px(20.0))
            .pt(px(10.0))
            .pb(px(18.0))
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        UserLogoUi::new("contact-photo-suggestion-logo")
                            .source(
                                suggestion.suggester.as_ref().map(|suggester| SharedString::from(suggester.logo.clone())),
                            )
                            .name(SharedString::from(name.clone()))
                            .user_id(SharedString::from(suggestion.suggester_user_id.clone()))
                            .size(44.0)
                            .show_story_ring(false),
                    )
                    .child(
                        vstack(4.0)
                            .align(HorizontalAlignment::Leading)
                            .min_w_0()
                            .child(
                                MainText::new(tr("contact_photo_suggestion_title"))
                                    .px(20.0)
                                    .font_weight(FontWeight::BOLD)
                                    .color(theme.text.primary)
                                    .number_of_lines(2),
                            )
                            .child(
                                MainText::new(tr("contact_photo_suggestion_from").replace("{name}", &name))
                                    .px(14.0)
                                    .color(theme.text.secondary)
                                    .number_of_lines(2),
                            ),
                    )
                    .child(spacer()),
            )
            .child(
                div()
                    .relative()
                    .size(px(210.0))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.bg._200)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .relative()
                            .size(px(196.0))
                            .child(
                                div().size_full().rounded_full().overflow_hidden().child(
                                    CleverImageUi::new(suggestion.stable_file_url.clone())
                                        .content_mode(ContentMode::Fill)
                                        .corner_radius(98.0)
                                        .cache_policy(CleverImageCachePolicy::PrivateEphemeral),
                                ),
                            )
                            .child(centred_stroke(theme.border._200.opacity(0.7), 1.0, Shape::Circle)),
                    ),
            )
            .children(error_text.map(|error_text| {
                MainText::new(error_text)
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .color(theme.semantic.error_100)
                    .number_of_lines(3)
            }))
    }

    fn suggestion_footer(
        suggestion: &UserContactPhotoSuggestionWireDTO,
        state: &Entity<ContactPhotoSuggestionState>,
        theme: &AppTheme,
        cx: &App,
    ) -> Div {
        let resolving_action = state.read(cx).resolving_action;
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .child(Self::action_button(
                "contact-photo-suggestion-reject",
                tr("contact_photo_suggestion_reject"),
                theme.semantic.error_100,
                theme.bg._200,
                resolving_action,
                ResolutionAction::Reject,
                suggestion,
                state,
                theme,
            ))
            .child(Self::action_button(
                "contact-photo-suggestion-accept",
                tr("contact_photo_suggestion_accept"),
                white(),
                theme.primary._100,
                resolving_action,
                ResolutionAction::Accept,
                suggestion,
                state,
                theme,
            ))
    }

    #[allow(clippy::too_many_arguments)]
    fn action_button(
        id: &'static str,
        title: String,
        foreground: Hsla,
        background: Hsla,
        resolving_action: Option<ResolutionAction>,
        action: ResolutionAction,
        suggestion: &UserContactPhotoSuggestionWireDTO,
        state: &Entity<ContactPhotoSuggestionState>,
        theme: &AppTheme,
    ) -> PlainButton {
        let is_loading = resolving_action == Some(action);
        let suggestion_id = suggestion.id.clone();
        let state = state.clone();
        let label = if is_loading {
            progress_view().tint(foreground).into_any_element()
        } else {
            MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(foreground).into_any_element()
        };
        plain(id)
            .flex_1()
            .min_w_0()
            .disabled(resolving_action.is_some())
            .opacity(if resolving_action.is_none() || is_loading { 1.0 } else { 0.62 })
            .on_press(move |_, cx| Self::resolve(action, &suggestion_id, &state, cx))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(50.0))
                    .rounded_full()
                    .bg(background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(label)
                    .child(centred_stroke(theme.border._200.opacity(0.55), 0.6, Shape::Capsule)),
            )
    }

    fn resolve(
        action: ResolutionAction,
        suggestion_id: &str,
        state: &Entity<ContactPhotoSuggestionState>,
        cx: &mut App,
    ) {
        if state.read(cx).resolving_action.is_some() {
            return;
        }
        state.update(cx, |state, cx| {
            state.resolving_action = Some(action);
            state.error_text = None;
            cx.notify();
        });
        let result_state = state.downgrade();
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.resolve_contact_photo_suggestion(
                suggestion_id,
                action == ResolutionAction::Accept,
                move |result, cx| {
                    let _ = result_state.update(cx, |state, cx| {
                        state.resolving_action = None;
                        if let Err(error) = result.as_ref() {
                            state.error_text = Some(CanonErrors::refusal_text(error));
                        }
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }
}

impl RenderOnce for ContactPhotoSuggestionSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("contact-photo-suggestion-state", cx, |_, cx: &mut Context<ContactPhotoSuggestionState>| {
            ContactPhotoSuggestionState::new(cx)
        });

        let suggestion_id = Self::suggestion(&state, cx).map(|suggestion| suggestion.id);
        let change_state = state.clone();
        on_change("contact-photo-suggestion-id", suggestion_id, true, window, cx, move |_, _, _, cx| {
            change_state.update(cx, |state, cx| {
                state.resolving_action = None;
                state.error_text = None;
                cx.notify();
            });
        });

        let read_state = state.clone();
        let is_presented = Binding::new(move |cx: &App| Self::suggestion(&read_state, cx).is_some(), |_, _, _| {});
        let content_state = state.clone();
        let content_theme = theme.clone();
        let footer_state = state.clone();
        let footer_theme = theme.clone();
        let is_locked = state.read(cx).resolving_action.is_some();

        BottomSheetUi::new(
            "contact-photo-suggestion-sheet",
            is_presented,
            move |_, cx| match Self::suggestion(&content_state, cx) {
                Some(suggestion) => {
                    Self::suggestion_content(&suggestion, &content_state, &content_theme, cx).into_any_element()
                }
                None => Empty.into_any_element(),
            },
            move |_, cx| match Self::suggestion(&footer_state, cx) {
                Some(suggestion) => {
                    Self::suggestion_footer(&suggestion, &footer_state, &footer_theme, cx).into_any_element()
                }
                None => Empty.into_any_element(),
            },
        )
        .snap_fractions(vec![0.45])
        .full_page_allows_swipe_dismiss(false)
        .footer_shows_top_border(false)
        .fits_content(true)
        .full_page_interaction_locked(is_locked)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }
}
