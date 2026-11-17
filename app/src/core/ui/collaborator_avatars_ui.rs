use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, px,
};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::{MainText, PressableUi, TextAlignT};
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{Alignment, hstack, zstack};

use crate::core::ui::user_logo_ui::UserLogoUi;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CollaboratorAvatar {
    pub id: String,
    pub logo: Option<String>,
    pub name: String,
    pub tag: String,
}

impl CollaboratorAvatar {
    pub fn new(id: impl Into<String>, logo: Option<String>, name: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            logo,
            name: name.into(),
            tag: tag.into(),
        }
    }
}

pub type CollaboratorAvatarAction = Rc<dyn Fn(&CollaboratorAvatar, &mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CollaboratorNamesLineUiSegment {
    Literal(String),
    Author,
    Coauthor,
    More(String),
}

/// one line of names for a co authored post
/// author then first co author then and N more, each name opens its profile and the tail the list
/// words around come from the locale template so order differs per language
#[derive(IntoElement)]
pub struct CollaboratorNamesLineUi {
    id: ElementId,
    author: CollaboratorAvatar,
    coauthor: CollaboratorAvatar,
    total_count: i64,
    px: f32,
    font_weight: FontWeight,
    color: Option<Hsla>,
    on_person: CollaboratorAvatarAction,
    on_more: ButtonAction,
}

impl CollaboratorNamesLineUi {
    pub fn new(
        id: impl Into<ElementId>,
        author: CollaboratorAvatar,
        coauthor: CollaboratorAvatar,
        total_count: i64,
    ) -> Self {
        Self {
            id: id.into(),
            author,
            coauthor,
            total_count,
            px: 16.0,
            font_weight: FontWeight::NORMAL,
            color: None,
            on_person: Rc::new(|_, _, _| {}),
            on_more: Rc::new(|_, _| {}),
        }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    pub fn color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.color = color.into();
        self
    }

    pub fn on_person(mut self, on_person: impl Fn(&CollaboratorAvatar, &mut Window, &mut App) + 'static) -> Self {
        self.on_person = Rc::new(on_person);
        self
    }

    pub fn on_more(mut self, on_more: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_more = Rc::new(on_more);
        self
    }

    fn text(&self, text: impl Into<SharedString>) -> MainText {
        MainText::new(text)
            .px(self.px)
            .font_weight(self.font_weight)
            .color(self.color)
            .number_of_lines(1)
    }

    fn name_button(&self, person: &CollaboratorAvatar, index: usize) -> AnyElement {
        let label = if person.name.is_empty() {
            format!("@{}", person.tag)
        } else {
            person.name.clone()
        };
        let on_person = self.on_person.clone();
        let target = person.clone();
        plain(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::from(format!("name-{index}"))))
            .min_w_0()
            .on_press(move |window, cx| on_person(&target, window, cx))
            .child(self.text(label))
            .into_any_element()
    }

    fn segments(&self) -> Vec<CollaboratorNamesLineUiSegment> {
        let remaining = self.total_count - 2;
        let template = if remaining > 0 {
            tr("post_coauthor_names_more")
        } else {
            tr("post_coauthor_names_two")
        };
        let (Some(author_start), Some(coauthor_start)) = (template.find("%1$@"), template.find("%2$@")) else {
            return Self::fallback_segments();
        };
        if author_start >= coauthor_start {
            return Self::fallback_segments();
        }
        let author_end = author_start + "%1$@".len();
        let coauthor_end = coauthor_start + "%2$@".len();
        let mut result = Vec::new();
        let head = &template[..author_start];
        if !head.is_empty() {
            result.push(CollaboratorNamesLineUiSegment::Literal(head.to_owned()));
        }
        result.push(CollaboratorNamesLineUiSegment::Author);
        let middle = &template[author_end..coauthor_start];
        if !middle.is_empty() {
            result.push(CollaboratorNamesLineUiSegment::Literal(middle.to_owned()));
        }
        result.push(CollaboratorNamesLineUiSegment::Coauthor);
        let tail = &template[coauthor_end..];
        if remaining > 0 {
            let text = tail.replace("%3$d", &remaining.to_string());
            if !text.is_empty() {
                result.push(CollaboratorNamesLineUiSegment::More(text));
            }
        } else if !tail.is_empty() {
            result.push(CollaboratorNamesLineUiSegment::Literal(tail.to_owned()));
        }
        result
    }

    fn fallback_segments() -> Vec<CollaboratorNamesLineUiSegment> {
        vec![
            CollaboratorNamesLineUiSegment::Author,
            CollaboratorNamesLineUiSegment::Literal(", ".to_owned()),
            CollaboratorNamesLineUiSegment::Coauthor,
        ]
    }
}

impl RenderOnce for CollaboratorNamesLineUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let segments = self.segments().into_iter().map(|segment| match segment {
            CollaboratorNamesLineUiSegment::Literal(text) => div().flex_none().child(self.text(text)).into_any_element(),
            CollaboratorNamesLineUiSegment::Author => self.name_button(&self.author, 0),
            CollaboratorNamesLineUiSegment::Coauthor => self.name_button(&self.coauthor, 1),
            CollaboratorNamesLineUiSegment::More(text) => {
                let on_more = self.on_more.clone();
                plain(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "more".into()))
                    .flex_none()
                    .on_press(move |window, cx| on_more(window, cx))
                    .child(self.text(text))
                    .into_any_element()
            }
        });
        let children: Vec<AnyElement> = segments.collect();
        hstack(0.0).min_w_0().max_w_full().overflow_hidden().children(children)
    }
}

/// author stack for one post, author plus co authors inside the box one avatar occupied
/// face type isnt domain on purpose, feed riffs and profile all use it
/// while the post collaborator user lives in the post module
#[derive(IntoElement)]
pub struct CollaboratorAvatarsUi {
    id: ElementId,
    author: CollaboratorAvatar,
    coauthors: Vec<CollaboratorAvatar>,
    total_count: i64,
    size: f32,
    ring_color: Option<Hsla>,
    on_press: ButtonAction,
    on_long_press: Option<ButtonAction>,
}

impl CollaboratorAvatarsUi {
    pub fn new(id: impl Into<ElementId>, author: CollaboratorAvatar) -> Self {
        Self {
            id: id.into(),
            author,
            coauthors: Vec::new(),
            total_count: 1,
            size: 40.0,
            ring_color: None,
            on_press: Rc::new(|_, _| {}),
            on_long_press: None,
        }
    }

    pub fn coauthors(mut self, coauthors: Vec<CollaboratorAvatar>) -> Self {
        self.coauthors = coauthors;
        self
    }

    pub fn total_count(mut self, total_count: i64) -> Self {
        self.total_count = total_count;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn ring_color(mut self, ring_color: impl Into<Option<Hsla>>) -> Self {
        self.ring_color = ring_color.into();
        self
    }

    pub fn on_press(mut self, on_press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    pub fn on_long_press(mut self, on_long_press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_long_press = Some(Rc::new(on_long_press));
        self
    }

    fn stack(&self, theme: &AppTheme) -> AnyElement {
        let ring = self.ring_color.unwrap_or(theme.bg._100);
        let people: Vec<&CollaboratorAvatar> = std::iter::once(&self.author).chain(self.coauthors.iter()).collect();
        let size = self.size;
        if people.len() == 1 {
            return div()
                .size(px(size))
                .child(self.avatar(people[0], 0, size, ring, false))
                .into_any_element();
        }
        if people.len() == 2 {
            let diameter = size * 0.68;
            let shift = size - diameter;
            return zstack(Alignment::TopLeading)
                .size(px(size))
                .child(Self::placed(self.avatar(people[0], 0, diameter, ring, true), 0.0, 0.0))
                .child(Self::placed(self.avatar(people[1], 1, diameter, ring, true), shift, shift))
                .into_any_element();
        }
        let diameter = size * 0.6;
        let shift = size - diameter;
        let third = if self.total_count > 3 {
            self.overflow_bubble(diameter, ring, theme)
        } else {
            self.avatar(people[2], 2, diameter, ring, true)
        };
        zstack(Alignment::TopLeading)
            .size(px(size))
            .child(Self::placed(self.avatar(people[0], 0, diameter, ring, true), shift / 2.0, 0.0))
            .child(Self::placed(self.avatar(people[1], 1, diameter, ring, true), 0.0, shift))
            .child(Self::placed(third, shift, shift))
            .into_any_element()
    }

    /// moves the drawing without moving the layout slot
    fn placed(child: AnyElement, x: f32, y: f32) -> AnyElement {
        div().relative().left(px(x)).top(px(y)).child(child).into_any_element()
    }

    fn avatar(&self, face: &CollaboratorAvatar, index: usize, diameter: f32, ring: Hsla, shows_ring: bool) -> AnyElement {
        let id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::from(format!("avatar-{index}")));
        let mut avatar = div().relative().size(px(diameter)).child(
            UserLogoUi::new(id)
                .source(face.logo.clone().map(SharedString::from))
                .name(Some(SharedString::from(face.name.clone())))
                .user_id(Some(SharedString::from(face.id.clone())))
                .size(diameter)
                .show_border(!shows_ring)
                .show_story_ring(false)
                .open_story_on_press(false),
        );
        if shows_ring {
            avatar = avatar.child(centred_stroke(ring, 1.5, Shape::Circle));
        }
        avatar.into_any_element()
    }

    fn overflow_bubble(&self, diameter: f32, ring: Hsla, theme: &AppTheme) -> AnyElement {
        let count = (self.total_count - 2).max(1);
        zstack(Alignment::Center)
            .relative()
            .size(px(diameter))
            .child(div().size_full().circle().bg(theme.bg._200))
            .child(
                MainText::new(format_printf(&tr("post_collaborator_plus_count"), &[FormatArg::Int(count)]))
                    .px((diameter * 0.38).max(8.0))
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
            .child(centred_stroke(ring, 1.5, Shape::Circle))
            .into_any_element()
    }

    /// names line under the stack, %1$@ is the author
    /// %2$@ the first co author
    /// %3$d is the count of people besides the author
    pub fn names_line(author: &str, coauthors: &[String], total_count: i64) -> String {
        let Some(first) = coauthors.first().filter(|first| !first.is_empty()) else {
            return author.to_owned();
        };
        if total_count <= 2 {
            return format_printf(
                &tr("post_coauthor_names_two"),
                &[FormatArg::from(author), FormatArg::from(first)],
            );
        }
        format_printf(
            &tr("post_coauthor_names_more"),
            &[FormatArg::from(author), FormatArg::from(first), FormatArg::Int((total_count - 2).max(1))],
        )
    }
}

impl RenderOnce for CollaboratorAvatarsUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().clone()
        };
        let on_press = self.on_press.clone();
        let mut pressable = PressableUi::new(self.id.clone())
            .tap_scale(0.96)
            .on_press(move |window, cx| on_press(window, cx));
        if let Some(on_long_press) = self.on_long_press.clone() {
            pressable = pressable.on_long_press(move |window, cx| on_long_press(window, cx));
        }
        pressable.child(self.stack(&theme))
    }
}
