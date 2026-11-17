use gpui::{
    App, Bounds, Context, ElementId, Entity, FontWeight, Hsla, Pixels, ScrollHandle, SharedString, Subscription,
    Window, point, white,
};
use rise_canon::{RiseGuideCategory, RiseGuideIcon, RiseGuideTopic};
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::guides::engine::guide_runner::GuideRunner;
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::stores::guides_interactions::guides_interactions::GuidesInteractionsStore;
use crate::modules::guides::stores::guides_services::guides_services::GuidesServicesStore;

fn pop_on_back(navigation_store: Option<Entity<NavigationStore>>) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_, cx| {
        if let Some(navigation_store) = navigation_store.as_ref() {
            navigation_store.update(cx, |navigation_store, cx| navigation_store.pop(cx));
        }
    }
}

/// scroller of both pages, shared so both draw the same list under the shell chrome
#[derive(IntoElement)]
struct GuidesCatalogueScrollBody {
    id: ElementId,
    scroll_handle: Option<ScrollHandle>,
    content: Vec<AnyElement>,
}

impl RenderOnce for GuidesCatalogueScrollBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let scroller = scroll_view(self.id);
        let scroller = match self.scroll_handle {
            Some(scroll_handle) => scroller.scroll_handle(scroll_handle),
            None => scroller,
        };
        scroller.list_shell_scroll_under_chrome(cx).bg(background).child(
            vstack(20.0)
                .w_full()
                .pt(px(12.0))
                .children(self.content)
                .child(spacer_min_h(80.0)),
        )
    }
}

/// every guide this account can run, replayable in two levels
/// canon categories then one category sections with guides
/// guides sent without a section land in a trailing group on the first level
pub struct GuidesCatalogue {
    services: Entity<GuidesServicesStore>,
    interactions: Entity<GuidesInteractionsStore>,
    state: GuidesCatalogueState,
    _services: Subscription,
}

impl GuidesCatalogue {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = GuidesServicesStore::shared(cx);
        let interactions = GuidesInteractionsStore::shared(cx);
        let state = GuidesCatalogueState::read(&services, cx);
        let subscription = cx.observe(&services, |page, services, cx| {
            if page.state.adopt(&services, cx) {
                cx.notify();
            }
        });
        Self { services, interactions, state, _services: subscription }
    }

    fn auto_start_card(&self) -> GroupedBtns {
        let row = GroupedBtn::new("guide_auto_start", tr("guide_auto_start_title"))
            .icon("sparkles")
            .icon_background(ChatTileColor::purple())
            .icon_foreground(white())
            .subtitle(tr("guide_auto_start_subtitle"))
            .trailing_slot(SwitchUi::new("guide_auto_start_switch", self.auto_start_binding()));
        let services = self.services.clone();
        let interactions = self.interactions.clone();
        GroupedBtns::new(
            "guide_auto_start_card",
            vec![row.callback(move |_, cx| {
                let enabled = !services.read(cx).auto_start(cx);
                interactions.update(cx, |interactions, cx| interactions.set_auto_start(enabled, cx));
            })],
        )
    }

    fn auto_start_binding(&self) -> Binding<bool> {
        let services = self.services.clone();
        let interactions = self.interactions.clone();
        Binding::new(
            move |cx| services.read(cx).auto_start(cx),
            move |enabled, _, cx| {
                interactions.update(cx, |interactions, cx| interactions.set_auto_start(enabled, cx));
            },
        )
    }

    fn category_row(
        category: RiseGuideCategory,
        shelf: &GuideCatalogueShelf,
        navigation_store: Option<Entity<NavigationStore>>,
    ) -> GroupedBtn {
        GroupedBtn::new(category.raw_value(), tr(category.i18n_key()))
            .icon(category.system_image())
            .icon_background(category.tile_color())
            .icon_foreground(white())
            .trailing(tr_plural("guide_catalogue_count", shelf.count(category)))
            .callback(move |_, cx| {
                if let Some(navigation_store) = navigation_store.as_ref() {
                    navigation_store.update(cx, |navigation_store, cx| {
                        navigation_store.push(
                            RootRoute::GuidesCategory { category: category.raw_value().to_string() },
                            NavigationStoreSettlingPush::Drop,
                            cx,
                        );
                    });
                }
            })
    }

    fn empty_state(theme: &AppTheme) -> AnyElement {
        vstack(10.0)
            .children(IconUi::sized("book.closed", px(34.0), theme.text.secondary, false))
            .child(
                div().w_full().child(
                    MainText::new(tr("guide_catalogue_empty"))
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            )
            .w_full()
            .py(px(60.0))
            .px(px(16.0))
            .into_any_element()
    }

    fn start_row(interactions: &Entity<GuidesInteractionsStore>) -> impl Fn(String, &mut App) + 'static {
        let interactions = interactions.clone();
        move |key, cx| {
            interactions.update(cx, |interactions, cx| interactions.start_from_catalogue(key, cx));
        }
    }
}

impl Render for GuidesCatalogue {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let refresh = self.interactions.clone();
        use_task("guides-catalogue-refresh", (), window, cx, async move |cx| {
            cx.update(|_, cx| refresh.update(cx, |interactions, cx| interactions.refresh(true, cx))).ok();
        });

        let theme = rise_ui::theme(cx).clone();
        let navigation_store = env::<NavigationStoreKey>(cx);
        let shelf = &self.state.shelf;
        let is_running = self.state.is_running;

        let mut content = vec![self.auto_start_card().into_any_element()];
        if shelf.guides.is_empty() {
            content.push(Self::empty_state(&theme));
        } else {
            let categories = shelf.categories();
            if !categories.is_empty() {
                let items = categories
                    .into_iter()
                    .map(|category| Self::category_row(category, shelf, navigation_store.clone()))
                    .collect();
                content.push(GroupedBtns::new("guide_catalogue_categories", items).into_any_element());
            }
            let unsectioned = shelf.unsectioned();
            if !unsectioned.is_empty() {
                let items = unsectioned
                    .into_iter()
                    .map(|guide| {
                        GuideCatalogueRow::item(
                            guide,
                            is_running,
                            &theme,
                            Self::start_row(&self.interactions),
                            None,
                        )
                    })
                    .collect();
                content.push(
                    GroupedBtns::new("guide_catalogue_unsectioned", items)
                        .title(tr("guide_catalogue_other").to_uppercase())
                        .into_any_element(),
                );
            }
        }

        SettingsWrapperUi::new(tr("guide_catalogue_title"), pop_on_back(navigation_store)).content(
            GuidesCatalogueScrollBody { id: "guides-catalogue-scroll".into(), scroll_handle: None, content },
        )
    }
}

/// one category, its sections in canon order each a card of guides
pub struct GuidesCategoryPage {
    category_wire: String,
    interactions: Entity<GuidesInteractionsStore>,
    scroll_handle: ScrollHandle,
    return_row_key: Option<String>,
    state: GuidesCatalogueState,
    _services: Subscription,
}

impl GuidesCategoryPage {
    pub fn new(category_wire: String, cx: &mut Context<Self>) -> Self {
        let services = GuidesServicesStore::shared(cx);
        let interactions = GuidesInteractionsStore::shared(cx);
        let state = GuidesCatalogueState::read(&services, cx);
        let subscription = cx.observe(&services, |page, services, cx| {
            if page.state.adopt(&services, cx) {
                cx.notify();
            }
        });
        Self {
            category_wire,
            interactions,
            scroll_handle: ScrollHandle::new(),
            return_row_key: None,
            state,
            _services: subscription,
        }
    }

    fn category(&self) -> Option<RiseGuideCategory> {
        RiseGuideCategory::from_raw(&self.category_wire)
    }

    fn return_to_started_row(&mut self, cx: &mut Context<Self>) {
        let Some(category) = self.category() else {
            return;
        };
        let keys: Vec<String> = self
            .state
            .shelf
            .topics(category)
            .into_iter()
            .flat_map(|topic| self.state.shelf.guides(topic))
            .map(|guide| guide.key.clone())
            .collect();
        let key = self.interactions.update(cx, |interactions, _| interactions.take_return_row(&keys));
        if key.is_some() {
            self.return_row_key = key;
            cx.notify();
        }
    }

    fn center_row(scroll_handle: &ScrollHandle, row: Bounds<Pixels>) {
        let viewport = scroll_handle.bounds();
        let offset = scroll_handle.offset();
        let row_center = f32::from(row.origin.y) + f32::from(row.size.height) / 2.0;
        let in_content = row_center - f32::from(viewport.origin.y) - f32::from(offset.y);
        let target = in_content - f32::from(viewport.size.height) / 2.0;
        let clamped = target.min(f32::from(scroll_handle.max_offset().y)).max(0.0);
        scroll_handle.set_offset(point(offset.x, px(-clamped)));
    }

    fn return_row_frame(&self, cx: &Context<Self>) -> GuideCatalogueRowFrame {
        let page = cx.weak_entity();
        let scroll_handle = self.scroll_handle.clone();
        Box::new(move |row: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
            Self::center_row(&scroll_handle, row);
            page.update(cx, |page, cx| {
                page.return_row_key = None;
                cx.notify();
            })
            .ok();
        })
    }

    fn topic_card(
        &self,
        topic: RiseGuideTopic,
        shelf: &GuideCatalogueShelf,
        is_running: bool,
        theme: &AppTheme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let items = shelf
            .guides(topic)
            .into_iter()
            .map(|guide| {
                let row_frame = (self.return_row_key.as_deref() == Some(guide.key.as_str()))
                    .then(|| self.return_row_frame(cx));
                GuideCatalogueRow::item(
                    guide,
                    is_running,
                    theme,
                    GuidesCatalogue::start_row(&self.interactions),
                    row_frame,
                )
            })
            .collect();
        GroupedBtns::new(SharedString::from(format!("guide_topic_{}", topic.raw_value())), items)
            .title(tr(topic.i18n_key()).to_uppercase())
            .into_any_element()
    }
}

impl Render for GuidesCategoryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let navigation_store = env::<NavigationStoreKey>(cx);
        let category = self.category();
        let shelf = &self.state.shelf;
        let is_running = self.state.is_running;
        let title = match category {
            Some(category) => tr(category.i18n_key()),
            None => tr("guide_catalogue_title"),
        };
        let content = match category {
            Some(category) => shelf
                .topics(category)
                .into_iter()
                .map(|topic| self.topic_card(topic, shelf, is_running, &theme, cx))
                .collect(),
            None => Vec::new(),
        };

        let page = cx.weak_entity();
        let wire = &self.category_wire;
        SettingsWrapperUi::new(title, pop_on_back(navigation_store)).content(
            GuidesCatalogueScrollBody {
                id: SharedString::from(format!("guides-category-scroll-{wire}")).into(),
                scroll_handle: Some(self.scroll_handle.clone()),
                content,
            }
            .on_appear(SharedString::from(format!("guides-category-appear-{wire}")), move |_, cx| {
                page.update(cx, |page, cx| page.return_to_started_row(cx)).ok();
            }),
        )
    }
}

/// what the two pages draw from the guides store, kept on the page
/// the store notifies on every snapshot and runner step so only a change of what is drawn
/// renders the page
#[derive(PartialEq)]
struct GuidesCatalogueState {
    shelf: GuideCatalogueShelf,
    is_running: bool,
    auto_start: bool,
}

impl GuidesCatalogueState {
    fn read(services: &Entity<GuidesServicesStore>, cx: &App) -> Self {
        let services = services.read(cx);
        Self {
            shelf: GuideCatalogueShelf::new(services.catalogue(cx)),
            is_running: services.is_running(cx),
            auto_start: services.auto_start(cx),
        }
    }

    /// takes the store current state, true when the page has to render it
    fn adopt(&mut self, services: &Entity<GuidesServicesStore>, cx: &App) -> bool {
        let next = Self::read(services, cx);
        if next == *self {
            return false;
        }
        *self = next;
        true
    }
}

/// catalogue grouped the way the two pages draw it, built once per store change not per render
#[derive(PartialEq)]
pub struct GuideCatalogueShelf {
    pub guides: Vec<RiseGuide>,
}

impl GuideCatalogueShelf {
    /// the jobs lessons leave with the switched off vacancies section
    pub fn new(guides: Vec<RiseGuide>) -> Self {
        let guides = if RiseFeatureGate::VACANCIES {
            guides
        } else {
            guides.into_iter().filter(|guide| guide.topic != Some(RiseGuideTopic::ServicesJobs)).collect()
        };
        Self { guides }
    }

    pub fn categories(&self) -> Vec<RiseGuideCategory> {
        RiseGuideCategory::ALL.into_iter().filter(|category| self.count(*category) > 0).collect()
    }

    pub fn topics(&self, category: RiseGuideCategory) -> Vec<RiseGuideTopic> {
        RiseGuideTopic::ALL
            .into_iter()
            .filter(|topic| topic.category() == category.raw_value() && !self.guides(*topic).is_empty())
            .collect()
    }

    pub fn guides(&self, topic: RiseGuideTopic) -> Vec<&RiseGuide> {
        self.guides.iter().filter(|guide| guide.topic == Some(topic)).collect()
    }

    pub fn count(&self, category: RiseGuideCategory) -> i64 {
        self.guides
            .iter()
            .filter(|guide| guide.topic.is_some_and(|topic| topic.category() == category.raw_value()))
            .count() as i64
    }

    /// guides the server sent without a section this build can resolve
    pub fn unsectioned(&self) -> Vec<&RiseGuide> {
        self.guides.iter().filter(|guide| guide.topic.is_none()).collect()
    }
}

pub type GuideCatalogueRowFrame = Box<dyn FnOnce(Bounds<Pixels>, &mut Window, &mut App)>;

pub enum GuideCatalogueRow {}

impl GuideCatalogueRow {
    pub fn item(
        guide: &RiseGuide,
        disabled: bool,
        theme: &AppTheme,
        start: impl Fn(String, &mut App) + 'static,
        row_frame: Option<GuideCatalogueRowFrame>,
    ) -> GroupedBtn {
        let category = guide.topic.and_then(|topic| RiseGuideCategory::from_raw(topic.category()));
        let symbol = guide
            .icon
            .map(|icon| icon.system_image())
            .or_else(|| category.map(|category| category.system_image()))
            .unwrap_or("book.closed.fill");
        let tile = guide
            .icon
            .map(|icon| icon.tile_color())
            .or_else(|| category.map(|category| category.tile_color()))
            .unwrap_or_else(ChatTileColor::gray);
        let key = guide.key.clone();
        let row = GroupedBtn::new(guide.key.clone(), guide.title.clone())
            .icon(symbol)
            .icon_background(tile)
            .icon_foreground(white())
            .subtitle_line_limit(2)
            .disabled(disabled)
            .trailing_slot(
                div()
                    .relative()
                    .child(
                        MainText::new(tr("guide_start"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    )
                    .children(row_frame.map(|row_frame| {
                        measure(SharedString::from(format!("guide-return-row-{}", guide.key)), row_frame)
                    })),
            )
            .show_chevron_when_trailing_slot(false)
            .callback(move |_, cx| start(key.clone(), cx));
        let summary = GuideRunner::desktop_summary(guide);
        if summary.is_empty() { row } else { row.subtitle(summary) }
    }
}

pub trait RiseGuideCategoryPlatformLook {
    fn system_image(self) -> &'static str;
    fn tile_color(self) -> Hsla;
}

impl RiseGuideCategoryPlatformLook for RiseGuideCategory {
    fn system_image(self) -> &'static str {
        match self {
            Self::Music => "music.note",
            Self::Chats => "bubble.left.and.bubble.right.fill",
            Self::Account => "person.crop.circle.fill",
            Self::Maps => "map.fill",
            Self::Posts => "square.grid.2x2.fill",
            Self::Profile => "gearshape.fill",
            Self::Services => "briefcase.fill",
        }
    }

    fn tile_color(self) -> Hsla {
        match self {
            Self::Music => ChatTileColor::pink(),
            Self::Chats => ChatTileColor::blue(),
            Self::Account => ChatTileColor::indigo(),
            Self::Maps => ChatTileColor::green(),
            Self::Posts => ChatTileColor::orange(),
            Self::Profile => ChatTileColor::gray(),
            Self::Services => ChatTileColor::teal(),
        }
    }
}

pub trait RiseGuideIconPlatformLook {
    fn system_image(self) -> &'static str;
    fn tile_color(self) -> Hsla;
}

impl RiseGuideIconPlatformLook for RiseGuideIcon {
    fn system_image(self) -> &'static str {
        match self {
            Self::BellSlash => "bell.slash.fill",
            Self::EnvelopeBadge => "envelope.badge.fill",
            Self::FolderPlus => "folder.fill.badge.plus",
            Self::Headphones => "headphones",
            Self::Link => "link",
            Self::ListBullet => "list.bullet.rectangle.portrait.fill",
            Self::Microphone => "mic.fill",
            Self::PersonBadgeKey => "person.badge.key.fill",
            Self::PersonBadgePlus => "person.crop.circle.badge.plus",
            Self::Person3 => "person.3.fill",
            Self::PersonWave => "person.wave.2.fill",
            Self::Playlist => "music.note.list",
            Self::Waveform => "waveform",
            Self::ChartBar => "chart.bar.fill",
            Self::Eraser => "eraser.fill",
            Self::FaceSmiling => "face.smiling.inverse",
            Self::House => "house.fill",
            Self::MapPin => "mappin.and.ellipse",
            Self::Route => "point.topleft.down.to.point.bottomright.curvepath.fill",
            Self::Tag => "tag.fill",
            Self::Lock => "lock.fill",
            Self::HandRaised => "hand.raised.fill",
            Self::Photo => "photo.fill",
            Self::Bubbles => "bubble.left.and.bubble.right.fill",
            Self::PhotoSwap => "photo.on.rectangle.angled",
            Self::Star => "star.fill",
            Self::Highlight => "circle.dashed.inset.filled",
            Self::Archive => "archivebox.fill",
            Self::Person2 => "person.2.fill",
            Self::SquarePencil => "square.and.pencil",
            Self::MusicNote => "music.note",
            Self::Wand => "wand.and.stars",
            Self::Paintpalette => "paintpalette.fill",
            Self::Bolt => "bolt.fill",
            Self::Iphone => "iphone",
            Self::Target => "target",
            Self::Trophy => "trophy.fill",
            Self::Sparkles => "sparkles",
            Self::Bell => "bell.fill",
            Self::Ladybug => "ladybug.fill",
            Self::DocText => "doc.text.fill",
            Self::Paperplane => "paperplane.fill",
            Self::Briefcase => "briefcase.fill",
            Self::Cpu => "cpu",
            Self::Terminal => "terminal.fill",
            Self::Globe => "globe",
            Self::Apps => "square.grid.2x2.fill",
            Self::Map => "map.fill",
            Self::ScreenShare => "rectangle.on.rectangle",
            Self::Video => "video.fill",
        }
    }

    fn tile_color(self) -> Hsla {
        match self {
            Self::BellSlash => ChatTileColor::gray(),
            Self::EnvelopeBadge => ChatTileColor::orange(),
            Self::FolderPlus => ChatTileColor::teal(),
            Self::Headphones => ChatTileColor::green(),
            Self::Link => ChatTileColor::blue(),
            Self::ListBullet => ChatTileColor::indigo(),
            Self::Microphone => ChatTileColor::red(),
            Self::PersonBadgeKey => ChatTileColor::purple(),
            Self::PersonBadgePlus => ChatTileColor::indigo(),
            Self::Person3 => ChatTileColor::blue(),
            Self::PersonWave => ChatTileColor::purple(),
            Self::Playlist => ChatTileColor::pink(),
            Self::Waveform => ChatTileColor::purple(),
            Self::ChartBar => ChatTileColor::blue(),
            Self::Eraser => ChatTileColor::gray(),
            Self::FaceSmiling => ChatTileColor::orange(),
            Self::House => ChatTileColor::green(),
            Self::MapPin => ChatTileColor::red(),
            Self::Route => ChatTileColor::teal(),
            Self::Tag => ChatTileColor::orange(),
            Self::Lock => ChatTileColor::gray(),
            Self::HandRaised => ChatTileColor::red(),
            Self::Photo => ChatTileColor::blue(),
            Self::Bubbles => ChatTileColor::green(),
            Self::PhotoSwap => ChatTileColor::teal(),
            Self::Star => ChatTileColor::orange(),
            Self::Highlight => ChatTileColor::pink(),
            Self::Archive => ChatTileColor::gray(),
            Self::Person2 => ChatTileColor::blue(),
            Self::SquarePencil => ChatTileColor::indigo(),
            Self::MusicNote => ChatTileColor::purple(),
            Self::Wand => ChatTileColor::teal(),
            Self::Paintpalette => ChatTileColor::purple(),
            Self::Bolt => ChatTileColor::orange(),
            Self::Iphone => ChatTileColor::gray(),
            Self::Target => ChatTileColor::red(),
            Self::Trophy => ChatTileColor::orange(),
            Self::Sparkles => ChatTileColor::purple(),
            Self::Bell => ChatTileColor::red(),
            Self::Ladybug => ChatTileColor::red(),
            Self::DocText => ChatTileColor::green(),
            Self::Paperplane => ChatTileColor::blue(),
            Self::Briefcase => ChatTileColor::orange(),
            Self::Cpu => ChatTileColor::indigo(),
            Self::Terminal => ChatTileColor::gray(),
            Self::Globe => ChatTileColor::teal(),
            Self::Apps => ChatTileColor::purple(),
            Self::Map => ChatTileColor::green(),
            Self::ScreenShare => ChatTileColor::indigo(),
            Self::Video => ChatTileColor::green(),
        }
    }
}
