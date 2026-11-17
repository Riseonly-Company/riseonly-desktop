use std::rc::Rc;

use gpui::{AnyElement, App, Context, Entity, IntoElement, Window};

use crate::swiftui::prelude::*;

type ScreenContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// one page of the sheet built in paged navigation
/// the page declares its height fraction so the target exists before the first frame of motion
/// content is a closure so unmounted pages arent built
#[derive(Clone)]
pub struct BottomSheetScreenItem {
    pub id: String,
    pub title: String,
    pub fraction: f32,
    pub content: ScreenContent,
}

impl BottomSheetScreenItem {
    pub fn new<E: IntoElement>(
        id: impl Into<String>,
        title: impl Into<String>,
        fraction: f32,
        content: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            fraction: fraction.clamp(0.01, 1.0),
            content: Rc::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }
}

type NavigateAction = Rc<dyn Fn(String, &mut Window, &mut App)>;
type GoBackAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// entry point into paged navigation from page content
/// lives in the environment so a nested screen can navigate without knowing the sheet
/// use bottom_sheet_navigation(cx) then navigate(id)
#[derive(Default)]
pub struct BottomSheetNavigationState {
    current_screen_id: String,
    can_go_back: bool,

    on_navigate: Option<NavigateAction>,
    on_go_back: Option<GoBackAction>,
}

impl BottomSheetNavigationState {
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self::default())
    }

    pub fn current_screen_id(&self) -> &str {
        &self.current_screen_id
    }

    pub fn can_go_back(&self) -> bool {
        self.can_go_back
    }

    pub fn set_on_navigate(&mut self, action: impl Fn(String, &mut Window, &mut App) + 'static) {
        self.on_navigate = Some(Rc::new(action));
    }

    pub fn set_on_go_back(&mut self, action: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_go_back = Some(Rc::new(action));
    }

    /// callback runs outside the entity borrow, it usually updates the deck which syncs this state back
    pub fn navigate(
        this: &Entity<Self>,
        screen_id: impl Into<String>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let action = this.read(cx).on_navigate.clone();
        if let Some(action) = action {
            action(screen_id.into(), window, cx);
        }
    }

    pub fn go_back(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let action = this.read(cx).on_go_back.clone();
        if let Some(action) = action {
            action(window, cx);
        }
    }

    pub fn sync(&mut self, current_screen_id: &str, can_go_back: bool, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.current_screen_id != current_screen_id {
            self.current_screen_id = current_screen_id.to_owned();
            changed = true;
        }
        if self.can_go_back != can_go_back {
            self.can_go_back = can_go_back;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}

pub struct BottomSheetNavigationStateKey;

impl EnvKey for BottomSheetNavigationStateKey {
    type Value = Option<Entity<BottomSheetNavigationState>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// gets the paged navigation state from the environment
pub fn bottom_sheet_navigation(cx: &App) -> Option<Entity<BottomSheetNavigationState>> {
    env::<BottomSheetNavigationStateKey>(cx)
}
