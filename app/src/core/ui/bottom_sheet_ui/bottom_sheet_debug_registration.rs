#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, ElementId, Empty, Hsla, RenderOnce, Window};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;

type DebugCancel = Rc<dyn Fn()>;
type DebugMakeView = Rc<dyn Fn(Binding<bool>, &mut Window, &mut App) -> AnyElement>;
type DebugContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BottomSheetDebugRegistrationKind {
    Sheet,
    Modal,
    Alert,
}

impl BottomSheetDebugRegistrationKind {
    pub const ALL_CASES: [Self; 3] = [Self::Sheet, Self::Modal, Self::Alert];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Sheet => "sheet",
            Self::Modal => "modal",
            Self::Alert => "alert",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title(self) -> String {
        tr(&format!("debug_surface_{}", self.raw_value()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BottomSheetDebugRegistrationMode {
    Mock,
    Live,
}

impl BottomSheetDebugRegistrationMode {
    pub const ALL_CASES: [Self; 2] = [Self::Mock, Self::Live];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Live => "live",
        }
    }

    pub fn title(self) -> String {
        tr(if self == Self::Mock { "debug_sheet_mock" } else { "debug_sheet_live" })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BottomSheetDebugRegistrationModule {
    Limits,
    Other,
    Chats,
    Messages,
    Music,
    Stickers,
    Profile,
    Premium,
    Auth,
    Bots,
    Map,
    Notifications,
    Media,
    Moderation,
    Diagnostics,
    Posts,
    Comments,
    Stories,
    Calls,
    Shorts,
    Jobs,
}

impl BottomSheetDebugRegistrationModule {
    pub const ALL_CASES: [Self; 21] = [
        Self::Limits,
        Self::Other,
        Self::Chats,
        Self::Messages,
        Self::Music,
        Self::Stickers,
        Self::Profile,
        Self::Premium,
        Self::Auth,
        Self::Bots,
        Self::Map,
        Self::Notifications,
        Self::Media,
        Self::Moderation,
        Self::Diagnostics,
        Self::Posts,
        Self::Comments,
        Self::Stories,
        Self::Calls,
        Self::Shorts,
        Self::Jobs,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Limits => "limits",
            Self::Other => "other",
            Self::Chats => "chats",
            Self::Messages => "messages",
            Self::Music => "music",
            Self::Stickers => "stickers",
            Self::Profile => "profile",
            Self::Premium => "premium",
            Self::Auth => "auth",
            Self::Bots => "bots",
            Self::Map => "map",
            Self::Notifications => "notifications",
            Self::Media => "media",
            Self::Moderation => "moderation",
            Self::Diagnostics => "diagnostics",
            Self::Posts => "posts",
            Self::Comments => "comments",
            Self::Stories => "stories",
            Self::Calls => "calls",
            Self::Shorts => "shorts",
            Self::Jobs => "jobs",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    const fn title_key(self) -> &'static str {
        match self {
            Self::Limits => "debug_surface_limits",
            Self::Other => "other",
            Self::Chats => "chats_title",
            Self::Messages => "chat_search_section_messages",
            Self::Music => "navbtn_music",
            Self::Stickers => "settings_stickers_title",
            Self::Profile => "navbtn_profile",
            Self::Premium => "subscription_plan_active",
            Self::Auth => "weblogin_approve",
            Self::Bots => "global_search_section_bots",
            Self::Map => "brand_maps",
            Self::Notifications => "ctx_notifications",
            Self::Media => "chat_profile_tab_media",
            Self::Moderation => "settings_moderations_title",
            Self::Diagnostics => "report_riseonly_title",
            Self::Posts => "navbtn_posts",
            Self::Comments => "navbtn_comments",
            Self::Stories => "archive_tab_stories",
            Self::Calls => "bug_report_surface_call",
            Self::Shorts => "bug_report_surface_shorts",
            Self::Jobs => "bug_report_surface_vacancy",
        }
    }

    pub fn title(self) -> String {
        tr(self.title_key())
    }

    /// symbol name for this item, the catalog resolves it through the kit symbol map
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Limits => "gauge.with.dots.needle.100percent",
            Self::Other => "square.grid.2x2",
            Self::Chats => "bubble.left.and.bubble.right",
            Self::Messages => "text.bubble",
            Self::Music => "music.note",
            Self::Stickers => "face.smiling",
            Self::Profile => "person.crop.circle",
            Self::Premium => "diamond",
            Self::Auth => "key",
            Self::Bots => "terminal",
            Self::Map => "map",
            Self::Notifications => "bell",
            Self::Media => "photo.on.rectangle",
            Self::Moderation => "checkmark.shield",
            Self::Diagnostics => "ladybug",
            Self::Posts => "rectangle.stack",
            Self::Comments => "quote.bubble",
            Self::Stories => "circle.dashed",
            Self::Calls => "phone",
            Self::Shorts => "play.rectangle",
            Self::Jobs => "briefcase",
        }
    }
}

/// cold launch factory, never a captured live sheet or its account bindings
#[derive(Clone)]
pub struct BottomSheetDebugRegistration {
    pub id: String,
    pub module: BottomSheetDebugRegistrationModule,
    pub title_key: String,
    pub variant_key: Option<String>,
    pub kind: BottomSheetDebugRegistrationKind,
    pub mode: BottomSheetDebugRegistrationMode,
    pub source: String,
    pub cancel: DebugCancel,
    pub make_view: DebugMakeView,
}

impl BottomSheetDebugRegistration {
    pub fn title(&self) -> String {
        tr(&self.title_key)
    }

    pub fn variant(&self) -> Option<String> {
        self.variant_key.as_deref().map(tr)
    }

    /// constructor, variant mode kind and on_cancel keep defaults until the builder methods set them
    pub fn new<E: IntoElement>(
        id: impl Into<String>,
        module: BottomSheetDebugRegistrationModule,
        title: impl Into<String>,
        source: impl Into<String>,
        content: impl Fn(Binding<bool>, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            module,
            title_key: title.into(),
            variant_key: None,
            kind: BottomSheetDebugRegistrationKind::Sheet,
            mode: BottomSheetDebugRegistrationMode::Mock,
            source: source.into(),
            cancel: Rc::new(|| {}),
            make_view: Rc::new(move |open, window, cx| content(open, window, cx).into_any_element()),
        }
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant_key = variant;
        self
    }

    pub fn with_mode(mut self, mode: BottomSheetDebugRegistrationMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_kind(mut self, kind: BottomSheetDebugRegistrationKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn on_cancel(mut self, on_cancel: impl Fn() + 'static) -> Self {
        self.cancel = Rc::new(on_cancel);
        self
    }

    pub fn native<E: IntoElement>(
        id: impl Into<String>,
        module: BottomSheetDebugRegistrationModule,
        title: impl Into<String>,
        source: impl Into<String>,
        content: impl Fn(Binding<bool>, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        let id: String = id.into();
        let host_id = ElementId::from(id.clone());
        let content = Rc::new(content);
        Self::new(id, module, title, source, move |open: Binding<bool>, _, _| {
            let content = content.clone();
            let content_open = open.clone();
            NativeBottomSheetPresentationHost::new(host_id.clone(), open, move |window, cx| {
                content(content_open.clone(), window, cx)
            })
            .presentation_detents([PresentationDetent::Medium, PresentationDetent::Large])
            .presentation_drag_indicator(true)
        })
    }

    /// for content whose production caller owns the sheet shell
    pub fn sheet<E: IntoElement>(
        id: impl Into<String>,
        module: BottomSheetDebugRegistrationModule,
        title: impl Into<String>,
        source: impl Into<String>,
        fractions: Vec<f32>,
        background: Option<Hsla>,
        content: impl Fn(Binding<bool>, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        let id: String = id.into();
        let host_id = ElementId::from(id.clone());
        let content = Rc::new(content);
        Self::new(id, module, title, source, move |open: Binding<bool>, _, _| {
            let content = content.clone();
            let content_open = open.clone();
            BottomSheetDebugContentHost {
                id: host_id.clone(),
                is_presented: open,
                fractions: fractions.clone(),
                background,
                content: Rc::new(move |window, cx| {
                    content(content_open.clone(), window, cx).into_any_element()
                }),
            }
        })
    }
}

#[derive(IntoElement)]
struct BottomSheetDebugContentHost {
    id: ElementId,
    is_presented: Binding<bool>,
    fractions: Vec<f32>,
    background: Option<Hsla>,
    content: DebugContent,
}

impl RenderOnce for BottomSheetDebugContentHost {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let surface = theme_store.read(cx).current_theme().bg._100;
        let content = self.content.clone();
        let is_presented = self.is_presented.clone();
        BottomSheetUi::new(
            self.id,
            self.is_presented,
            move |window, cx| content(window, cx),
            |_, _| Empty,
        )
        .snap_fractions(self.fractions)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(self.background.unwrap_or(surface)))
        .on_presentation_refused(move |window, cx| is_presented.set(false, window, cx))
    }
}

impl BottomSheetUi {
    pub fn debug_registration(
        id: impl Into<String>,
        module: BottomSheetDebugRegistrationModule,
        title: impl Into<String>,
        source: impl Into<String>,
        make: impl Fn(Binding<bool>, &mut Window, &mut App) -> BottomSheetUi + 'static,
    ) -> BottomSheetDebugRegistration {
        BottomSheetDebugRegistration::new(id, module, title, source, make)
    }
}
