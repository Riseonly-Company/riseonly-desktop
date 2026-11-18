use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Subscription, UniformListScrollHandle, Window, uniform_list};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowUi};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::text_field_alert::TextFieldAlert;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::{ChatInputStickerCell, StickerItemDeletionRequest};
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldEntry, StickerHoldGridModifier, StickerHoldPreviewModifiers, StickerHoldPreviewPresenter,
    StickerPreviewAction, StickerPreviewAsset,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::stickers::components::sticker_pack_add_cell::{StickerAssetComposerStore, StickerPackAddCell};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_chat_send_target::StickerPackChatSendTargets;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const HEADER_HEIGHT: f32 = 65.0;
const GRID_TOP_INSET: f32 = 18.0;
const GRID_HORIZONTAL_INSET: f32 = 14.0;
const GRID_BOTTOM_SPACER: f32 = 24.0;
const SKELETON_CELL_COUNT: usize = 30;

fn title_rule() -> RiseFieldRule {
    RiseFieldRule::new("stickers_edit_pack", "/title")
}

fn column_count(kind: PackKind) -> usize {
    if kind == PackKind::Emoji { 8 } else { 5 }
}

fn grid_spacing(kind: PackKind) -> f32 {
    if kind == PackKind::Emoji { 0.0 } else { 8.0 }
}

fn sticker_cell_size(kind: PackKind, width: f32) -> f32 {
    let columns = column_count(kind) as f32;
    let total_spacing = grid_spacing(kind) * (columns - 1.0).max(0.0);
    let available = width - GRID_HORIZONTAL_INSET * 2.0 - total_spacing;
    (available / columns).max(36.0)
}

fn effective_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
}

fn is_self_premium(cx: &App) -> bool {
    rise_ui::shared::try_read::<AuthServicesStore>(cx)
        .and_then(AuthServicesStore::profile)
        .is_some_and(|profile| profile.more.is_premium)
}

fn is_locked(pack: &StickerPack, cx: &App) -> bool {
    pack.is_premium && !is_self_premium(cx)
}

fn sticker_menu(entry: &StickerHoldEntry, cx: &App) -> Vec<StickerPreviewAction> {
    let StickerPreviewAsset::Sticker(item) = &entry.asset else {
        return Vec::new();
    };
    let Some(user_id) = rise_ui::shared::try_read::<AuthServicesStore>(cx)
        .and_then(AuthServicesStore::effective_user_id_for_wire)
        .filter(|user_id| !user_id.is_empty())
    else {
        return Vec::new();
    };
    let Some(services) = rise_ui::shared::try_read::<StickersServicesStore>(cx) else {
        return Vec::new();
    };
    let is_favourite = services.is_favourite(&user_id, &item.id);
    let favourite_item = item.clone();
    let mut actions = vec![StickerPreviewAction::new(
        "favourite",
        if is_favourite { tr("favourite_remove") } else { tr("favourite_add") },
        if is_favourite { "bookmark.slash.fill" } else { "bookmark.fill" },
        Rc::new(move |_: &mut Window, cx: &mut App| {
            StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.toggle_favourite(&user_id, &favourite_item, cx);
            });
        }),
    )];
    let shown = rise_ui::shared::try_read::<StickerPackSheetStore>(cx)
        .and_then(|store| store.resolved_pack(cx))
        .filter(|pack| pack.id == item.pack_id);
    let Some(pack) = shown.or_else(|| services.pack_summary(&item.pack_id)) else {
        return actions;
    };
    if pack.can_edit_stickers() {
        let edit_pack = pack.clone();
        let edit_item = item.clone();
        actions.push(StickerPreviewAction::new(
            "edit",
            tr("edit_action"),
            "pencil",
            Rc::new(move |_: &mut Window, cx: &mut App| {
                StickerAssetComposerStore::shared(cx).update(cx, |composer, cx| {
                    composer.present_sticker(edit_pack.clone(), edit_item.clone(), cx);
                });
            }),
        ));
    }
    if pack.can_delete_stickers() {
        let delete_item = item.clone();
        actions.push(
            StickerPreviewAction::new(
                "delete",
                tr("delete_action"),
                "trash",
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    item_delete_alert(StickerItemDeletionRequest { item: delete_item.clone(), pack: pack.clone() }, cx);
                }),
            )
            .destructive(true),
        );
    }
    actions
}

fn send_to_chat_beside(entry: &StickerHoldEntry, window: &mut Window, cx: &mut App) {
    let StickerPreviewAsset::Sticker(item) = &entry.asset else {
        return;
    };
    let Some(send) = StickerPackChatSendTargets::current(cx) else {
        return;
    };
    let shown = rise_ui::shared::try_read::<StickerPackSheetStore>(cx)
        .and_then(|store| store.resolved_pack(cx))
        .filter(|pack| pack.id == item.pack_id);
    let pack = shown.or_else(|| {
        rise_ui::shared::try_read::<StickersServicesStore>(cx).and_then(|services| services.pack_summary(&item.pack_id))
    });
    if pack.as_ref().is_some_and(|pack| is_locked(pack, cx)) {
        let is_premium = is_self_premium(cx);
        let source = format!("sticker_pack:{}", item.pack_id);
        PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
            upsell.offer_premium_feature(tr("premium_upsell_reason_sticker_pack"), &source, is_premium, cx);
        });
        return;
    }
    send(item.clone(), window, cx);
}

fn item_delete_alert(request: StickerItemDeletionRequest, cx: &mut App) {
    let title = if request.pack.pack_kind == PackKind::Emoji {
        "picker_delete_emoji_title"
    } else {
        "picker_delete_sticker_title"
    };
    NativeConfirmAlert::present_delete(
        &tr(title),
        &tr("picker_delete_item_message"),
        Some(tr("delete_action")),
        None,
        false,
        None,
        None,
        move |cx: &mut App| {
            let Some(user_id) = effective_user_id(cx) else {
                return;
            };
            StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.delete_sticker(&user_id, &request.pack.id, &request.item.id, cx);
            });
        },
        cx,
    );
}

#[derive(Clone)]
enum StickerPackGridSlot {
    Add(StickerPack),
    Sticker(StickerItem),
}

pub struct StickerPackSheet {
    store: Entity<StickerPackSheetStore>,
    nav_store: Option<Entity<NavigationStore>>,
    hold_presenter: Entity<StickerHoldPreviewPresenter>,
    hold_was_active: bool,
    grid_scroll: UniformListScrollHandle,
    show_rename: bool,
    rename_title: String,
    stickers_count: usize,
    content_was_live: bool,
    paywall_request_count: u64,
    presents_in_side_panel: bool,
    width: f32,
    height: f32,
    footer_height: f32,
    _subscriptions: Vec<Subscription>,
}

impl StickerPackSheet {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = StickerPackSheetStore::shared(cx);
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let upsell = PremiumUpsellStore::shared(cx);
        let hold_presenter = StickerHoldPreviewPresenter::shared(cx);
        let paywall_request_count = upsell.read(cx).paywall_request_count();
        let hold_was_active = hold_presenter.read(cx).is_active();
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| this.stickers_did_change(cx)),
            cx.observe(&services, |this, _, cx| this.stickers_did_change(cx)),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&hold_presenter, |this, presenter, cx| {
                let is_active = presenter.read(cx).is_active();
                if is_active != this.hold_was_active {
                    this.hold_was_active = is_active;
                    cx.notify();
                }
            }),
            cx.observe(&upsell, |this, upsell, cx| {
                let count = upsell.read(cx).paywall_request_count();
                if count == this.paywall_request_count {
                    return;
                }
                this.paywall_request_count = count;
                let store = this.store.clone();
                if store.read(cx).is_presented() {
                    store.update(cx, |store, cx| store.dismiss(cx));
                }
            }),
        ];
        Self {
            store,
            nav_store: None,
            hold_presenter,
            hold_was_active,
            grid_scroll: UniformListScrollHandle::new(),
            show_rename: false,
            rename_title: String::new(),
            stickers_count: 0,
            content_was_live: false,
            paywall_request_count,
            presents_in_side_panel: false,
            width: 0.0,
            height: 0.0,
            footer_height: 0.0,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_presents_in_side_panel(&mut self, presents_in_side_panel: bool, cx: &mut Context<Self>) {
        if self.presents_in_side_panel != presents_in_side_panel {
            self.presents_in_side_panel = presents_in_side_panel;
            cx.notify();
        }
    }

    /// sheet content and footer without the sheet, for the right panel
    pub fn side_panel_content(this: &Entity<Self>, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let shows_footer = {
            let sheet = this.read(cx);
            Self::shows_footer(sheet.store.read(cx), cx)
        };
        let mut column = div().flex().flex_col().size_full().bg(theme.bg._100);
        column = column.child(div().flex_1().min_h_0().child(Self::content(this, cx)));
        if shows_footer {
            column = column
                .child(div().w_full().flex_shrink_0().h(theme.shell.column_divider).bg(theme.border._100))
                .child(div().w_full().flex_shrink_0().child(Self::footer(this, cx)));
        }
        column.into_any_element()
    }

    fn stickers_did_change(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let count = store.read(cx).stickers(cx).len();
        if count != self.stickers_count {
            self.stickers_count = count;
            store.update(cx, |store, cx| store.reconcile_resolved_link(cx));
        }
        let is_live = self.store.read(cx).is_presented() || self.presents_in_side_panel;
        if is_live || self.content_was_live {
            cx.notify();
        }
        self.content_was_live = is_live;
    }

    fn pack_kind(store: &StickerPackSheetStore, cx: &App) -> PackKind {
        store.resolved_pack(cx).map_or_else(|| store.kind(), |pack| pack.pack_kind)
    }

    /// saving follows the pack identity even before the first sticker is added
    /// members allowed to add use the grid cell for an empty pack
    fn shows_footer(store: &StickerPackSheetStore, cx: &App) -> bool {
        if store.is_unavailable(cx) {
            return false;
        }
        let Some(pack) = store.resolved_pack(cx) else {
            return false;
        };
        if !store.has_loaded_stickers(cx) || !store.stickers(cx).is_empty() {
            return true;
        }
        !pack.can_add_stickers()
    }

    fn sheet(this: &Entity<Self>, visible: &Binding<bool>, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let (shows_footer, is_active) = {
            let sheet = this.read(cx);
            (Self::shows_footer(sheet.store.read(cx), cx), sheet.hold_presenter.read(cx).is_active())
        };
        let content_sheet = this.clone();
        let footer_sheet = this.clone();
        BottomSheetUi::new(
            "sticker-pack-sheet",
            visible.clone(),
            move |_, cx| Self::content(&content_sheet, cx),
            move |_, cx| Self::footer(&footer_sheet, cx),
        )
        .snap_fractions(vec![0.62, 0.93])
        .full_page(true)
        .full_page_shows_drag_indicator(false)
        .footer_shows_top_border(shows_footer)
        .content_scrollable(true)
        .full_page_covers_full_screen(false)
        .full_page_interaction_locked(is_active)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .into_any_element()
    }

    fn content(this: &Entity<Self>, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let footer_reserve = {
            let sheet = this.read(cx);
            let overlaid = !sheet.presents_in_side_panel && Self::shows_footer(sheet.store.read(cx), cx);
            if overlaid { sheet.footer_height } else { 0.0 }
        };
        let (pack, stickers, is_unavailable, is_loading, kind, is_active, width, height, grid_scroll, in_side_panel) = {
            let sheet = this.read(cx);
            let store = sheet.store.read(cx);
            (
                store.resolved_pack(cx),
                store.stickers(cx),
                store.is_unavailable(cx),
                store.is_loading(cx),
                Self::pack_kind(store, cx),
                sheet.hold_presenter.read(cx).is_active(),
                sheet.width,
                sheet.height,
                sheet.grid_scroll.clone(),
                sheet.presents_in_side_panel,
            )
        };

        let body = if is_unavailable {
            Self::empty_state(&theme, "exclamationmark.circle", tr("sticker_pack_unavailable"))
        } else if is_loading {
            StickerPackSkeleton {
                columns: column_count(kind),
                cell_size: sticker_cell_size(kind, width),
                top_padding: HEADER_HEIGHT + GRID_TOP_INSET,
                spacing: grid_spacing(kind),
            }
            .into_any_element()
        } else if stickers.is_empty() && !pack.as_ref().is_some_and(StickerPack::can_add_stickers) {
            Self::empty_state(&theme, "face.smiling", tr("sticker_pack_empty"))
        } else {
            Self::grid(
                pack.as_ref(),
                stickers,
                kind,
                is_active,
                (width, height),
                footer_reserve,
                &grid_scroll,
                in_side_panel,
            )
        };

        let measuring = this.clone();
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(div().size_full().child(body))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(HEADER_HEIGHT + 36.0))
                            .child(ScrollChromeShadowUi::new(Self::header_scrim_style(&theme))),
                    )
                    .child(Self::header(this, &theme, pack.as_ref(), cx)),
            )
            .child(Self::rename_alert(this, pack, kind))
            .child(measure("sticker-pack-sheet-size", move |bounds, _window, cx| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                measuring.update(cx, |sheet, cx| {
                    tracing::warn!(
                        target: "riseonly::X1410",
                        width,
                        height,
                        in_side_panel = sheet.presents_in_side_panel,
                        shows_footer = Self::shows_footer(sheet.store.read(cx), cx),
                        "pack sheet bounds"
                    );
                    if sheet.width != width || sheet.height != height {
                        sheet.width = width;
                        sheet.height = height;
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }

    fn grid(
        pack: Option<&StickerPack>,
        stickers: Vec<StickerItem>,
        kind: PackKind,
        is_active: bool,
        (width, height): (f32, f32),
        footer_reserve: f32,
        grid_scroll: &UniformListScrollHandle,
        in_side_panel: bool,
    ) -> AnyElement {
        if width <= 0.0 || height <= 0.0 {
            return div().into_any_element();
        }
        let columns = column_count(kind);
        let spacing = grid_spacing(kind);
        let cell = sticker_cell_size(kind, width);

        let add_slot = pack.filter(|pack| pack.can_add_stickers()).cloned().map(StickerPackGridSlot::Add);
        let slots: Vec<StickerPackGridSlot> =
            add_slot.into_iter().chain(stickers.into_iter().map(StickerPackGridSlot::Sticker)).collect();
        let rows: Rc<Vec<Vec<StickerPackGridSlot>>> =
            Rc::new(slots.chunks(columns).map(<[StickerPackGridSlot]>::to_vec).collect());
        let row_count = rows.len();

        uniform_list("sticker-pack-sheet-grid", row_count, move |range, _window, _cx| {
            range
                .filter_map(|row| rows.get(row))
                .map(|row| Self::grid_row(row, spacing, cell))
                .collect::<Vec<AnyElement>>()
        })
        .w(px(width))
        .h(px(height))
        .px(px(GRID_HORIZONTAL_INSET))
        .pt(px(HEADER_HEIGHT + GRID_TOP_INSET))
        .pb(px(GRID_BOTTOM_SPACER + footer_reserve))
        .track_scroll(grid_scroll)
        .when(is_active, |list| list.overflow_hidden())
        .sticker_hold_grid(
            StickerHoldGridModifier::new(
                "sticker-pack-sheet-hold",
                Rc::new(move |entry: &StickerHoldEntry, window: &mut Window, cx: &mut App| {
                    if in_side_panel {
                        send_to_chat_beside(entry, window, cx);
                    }
                }),
            )
            .menu_for(Rc::new(sticker_menu)),
        )
        .into_any_element()
    }

    fn grid_row(row: &[StickerPackGridSlot], spacing: f32, cell: f32) -> AnyElement {
        hstack(spacing)
            .h(px(cell + spacing))
            .items_start()
            .children(row.iter().map(|slot| match slot {
                StickerPackGridSlot::Add(pack) => {
                    let composer_pack = pack.clone();
                    StickerPackAddCell::new(
                        ElementId::Name(format!("sticker-pack-sheet-add/{}", pack.id).into()),
                        pack.pack_kind,
                        cell,
                        move |_, cx| {
                            StickerAssetComposerStore::shared(cx)
                                .update(cx, |composer, cx| composer.present(composer_pack.clone(), cx));
                        },
                    )
                    .into_any_element()
                }
                StickerPackGridSlot::Sticker(item) => ChatInputStickerCell::new(item.clone(), cell).into_any_element(),
            }))
            .into_any_element()
    }

    fn header_scrim_style(theme: &AppTheme) -> ScrollChromeShadowStyle {
        ScrollChromeShadowStyle {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: HEADER_HEIGHT + 36.0,
            overflow_below_header: 0.0,
            peak_opacity: 1.0,
            color: theme.bg._100,
            mid_location: 0.62,
            mid_opacity_factor: 0.82,
            top_plateau_end: 0.2,
            top_plateau_opacity_factor: 1.0,
            ..ScrollChromeShadowStyle::default()
        }
    }

    fn header(
        this: &Entity<Self>,
        theme: &AppTheme,
        pack: Option<&StickerPack>,
        cx: &mut App,
    ) -> AnyElement {
        let closing = this.read(cx).store.clone();
        div()
            .relative()
            .w_full()
            .px(px(14.))
            .py(px(10.))
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(
                        PageHeaderIconButton::new("sticker-pack-sheet-close", "xmark", move |_, cx| {
                            closing.update(cx, |store, cx| store.dismiss(cx));
                        })
                        .font_size(15.),
                    )
                    .child(Self::actions_menu(this, theme, pack, cx))
                    .child(
                        div().absolute().inset_0().px(px(56.)).flex().items_center().justify_center().child(
                            MainText::new(pack.map_or_else(String::new, |pack| pack.title.clone()))
                                .px(16.)
                                .color(theme.text.primary)
                                .number_of_lines(1)
                                .truncation(TruncationMode::Tail),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn actions_menu_glyph(theme: &AppTheme, disabled: bool) -> AnyElement {
        PageHeaderToolbarSlot::new(div().children(IconUi::sized("ellipsis", px(17.), theme.text.primary, false)))
            .liquid_press("sticker-pack-sheet-menu-press", LiquidPressShape::Circle, true, disabled, true, None)
    }

    fn actions_menu(this: &Entity<Self>, theme: &AppTheme, pack: Option<&StickerPack>, cx: &App) -> AnyElement {
        let label = Self::actions_menu_glyph(theme, pack.is_none());
        let Some(pack) = pack else {
            return div().opacity(0.4).child(label).into_any_element();
        };
        div()
            .id("sticker-pack-sheet-menu-hint")
            .tooltip_label(tr("more"))
            .child(
                ContextMenuUi::new("sticker-pack-sheet-menu", Self::pack_menu_actions(this, pack, cx), label)
                    .shows_item_icons(),
            )
            .into_any_element()
    }

    fn pack_menu_actions(this: &Entity<Self>, pack: &StickerPack, cx: &App) -> Vec<ContextMenuItem> {
        let share_pack = pack.clone();
        let copy_pack = pack.clone();
        let mut items = vec![
            ContextMenuItem::new(
                0,
                tr("share_action"),
                Some("square.and.arrow.up".to_owned()),
                "share",
                move |_, cx| {
                    let link = share_pack.link.clone();
                    let title = share_pack.title.clone();
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_sticker_pack(link, title, cx));
                },
            ),
            ContextMenuItem::new(1, tr("copy_link"), Some("link".to_owned()), "copy_link", move |_, cx| {
                Self::copy_link(&copy_pack, cx);
            }),
        ];
        if pack.can_edit_pack() {
            let renaming = this.clone();
            let rename_from = pack.title.clone();
            items.push(ContextMenuItem::new(2, tr("edit_action"), Some("pencil".to_owned()), "edit", move |_, cx| {
                let title = rename_from.clone();
                renaming.update(cx, |sheet, cx| {
                    sheet.rename_title = title;
                    sheet.show_rename = true;
                    cx.notify();
                });
            }));
        }
        let pack_id = pack.id.clone();
        if pack.is_creator {
            let delete_pack = pack.clone();
            let deleting = this.clone();
            items.push(
                ContextMenuItem::new(3, tr("delete_action"), Some("trash".to_owned()), "delete", move |_, cx| {
                    let pack = delete_pack.clone();
                    let store = deleting.read(cx).store.clone();
                    NativeConfirmAlert::present_delete(
                        &tr("delete_pack_confirm_title"),
                        &tr("delete_pack_confirm_message"),
                        Some(tr("delete_action")),
                        None,
                        false,
                        None,
                        None,
                        move |cx: &mut App| Self::delete_pack(&pack, &store, cx),
                        cx,
                    );
                })
                .destructive(true),
            );
            items.push(
                Self::collaboration_item(
                    this,
                    4,
                    "sticker_pack_coauthors_title",
                    "person.2",
                    "coauthors",
                    RootRoute::StickerPackCoauthors { pack_id: pack_id.clone() },
                )
                .begins_section(true),
            );
            items.push(Self::collaboration_item(
                this,
                5,
                "sticker_pack_invite_links_title",
                "link.badge.plus",
                "invite_links",
                RootRoute::StickerPackInviteLinks { pack_id: pack_id.clone() },
            ));
            items.push(Self::collaboration_item(
                this,
                6,
                "sticker_pack_requests_title",
                "person.badge.clock",
                "requests",
                RootRoute::StickerPackJoinRequests { pack_id },
            ));
        } else if pack.is_coauthor() {
            items.push(
                Self::collaboration_item(
                    this,
                    4,
                    "sticker_pack_coauthors_title",
                    "person.2",
                    "coauthors",
                    RootRoute::StickerPackCoauthors { pack_id },
                )
                .begins_section(true),
            );
        } else if let Some(owner) = Self::reportable_owner_id(this, pack, cx) {
            items.push(
                ContextMenuItem::new(
                    7,
                    tr("ctx_report"),
                    Some("exclamationmark.bubble".to_owned()),
                    "report",
                    move |_, cx| {
                        let target = ReportTarget::User(owner.clone());
                        let note = format!("sticker pack {pack_id}");
                        ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, Some(note), cx));
                    },
                )
                .destructive(true)
                .begins_section(true),
            );
        }
        items
    }

    fn collaboration_item(
        this: &Entity<Self>,
        id: i64,
        title_key: &str,
        icon: &str,
        key: &str,
        route: RootRoute,
    ) -> ContextMenuItem {
        let opening = this.clone();
        ContextMenuItem::new(id, tr(title_key), Some(icon.to_owned()), key, move |_, cx| {
            Self::open_collaboration(&opening, route.clone(), cx);
        })
    }

    /// someone elses pack is reported as its owner with the pack named,
    /// default packs have no owner to report
    fn reportable_owner_id(this: &Entity<Self>, pack: &StickerPack, cx: &App) -> Option<String> {
        if pack.is_default || pack.is_creator || pack.is_coauthor() {
            return None;
        }
        let owner = this.read(cx).store.read(cx).owner_id(&pack.id)?;
        let self_id =
            rise_ui::shared::try_read::<AuthServicesStore>(cx).and_then(AuthServicesStore::effective_user_id_for_wire);
        (self_id.as_deref() != Some(owner.as_str())).then_some(owner)
    }

    fn open_collaboration(this: &Entity<Self>, route: RootRoute, cx: &mut App) {
        let (store, nav_store) = {
            let sheet = this.read(cx);
            (sheet.store.clone(), sheet.nav_store.clone())
        };
        store.update(cx, |store, cx| store.dismiss(cx));
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn rename_alert(this: &Entity<Self>, pack: Option<StickerPack>, kind: PackKind) -> AnyElement {
        let is_presented = Binding::entity(
            this.clone(),
            |sheet: &Self| sheet.show_rename,
            |sheet, value, cx| {
                sheet.show_rename = value;
                cx.notify();
            },
        );
        let text = Binding::entity(
            this.clone(),
            |sheet: &Self| sheet.rename_title.clone(),
            |sheet, value, cx| {
                sheet.rename_title = value;
                cx.notify();
            },
        );
        let renaming = this.clone();
        TextFieldAlert::new(
            "sticker-pack-sheet-rename",
            is_presented,
            tr("edit_action"),
            tr(if kind == PackKind::Emoji { "emoji_pack_name_placeholder" } else { "sticker_pack_name_placeholder" }),
            title_rule().binding(text),
            tr("cancel"),
            tr("save"),
            move |_, cx| Self::rename_pack(&renaming, pack.as_ref(), cx),
        )
        .into_any_element()
    }

    fn empty_state(theme: &AppTheme, system_image: &str, text: String) -> AnyElement {
        vstack(10.)
            .w_full()
            .items_center()
            .pt(px(HEADER_HEIGHT + 48.))
            .children(IconUi::sized(system_image, px(34.), theme.text.secondary, false))
            .child(MainText::new(text).px(14.).tac(TextAlignT::Center).color(theme.text.secondary))
            .into_any_element()
    }

    fn footer(this: &Entity<Self>, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let store = this.read(cx).store.clone();
        let (pack, count, has_loaded) = {
            let store = store.read(cx);
            if !Self::shows_footer(store, cx) {
                return div().h(px(0.)).into_any_element();
            }
            let Some(pack) = store.resolved_pack(cx) else {
                return div().h(px(0.)).into_any_element();
            };
            (pack, store.stickers(cx).len(), store.has_loaded_stickers(cx))
        };
        let locked = is_locked(&pack, cx);
        let text = if locked { tr("sticker_pack_premium_unlock") } else { Self::footer_title(&pack, count) };
        let color = if pack.is_saved && !locked { theme.text.primary } else { theme.content_on(theme.primary._100) };

        let label = div()
            .w_full()
            .py(px(16.))
            .flex()
            .justify_center()
            .rounded_full()
            .overflow_hidden()
            .child(
                div()
                    .opacity(if locked || has_loaded { 1.0 } else { 0.0 })
                    .child(MainText::new(text).px(15.).font_weight(FontWeight::SEMIBOLD).color(color)),
            );
        let surface = if pack.is_saved && !locked {
            label.liquid_chrome_capsule()
        } else {
            label.bg(theme.primary._100).into_any_element()
        };
        let pressed = pack;
        let measured = this.downgrade();
        div()
            .w_full()
            .relative()
            .px(px(14.))
            .py(px(12.))
            .child(measure("sticker-pack-sheet-footer-height", move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                let _ = measured.update(cx, |sheet, cx| {
                    if (sheet.footer_height - height).abs() > 0.5 {
                        sheet.footer_height = height;
                        cx.notify();
                    }
                });
            }))
            .child(surface.liquid_press(
                "sticker-pack-sheet-footer",
                LiquidPressShape::Capsule,
                true,
                false,
                false,
                Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                    if is_locked(&pressed, cx) {
                        let reason = tr(if pressed.pack_kind == PackKind::Emoji {
                            "premium_upsell_reason_emoji_pack"
                        } else {
                            "premium_upsell_reason_sticker_pack"
                        });
                        let source = format!("sticker_pack:{}", pressed.id);
                        PremiumUpsellStore::shared(cx)
                            .update(cx, |store, cx| store.offer_premium_feature(reason, &source, false, cx));
                    } else {
                        Self::toggle_saved(&pressed, cx);
                    }
                })),
            ))
            .into_any_element()
    }

    fn footer_title(pack: &StickerPack, count: usize) -> String {
        let plural_count = i64::try_from(count).unwrap_or(i64::MAX);
        match (pack.is_saved, pack.pack_kind) {
            (true, PackKind::Emoji) => tr("remove_pack_count_emoji").replace("{n}", &count.to_string()),
            (false, PackKind::Emoji) => tr("add_pack_count_emoji").replace("{n}", &count.to_string()),
            (true, _) => tr_plural("remove_pack_count_sticker", plural_count),
            (false, _) => tr_plural("add_pack_count_sticker", plural_count),
        }
    }

    fn toggle_saved(pack: &StickerPack, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        let _ = selection_changed();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            if pack.is_saved {
                interactions.remove_saved_pack(&user_id, &pack.id, pack.pack_kind, Some(pack.clone()), cx);
            } else {
                interactions.save_pack(&user_id, &pack.id, pack.pack_kind, Some(pack.clone()), cx);
            }
        });
    }

    fn delete_pack(pack: &StickerPack, store: &Entity<StickerPackSheetStore>, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.delete_pack(&user_id, &pack.id, pack.pack_kind, cx);
        });
        store.update(cx, |store, cx| store.dismiss(cx));
    }

    fn rename_pack(this: &Entity<Self>, pack: Option<&StickerPack>, cx: &mut App) {
        let title = trimming_whitespaces_and_newlines(&this.read(cx).rename_title).to_owned();
        let Some(pack) = pack else {
            return;
        };
        if title.is_empty() || title == pack.title {
            return;
        }
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.rename_pack(&user_id, &pack.id, pack.pack_kind, &title, cx);
        });
    }

    fn copy_link(pack: &StickerPack, cx: &mut App) {
        let link = RiseonlyPublicLink::StickerPack { slug: pack.link.clone() };
        if let Some(url) = link.web_url(AppEnvironment::current()) {
            UIPasteboard::general().set_url(&url, cx);
        }
        Notifier::copied(cx);
    }
}

impl Render for StickerPackSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        let store = self.store.clone();
        let in_side_panel = self.presents_in_side_panel;
        let visible = Binding::entity(
            store,
            move |store: &StickerPackSheetStore| store.is_presented() && !in_side_panel,
            move |store, value, cx| {
                if !value && !in_side_panel {
                    store.dismiss(cx);
                }
            },
        );
        let this = cx.entity();
        let host_visible = visible.clone();
        ExperimentalBottomSheetPresentationHost::new("sticker-pack-sheet-host", visible, move |_, cx| {
            Self::sheet(&this, &host_visible, cx)
        })
    }
}

#[derive(IntoElement)]
struct StickerPackSkeleton {
    columns: usize,
    cell_size: f32,
    top_padding: f32,
    spacing: f32,
}

impl RenderOnce for StickerPackSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let rows = SKELETON_CELL_COUNT.div_ceil(self.columns);
        let mut remaining = SKELETON_CELL_COUNT;
        let mut grid = vstack(self.spacing).w_full().px(px(GRID_HORIZONTAL_INSET)).pt(px(self.top_padding));
        for _ in 0..rows {
            let in_row = remaining.min(self.columns);
            remaining -= in_row;
            grid = grid.child(
                hstack(self.spacing)
                    .children((0..in_row).map(|_| SkeletonBlock::new(self.cell_size, self.cell_size, 12.0))),
            );
        }
        div().size_full().overflow_hidden().child(grid.skeleton_shimmer(None))
    }
}
