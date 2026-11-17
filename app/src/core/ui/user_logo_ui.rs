use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Rgba, Styled, Window, div, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::skeleton_ui::SkeletonCircle;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthroughModifiers;
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke, pass_through};
use rise_ui::swiftui::stacks::{Alignment, zstack};
use rise_ui::user_logo_story_hooks::user_logo_story_hooks;

use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};

/// callback that gets the user id whose stories to open
pub type UserLogoOpenStory = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct UserLogoUi {
    id: ElementId,
    source: Option<SharedString>,
    name: Option<SharedString>,
    user_id: Option<SharedString>,
    color_scope_id: Option<SharedString>,
    is_deleted: bool,
    size: f32,
    is_button: bool,
    show_border: bool,
    border_color: Option<Hsla>,
    show_story_ring: bool,
    story_ring_state: Option<(bool, bool)>,
    open_story_on_press: bool,
    show_avatar_context_menu: bool,
    on_open_profile: Option<ButtonAction>,
    on_open_story: Option<UserLogoOpenStory>,
    image_cache_policy: CleverImageCachePolicy,
    on_press: ButtonAction,
}

impl UserLogoUi {
    /// id keeps this avatar state (story load, press) apart from its siblings
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            source: None,
            name: None,
            user_id: None,
            color_scope_id: None,
            is_deleted: false,
            size: 40.0,
            is_button: false,
            show_border: true,
            border_color: None,
            show_story_ring: true,
            story_ring_state: None,
            open_story_on_press: true,
            show_avatar_context_menu: false,
            on_open_profile: None,
            on_open_story: None,
            image_cache_policy: CleverImageCachePolicy::Standard,
            on_press: Rc::new(|_, _| {}),
        }
    }

    pub fn source(mut self, source: impl Into<Option<SharedString>>) -> Self {
        self.source = source.into();
        self
    }

    pub fn name(mut self, name: impl Into<Option<SharedString>>) -> Self {
        self.name = name.into();
        self
    }

    pub fn user_id(mut self, user_id: impl Into<Option<SharedString>>) -> Self {
        self.user_id = user_id.into();
        self
    }

    pub fn color_scope_id(mut self, color_scope_id: impl Into<Option<SharedString>>) -> Self {
        self.color_scope_id = color_scope_id.into();
        self
    }

    pub fn is_deleted(mut self, is_deleted: bool) -> Self {
        self.is_deleted = is_deleted;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn is_button(mut self, is_button: bool) -> Self {
        self.is_button = is_button;
        self
    }

    pub fn show_border(mut self, show_border: bool) -> Self {
        self.show_border = show_border;
        self
    }

    pub fn border_color(mut self, border_color: impl Into<Option<Hsla>>) -> Self {
        self.border_color = border_color.into();
        self
    }

    pub fn show_story_ring(mut self, show_story_ring: bool) -> Self {
        self.show_story_ring = show_story_ring;
        self
    }

    /// draws the ring from this active and unseen state instead of reading the story store
    pub fn story_ring_state(mut self, has_active_stories: bool, has_unseen_stories: bool) -> Self {
        self.story_ring_state = Some((has_active_stories, has_unseen_stories));
        self
    }

    pub fn open_story_on_press(mut self, open_story_on_press: bool) -> Self {
        self.open_story_on_press = open_story_on_press;
        self
    }

    pub fn show_avatar_context_menu(mut self, show_avatar_context_menu: bool) -> Self {
        self.show_avatar_context_menu = show_avatar_context_menu;
        self
    }

    pub fn on_open_profile(mut self, on_open_profile: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open_profile = Some(Rc::new(on_open_profile));
        self
    }

    pub fn on_open_story(mut self, on_open_story: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_open_story = Some(Rc::new(on_open_story));
        self
    }

    pub fn image_cache_policy(mut self, image_cache_policy: CleverImageCachePolicy) -> Self {
        self.image_cache_policy = image_cache_policy;
        self
    }

    pub fn on_press(mut self, on_press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    fn has_active_stories(&self, cx: &mut App) -> bool {
        if self.is_deleted || !self.show_story_ring {
            return false;
        }
        match self.story_ring_state {
            Some((has_active, _)) => has_active,
            None => user_logo_story_hooks().is_some_and(|hooks| (hooks.has_active_stories)(self.user_id.as_deref(), cx)),
        }
    }

    fn has_unseen_stories(&self, cx: &mut App) -> bool {
        match self.story_ring_state {
            Some((_, has_unseen)) => has_unseen,
            None => user_logo_story_hooks().is_some_and(|hooks| (hooks.has_unseen_stories)(self.user_id.as_deref(), cx)),
        }
    }

    fn story_state_task_id(&self) -> Option<String> {
        if !self.show_story_ring {
            return None;
        }
        self.normalized_user_id()
    }

    fn normalized_user_id(&self) -> Option<String> {
        let id = trimming_whitespaces_and_newlines(self.user_id.as_deref()?);
        if id.is_empty() { None } else { Some(id.to_owned()) }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn open_story(
        user_id: String,
        on_open_story: Option<&UserLogoOpenStory>,
        on_press: &ButtonAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(on_open_story) = on_open_story {
            on_open_story(user_id, window, cx);
        } else if let Some(hooks) = user_logo_story_hooks() {
            (hooks.open_stories)(&user_id, on_press.clone(), window, cx);
        }
    }

    fn avatar_body(&self, theme: &AppTheme) -> AnyElement {
        if self.is_deleted {
            return self.deleted_placeholder(theme);
        }
        if let Some(url) = AvatarPalette::resolved_logo_url(self.source.as_deref()) {
            let size = self.size;
            return div()
                .size(px(size))
                .clip_shape(Shape::Circle)
                .child(
                    CleverImageUi::new(url)
                        .id(self.child_id("user-logo-image"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(size / 2.0)
                        .persist_to_disk(self.image_cache_policy == CleverImageCachePolicy::Standard)
                        .maximum_decode_pixel_dimension((size * 3.0).ceil() as i64)
                        .cache_policy(self.image_cache_policy),
                )
                .into_any_element();
        }
        self.letter_placeholder()
    }

    fn deleted_placeholder(&self, theme: &AppTheme) -> AnyElement {
        let glyph_side = (self.size * 0.38).max(9.0);
        zstack(Alignment::Center)
            .size(px(self.size))
            .child(div().size_full().circle().bg(theme.bg._300))
            .children(IconUi::sized("person.fill.questionmark", px(glyph_side), theme.text.secondary, true))
            .into_any_element()
    }

    fn letter_placeholder(&self) -> AnyElement {
        if trimming_whitespaces_and_newlines(self.name.as_deref().unwrap_or_default()).is_empty() {
            return SkeletonCircle::new(self.size).into_any_element();
        }
        self.letter_circle()
    }

    fn letter_circle(&self) -> AnyElement {
        let letter = AvatarPalette::display_letter(self.name.as_deref());
        let color = self.placeholder_color();
        let font_size = (self.size * 0.46).max(11.0);

        zstack(Alignment::Center)
            .size(px(self.size))
            .child(div().size_full().circle().bg(color))
            .child(
                div()
                    .size(px(self.size * 0.72))
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system_weight(font_size, FontWeight::SEMIBOLD))
                    .text_color(AvatarPalette::letter_color_on(color))
                    .child(MinimumScaleText::new(letter, 0.55).line_limit(1)),
            )
            .into_any_element()
    }

    fn placeholder_color(&self) -> Hsla {
        if let Some(owner) = AvatarPalette::generated_avatar_owner_id(self.source.as_deref()) {
            return AvatarPalette::color_for_user(Some(&owner), None);
        }
        if let Some(uid) = self.normalized_user_id() {
            return AvatarPalette::color_for_user(Some(&uid), self.name.as_deref());
        }
        if let Some(color_scope_id) = self.color_scope_id.as_deref() {
            return AvatarPalette::color_in(color_scope_id, Some("none"));
        }
        AvatarPalette::color_for_user(None, self.name.as_deref())
    }
}

impl RenderOnce for UserLogoUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().clone()
        };
        let stroke = self.border_color.unwrap_or(theme.border._100);
        let has_active_stories = self.has_active_stories(cx);
        let has_unseen_stories = has_active_stories && self.has_unseen_stories(cx);

        let story_state_task_id = self.story_state_task_id();
        use_task(
            self.child_id("user-logo-story-state"),
            story_state_task_id.clone(),
            window,
            cx,
            async move |cx| {
                let Some(id) = story_state_task_id else {
                    return;
                };
                let _ = cx.update(|_, cx| {
                    let Some(hooks) = user_logo_story_hooks() else {
                        return;
                    };
                    if (hooks.has_known_stories_state)(Some(&id), cx) {
                        return;
                    }
                    (hooks.load_stories_state)(vec![id], cx);
                });
            },
        );

        let avatar_body = self.avatar_body(&theme);
        let size = self.size;
        let mut content = if self.is_button {
            let handle_press = {
                let is_deleted = self.is_deleted;
                let open_story_on_press = self.open_story_on_press;
                let show_story_ring = self.show_story_ring;
                let user_id = self.normalized_user_id();
                let on_open_story = self.on_open_story.clone();
                let on_press = self.on_press.clone();
                move |window: &mut Window, cx: &mut App| {
                    if !is_deleted
                        && open_story_on_press
                        && show_story_ring
                        && let Some(id) = user_id.clone()
                        && let Some(hooks) = user_logo_story_hooks()
                    {
                        if (hooks.has_active_stories)(Some(&id), cx) || !(hooks.has_known_stories_state)(Some(&id), cx) {
                            let _ = selection_changed();
                            Self::open_story(id, on_open_story.as_ref(), &on_press, window, cx);
                            return;
                        }
                    }
                    on_press(window, cx);
                }
            };
            div()
                .relative()
                .size(px(size))
                .child(plain(self.child_id("user-logo-button")).on_press(handle_press).child(avatar_body))
        } else {
            div().relative().size(px(size)).child(avatar_body)
        };

        if self.show_border {
            content = content.child(centred_stroke(stroke, 0.5, Shape::Circle));
        }
        if has_active_stories {
            content = content.child(StoryRingUi::new(size, has_unseen_stories, theme.clone()));
        }

        if !self.show_avatar_context_menu {
            return content.into_any_element();
        }

        let items: Vec<ContextMenuItem> = Self::avatar_menu_actions(has_active_stories)
            .into_iter()
            .zip(0_i64..)
            .map(|(action, index)| {
                let handle = self.avatar_menu_action_handler();
                let id = action.id.clone();
                ContextMenuItem::new(index, action.title, action.system_image, action.id, move |window: &mut Window, cx: &mut App| {
                    handle(&id, window, cx)
                })
            })
            .collect();
        content
            .bubble_scroll_passthrough(self.child_id("user-logo-scroll-passthrough"))
            .context_menu(self.child_id("user-logo-context-menu"), items)
            .into_any_element()
    }
}

impl UserLogoUi {
    fn avatar_menu_actions(has_active_stories: bool) -> Vec<HoldContextMenuAction> {
        let mut actions =
            vec![HoldContextMenuAction::new("open_profile", tr("ctx_open_profile")).system_image("person.crop.circle")];
        if has_active_stories {
            actions.push(HoldContextMenuAction::new("view_story", tr("ctx_view_story")).system_image("play.circle"));
        }
        actions
    }

    fn avatar_menu_action_handler(&self) -> impl Fn(&str, &mut Window, &mut App) + 'static {
        let on_open_profile = self.on_open_profile.clone();
        let on_press = self.on_press.clone();
        let user_id = self.normalized_user_id();
        let on_open_story = self.on_open_story.clone();
        move |id: &str, window: &mut Window, cx: &mut App| match id {
            "open_profile" => match on_open_profile.as_ref() {
                Some(on_open_profile) => on_open_profile(window, cx),
                None => on_press(window, cx),
            },
            "view_story" => {
                let Some(user_id) = user_id.clone() else {
                    return;
                };
                Self::open_story(user_id, on_open_story.as_ref(), &on_press, window, cx);
            }
            _ => {}
        }
    }
}

#[derive(IntoElement)]
pub struct StoryRingUi {
    size: f32,
    has_unseen: bool,
    theme: AppTheme,
}

impl StoryRingUi {
    pub fn new(size: f32, has_unseen: bool, theme: AppTheme) -> Self {
        Self { size, has_unseen, theme }
    }
}

impl RenderOnce for StoryRingUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let line_width = (self.size * 0.026).max(1.25);
        let ring_diameter = self.size + (self.size * 0.09).max(4.0);
        let inset = (self.size - ring_diameter) / 2.0;

        let ring = if self.has_unseen {
            angular_gradient_circle_stroke(
                AngularGradient::new(vec![
                    self.theme.primary._100,
                    self.theme.primary._200,
                    self.theme.primary._300,
                    self.theme.primary._100,
                ]),
                line_width,
                ring_diameter,
            )
            .into_any_element()
        } else {
            let seen: Hsla = Rgba { r: 0.24, g: 0.24, b: 0.26, a: 1.0 }.into();
            div()
                .relative()
                .size(px(ring_diameter))
                .child(centred_stroke(seen, line_width, Shape::Circle))
                .into_any_element()
        };

        pass_through()
            .absolute()
            .left(px(inset))
            .top(px(inset))
            .size(px(ring_diameter))
            .child(ring)
    }
}
