use std::collections::HashSet;

use gpui::{App, Context, Entity, Pixels, SharedString, StyleRefinement, Subscription, WeakEntity, Window};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatReactionCatalog, ChatReactionsType, ChatSettingsModel, ChatType,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::sparks::components::sparks_icon_view::SparksSettingsTile;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem};

const REMEMBERED_REACTION_LIMIT_SERVICE: &str = "api-gateway";
const REMEMBERED_REACTION_LIMIT_NAME: &str = "max_allowed_reactions";
const GRID_COLUMNS: u16 = 6;
const GRID_SPACING: f32 = 10.0;
const CELL_MINIMUM_HEIGHT: f32 = 50.0;
const CELL_CORNER_RADIUS: f32 = 12.0;
const CELL_STROKE_WIDTH: f32 = 1.5;
const CELL_GLYPH_SIZE: f32 = 26.0;
const EMOJI_PICKER_HEIGHT: f32 = 376.0;
const EMOJI_PICKER_PADDING: f32 = 8.0;
const EMOJI_PICKER_START_SCALE: f32 = 0.96;

type NavStore = Option<Entity<NavigationStore>>;

fn remembered_reaction_limit() -> usize {
    RiseLimits::SERVICE_LIMITS
        .iter()
        .find(|limit| limit.service == REMEMBERED_REACTION_LIMIT_SERVICE && limit.name == REMEMBERED_REACTION_LIMIT_NAME)
        .map(|limit| limit.value as usize)
        .unwrap_or(0)
}

pub struct ChatSettingsAllowedReactions {
    chat_id: Option<String>,
    column: Entity<ChatSettingsAllowedReactionsColumn>,
}

/// scrolled column of the page as its own cached view
/// it owns all page state so its render never reads the page
/// wheel steps would notify the page otherwise
struct ChatSettingsAllowedReactionsColumn {
    chat_id: Option<String>,
    nav_store: NavStore,
    custom_picker_open: bool,
    custom_picker_motion: Animated<f32>,
    standard_reactions: Vec<String>,
    services: Entity<ChatServicesStore>,
    height: Option<Pixels>,
    _subscription: Subscription,
}

impl Render for ChatSettingsAllowedReactionsColumn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column = self.column(window, cx);
        let measured = cx.weak_entity();
        div()
            .relative()
            .w_full()
            .child(column)
            .child(measure("chat-settings-reactions-column-size", move |bounds, _, cx| {
                let _ = measured.update(cx, |column, cx| {
                    if column.height != Some(bounds.size.height) {
                        column.height = Some(bounds.size.height);
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }
}

impl ChatSettingsAllowedReactions {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let column_chat_id = chat_id.clone();
        let column = cx.new(|cx| ChatSettingsAllowedReactionsColumn::new(column_chat_id, cx));
        Self { chat_id, column }
    }

    fn resolved_chat_id(&self) -> &str {
        self.chat_id.as_deref().unwrap_or_default()
    }
}

impl ChatSettingsAllowedReactionsColumn {
    fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let subscription = cx.observe(&services, |_, _, cx| cx.notify());
        Self {
            chat_id,
            nav_store: None,
            custom_picker_open: false,
            custom_picker_motion: Animated::new(0.0),
            standard_reactions: ChatReactionCatalog::all_emojis().to_vec(),
            services,
            height: None,
            _subscription: subscription,
        }
    }

    fn resolved_chat_id(&self) -> &str {
        self.chat_id.as_deref().unwrap_or_default()
    }

    fn model<'a>(&'a self, cx: &'a App) -> Option<&'a ChatSettingsModel> {
        self.services.read(cx).chat_settings(self.resolved_chat_id(), cx)
    }

    fn reactions_type(&self, cx: &App) -> ChatReactionsType {
        self.model(cx).map(|model| model.reactions_type).unwrap_or(ChatReactionsType::All)
    }

    fn set_custom_picker_open(&mut self, open: bool, motion: Option<(f64, f64)>, cx: &mut Context<Self>) {
        self.custom_picker_open = open;
        self.custom_picker_motion.set_in(
            if open { 1.0 } else { 0.0 },
            motion.map(|(response, damping_fraction)| spring(response, damping_fraction)),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }

    fn sparks_row(chat_id: &str, chat_type: ChatType, allowed: bool, can_manage_reactions: bool) -> GroupedBtn {
        let binding_chat_id = chat_id.to_owned();
        let callback_chat_id = chat_id.to_owned();
        let is_channel = chat_type == ChatType::Channel;
        GroupedBtn::new(
            "allowed_reactions_sparks",
            tr(if is_channel { "chat_settings_paid_reactions_channel" } else { "chat_settings_paid_reactions" }),
        )
        .leading_slot(SparksSettingsTile::new())
        .trailing_slot(SwitchUi::new(
            SharedString::from(format!("chat-settings-paid-reactions-switch-{chat_id}")),
            Binding::new(
                move |_: &App| allowed,
                move |value: bool, _: &mut Window, cx: &mut App| {
                    if !can_manage_reactions {
                        return;
                    }
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.set_paid_reactions_allowed(&binding_chat_id, value, cx));
                },
            ),
        ))
        .disabled(!can_manage_reactions)
        .callback(move |_: &mut Window, cx: &mut App| {
            ChatInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.set_paid_reactions_allowed(&callback_chat_id, !allowed, cx));
        })
    }

    fn type_items(&self, reactions_type: ChatReactionsType, can_manage_reactions: bool, theme: &AppTheme) -> Vec<GroupedBtn> {
        vec![
            self.row(ChatReactionsType::All, tr("chat_reactions_all"), reactions_type, can_manage_reactions, theme),
            self.row(ChatReactionsType::Custom, tr("chat_reactions_some"), reactions_type, can_manage_reactions, theme),
            self.row(ChatReactionsType::None, tr("chat_reactions_none"), reactions_type, can_manage_reactions, theme),
        ]
    }

    fn row(
        &self,
        t: ChatReactionsType,
        title: String,
        selected_type: ChatReactionsType,
        can_manage_reactions: bool,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let chat_id = self.resolved_chat_id().to_owned();
        let row = GroupedBtn::new(t.wire(), title)
            .show_chevron(false)
            .disabled(!can_manage_reactions)
            .callback(move |_: &mut Window, cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.set_reactions_type(&chat_id, t, cx));
            });
        if selected_type != t {
            return row;
        }
        match IconUi::sized("checkmark", px(14.0), theme.primary._100, false) {
            Some(icon) => row.trailing_slot(icon),
            None => row,
        }
    }

    fn footer(reactions_type: ChatReactionsType, is_channel: bool) -> String {
        tr(match reactions_type {
            ChatReactionsType::All => {
                if is_channel { "chat_reactions_footer_all_channel" } else { "chat_reactions_footer_all" }
            }
            ChatReactionsType::Custom => {
                if is_channel { "chat_reactions_footer_custom_channel" } else { "chat_reactions_footer_custom" }
            }
            ChatReactionsType::None => {
                if is_channel { "chat_reactions_footer_none_channel" } else { "chat_reactions_footer_none" }
            }
        })
    }

    fn reaction_grid(
        reactions: &[String],
        chat_id: &str,
        allowed: &HashSet<String>,
        can_manage_reactions: bool,
        theme: &AppTheme,
    ) -> Div {
        div()
            .w_full()
            .px(px(12.0))
            .grid()
            .grid_cols(GRID_COLUMNS)
            .gap(px(GRID_SPACING))
            .children(reactions.iter().map(|reaction| {
                Self::reaction_cell(reaction, chat_id, allowed.contains(reaction), can_manage_reactions, theme)
            }))
    }

    fn emoji_grid(&self, allowed: &HashSet<String>, can_manage_reactions: bool, theme: &AppTheme) -> Div {
        vstack(8.0)
            .w_full()
            .items_stretch()
            .child(
                div()
                    .px(px(16.0))
                    .child(MainText::new(tr("chat_reactions_available_section")).px(12.0).color(theme.text.secondary)),
            )
            .child(Self::reaction_grid(
                &self.standard_reactions,
                self.resolved_chat_id(),
                allowed,
                can_manage_reactions,
                theme,
            ))
    }

    fn custom_emoji_section(
        &self,
        this: &WeakEntity<Self>,
        nav_store: &NavStore,
        selected: &[String],
        allowed: &HashSet<String>,
        can_manage_reactions: bool,
        picker_progress: f32,
        theme: &AppTheme,
    ) -> Div {
        let toggle = this.clone();
        let mut picker_row = GroupedBtn::new("allowed_reactions_custom_emoji", tr("chat_reactions_custom_emoji_add"))
            .icon("face.smiling.inverse")
            .icon_background(SettingsIconPalette::stickers())
            .disabled(!can_manage_reactions)
            .callback(move |_: &mut Window, cx: &mut App| {
                let _ = selection_changed();
                let _ = toggle.update(cx, |page, cx| {
                    page.set_custom_picker_open(!page.custom_picker_open, Some((0.38, 0.86)), cx);
                });
            });
        if self.custom_picker_open || picker_progress > 0.0 {
            picker_row = picker_row.expanded_slot(self.custom_emoji_picker(this, nav_store, picker_progress));
        }
        vstack(8.0)
            .w_full()
            .items_stretch()
            .child(
                GroupedBtns::new("chat-settings-reactions-custom-emoji", vec![picker_row])
                    .title(tr("chat_reactions_custom_emoji_section"))
                    .corner_radius(24.0),
            )
            .map(|column| {
                if selected.is_empty() {
                    column.child(
                        div().px(px(16.0)).child(
                            SecondaryText::new(tr("chat_reactions_custom_emoji_empty")).px(14.0).tac(TextAlignT::Left),
                        ),
                    )
                } else {
                    column.child(Self::reaction_grid(
                        selected,
                        self.resolved_chat_id(),
                        allowed,
                        can_manage_reactions,
                        theme,
                    ))
                }
            })
    }

    fn reaction_cell(
        reaction: &str,
        chat_id: &str,
        is_on: bool,
        can_manage_reactions: bool,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let shape = Shape::continuous(CELL_CORNER_RADIUS);
        let toggle_chat_id = chat_id.to_owned();
        let toggle_reaction = reaction.to_owned();
        plain(SharedString::from(format!("chat-settings-reaction-{reaction}")))
            .w_full()
            .disabled(!can_manage_reactions)
            .on_press(move |_: &mut Window, cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.toggle_allowed_reaction(&toggle_chat_id, &toggle_reaction, cx);
                });
            })
            .child(
                shape
                    .round(div())
                    .relative()
                    .w_full()
                    .min_h(px(CELL_MINIMUM_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if is_on { theme.primary._100.opacity(0.18) } else { theme.bg._200 })
                    .child(ReactionEmojiGlyph::new(reaction.to_owned(), CELL_GLYPH_SIZE))
                    .when(is_on, |cell| cell.child(centred_stroke(theme.primary._100, CELL_STROKE_WIDTH, shape))),
            )
    }

    fn custom_emoji_picker(&self, this: &WeakEntity<Self>, nav_store: &NavStore, progress: f32) -> impl IntoElement + use<> {
        let chat_id = self.resolved_chat_id().to_owned();
        let picked = this.clone();
        let create_pack = this.clone();
        let create_store = nav_store.clone();
        div()
            .w_full()
            .overflow_hidden()
            .max_h(px((EMOJI_PICKER_HEIGHT + EMOJI_PICKER_PADDING) * progress.max(0.0)))
            .child(
                div()
                    .w_full()
                    .h(px(EMOJI_PICKER_HEIGHT))
                    .px(px(EMOJI_PICKER_PADDING))
                    .mb(px(EMOJI_PICKER_PADDING))
                    .child(
                        EmojiPickerUi::new("chat-settings-reactions-emoji-picker", |_: String, _, _| {})
                            .chrome(false)
                            .shows_custom_emoji(true)
                            .on_pick_custom(move |item: StickerItem, _, cx| {
                                let Some(reference) = EmojiPickerCatalog::custom_emoji_ref(&item.id) else {
                                    return;
                                };
                                ChatInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.toggle_allowed_reaction(&chat_id, &reference, cx));
                                let _ = picked.update(cx, |page, cx| {
                                    page.set_custom_picker_open(false, Some((0.36, 0.88)), cx);
                                });
                            })
                            .custom_emoji_only(true)
                            .on_create_custom_emoji_pack(move |_, cx| {
                                let _ = create_pack.update(cx, |page, cx| page.set_custom_picker_open(false, None, cx));
                                if let Some(nav_store) = &create_store {
                                    nav_store.update(cx, |nav_store, cx| {
                                        nav_store.push(
                                            RootRoute::MyStickerPacks { kind: PackKind::Emoji },
                                            NavigationStoreSettlingPush::Drop,
                                            cx,
                                        );
                                    });
                                }
                            }),
                    )
                    .opacity(progress.clamp(0.0, 1.0))
                    .scale_effect_anchored(
                        EMOJI_PICKER_START_SCALE + (1.0 - EMOJI_PICKER_START_SCALE) * progress,
                        UnitPoint::TOP,
                    ),
            )
    }
}

impl ChatSettingsAllowedReactionsColumn {
    fn column(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.weak_entity();
        let nav_store = self.nav_store.clone();
        let resolved_chat_id = self.resolved_chat_id().to_owned();
        let chat = self.services.read(cx).chat_info(&resolved_chat_id, cx);
        let can_manage_reactions =
            chat.as_ref().map(|chat| ChatProfilePermissions::manages_reactions(chat, cx)).unwrap_or(false);
        let chat_type = chat.as_ref().map(|chat| chat.r#type);
        let reactions_type = self.reactions_type(cx);
        let saved = self.model(cx).map(|model| model.allowed_reactions.clone());
        let allowed: HashSet<String> = saved.iter().flatten().cloned().collect();
        let allowed_custom_emoji: Vec<String> = saved
            .iter()
            .flatten()
            .filter(|reaction| reaction.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX))
            .cloned()
            .collect();

        let task_chat_id = resolved_chat_id.clone();
        use_task(
            SharedString::from(format!("chat-settings-reactions-task-{resolved_chat_id}")),
            resolved_chat_id.clone(),
            window,
            cx,
            async move |cx| {
                let _ = cx.update(|_, cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.load_chat_settings(&task_chat_id, cx);
                        store.load_effective_permissions(&task_chat_id, None, false, cx);
                    });
                });
            },
        );

        let saved_page = this.clone();
        on_change(
            SharedString::from(format!("chat-settings-reactions-saved-{resolved_chat_id}")),
            saved.clone(),
            true,
            window,
            cx,
            move |_, saved: &Option<Vec<String>>, _, cx| {
                let _ = saved_page.update(cx, |page, cx| {
                    let next = ChatSettingsReactionChoices::standard(
                        ChatReactionCatalog::all_emojis(),
                        saved.as_deref().unwrap_or_default(),
                        &page.standard_reactions,
                        remembered_reaction_limit(),
                        MessageReaction::CUSTOM_EMOJI_PREFIX,
                    );
                    if next != page.standard_reactions {
                        page.standard_reactions = next;
                        cx.notify();
                    }
                });
            },
        );

        let picker_progress = self.custom_picker_motion.frame_value(RiseClock::monotonic(), window);
        let is_channel = chat_type == Some(ChatType::Channel);

        let mut column = vstack(18.0)
            .w_full()
            .items_stretch()
            .pt(px(12.0))
            .child(
                GroupedBtns::new(
                    "chat-settings-reactions-type",
                    self.type_items(reactions_type, can_manage_reactions, &theme),
                )
                .title(tr("chat_reactions_section"))
                .end_group_title(Self::footer(reactions_type, is_channel)),
            );

        if let Some(chat_type) = chat_type.filter(|chat_type| *chat_type == ChatType::Group || *chat_type == ChatType::Channel) {
            let allowed_paid = self.services.read(cx).paid_reactions_allowed(&resolved_chat_id, chat_type);
            column = column.child(
                GroupedBtns::new(
                    "chat-settings-reactions-sparks",
                    vec![Self::sparks_row(&resolved_chat_id, chat_type, allowed_paid, can_manage_reactions)],
                )
                .end_group_title(tr(if chat_type == ChatType::Channel {
                    "chat_settings_paid_reactions_hint_channel"
                } else {
                    "chat_settings_paid_reactions_hint_group"
                })),
            );
        }

        if reactions_type == ChatReactionsType::Custom {
            column = column.child(self.emoji_grid(&allowed, can_manage_reactions, &theme)).child(self.custom_emoji_section(
                &this,
                &nav_store,
                &allowed_custom_emoji,
                &allowed,
                can_manage_reactions,
                picker_progress,
                &theme,
            ));
        }

        column.child(spacer_min_h(60.0)).into_any_element()
    }
}

impl Render for ChatSettingsAllowedReactions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store: NavStore = env::<NavigationStoreKey>(cx);
        let (column_nav_store, column_height) = {
            let column = self.column.read(cx);
            (column.nav_store.clone(), column.height)
        };
        if column_nav_store.as_ref().map(Entity::entity_id) != nav_store.as_ref().map(Entity::entity_id) {
            let column_nav_store = nav_store.clone();
            self.column.update(cx, |column, _| column.nav_store = column_nav_store);
        }
        let resolved_chat_id = self.resolved_chat_id().to_owned();
        let column = match column_height {
            Some(height) => self.column.clone().cached(StyleRefinement::default().w_full().h(height)).into_any_element(),
            None => self.column.clone().into_any_element(),
        };
        let content = scroll_view(SharedString::from(format!("chat-settings-reactions-scroll-{resolved_chat_id}")))
            .list_shell_scroll_under_chrome(cx)
            .size_full()
            .bg(theme.bg._100)
            .child(column);

        SettingsWrapperUi::new(tr("chat_settings_allowed_reactions"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(content)
    }
}

enum ChatSettingsReactionChoices {}

impl ChatSettingsReactionChoices {
    fn standard(
        catalog: &[String],
        saved: &[String],
        previous: &[String],
        remembered_limit: usize,
        custom_prefix: &str,
    ) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for emoji in catalog {
            if seen.insert(emoji.clone()) {
                result.push(emoji.clone());
            }
        }

        let selected: HashSet<&String> = saved.iter().filter(|emoji| !emoji.starts_with(custom_prefix)).collect();
        let mut remembered = 0;
        for emoji in previous {
            if emoji.starts_with(custom_prefix) || seen.contains(emoji) {
                continue;
            }
            if !selected.contains(emoji) && remembered >= remembered_limit {
                continue;
            }
            if !selected.contains(emoji) {
                remembered += 1;
            }
            seen.insert(emoji.clone());
            result.push(emoji.clone());
        }
        for emoji in saved {
            if !emoji.starts_with(custom_prefix) && seen.insert(emoji.clone()) {
                result.push(emoji.clone());
            }
        }
        result
    }
}
