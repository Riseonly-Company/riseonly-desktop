use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{App, BorrowAppContext, ElementId, Global, IntoElement, RenderOnce, Styled, Window, WindowAppearance, WindowId, div};
use rise_platform::window_chrome::set_system_controls_appearance;

use crate::swiftui::lifecycle::{LifecycleModifiers, on_change};

/// interface style of the background under the window controls
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UserInterfaceStyle {
    #[default]
    Unspecified,
    Light,
    Dark,
}

/// system controls for a screen whose background ignores the app theme (qr wallpaper, black feed)
/// on desktop those are the window own controls over the screen
/// explicit scheme while visible, is_active false hands back to the theme
#[derive(IntoElement)]
pub struct StatusBarStyleAnchor {
    id: ElementId,
    style: UserInterfaceStyle,
    is_active: bool,
}

impl StatusBarStyleAnchor {
    pub fn new(id: impl Into<ElementId>, style: UserInterfaceStyle) -> Self {
        Self {
            id: id.into(),
            style,
            is_active: true,
        }
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }
}

/// identity of one mounted anchor
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AnchorOwner(u64);

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

impl RenderOnce for StatusBarStyleAnchor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let owner_key = ElementId::NamedChild(Arc::new(self.id.clone()), "status-bar-owner".into());
        let owner = *window
            .use_keyed_state(owner_key, cx, |_, _| AnchorOwner(NEXT_OWNER.fetch_add(1, Ordering::Relaxed)))
            .read(cx);
        let change_key = ElementId::NamedChild(Arc::new(self.id.clone()), "status-bar-style".into());
        on_change(
            change_key,
            (self.style, self.is_active),
            false,
            window,
            cx,
            move |_, (style, is_active), window, cx| sync(owner, *style, *is_active, window, cx),
        );
        let (style, is_active) = (self.style, self.is_active);
        div()
            .absolute()
            .size_0()
            .on_appear(self.id, move |window, cx| sync(owner, style, is_active, window, cx))
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    StatusBarStyleOverlay::release(owner, window, cx);
                }
            })
    }
}

fn sync(owner: AnchorOwner, style: UserInterfaceStyle, is_active: bool, window: &mut Window, cx: &mut App) {
    if !is_active || style == UserInterfaceStyle::Unspecified {
        StatusBarStyleOverlay::release(owner, window, cx);
        return;
    }
    StatusBarStyleOverlay::claim(owner, style, window, cx);
}

/// one shared claim list per window for every anchor, the latest claim decides the controls
/// the drawn titlebar observes this global and reads style
#[derive(Default)]
pub struct StatusBarStyleOverlay {
    claims: HashMap<WindowId, Vec<(AnchorOwner, UserInterfaceStyle)>>,
}

impl Global for StatusBarStyleOverlay {}

impl StatusBarStyleOverlay {
    /// scheme the window controls take now, none when they follow the theme
    pub fn style(window: WindowId, cx: &App) -> Option<UserInterfaceStyle> {
        cx.try_global::<Self>()
            .and_then(|overlay| overlay.claims.get(&window))
            .and_then(|claims| claims.last())
            .map(|(_, style)| *style)
    }

    fn claim(owner: AnchorOwner, style: UserInterfaceStyle, window: &mut Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        cx.default_global::<Self>();
        cx.update_global::<Self, _>(|overlay, _| {
            let claims = overlay.claims.entry(id).or_default();
            claims.retain(|(claimed, _)| *claimed != owner);
            claims.push((owner, style));
        });
        Self::apply(window, cx);
    }

    fn release(owner: AnchorOwner, window: &mut Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let Some(overlay) = cx.try_global::<Self>() else {
            return;
        };
        let holds = overlay
            .claims
            .get(&id)
            .is_some_and(|claims| claims.iter().any(|(claimed, _)| *claimed == owner));
        if !holds {
            return;
        }
        cx.update_global::<Self, _>(|overlay, _| {
            if let Some(claims) = overlay.claims.get_mut(&id) {
                claims.retain(|(claimed, _)| *claimed != owner);
                if claims.is_empty() {
                    overlay.claims.remove(&id);
                }
            }
        });
        Self::apply(window, cx);
    }

    fn apply(window: &mut Window, cx: &mut App) {
        let appearance = Self::style(window.window_handle().window_id(), cx).and_then(|style| match style {
            UserInterfaceStyle::Light => Some(WindowAppearance::Light),
            UserInterfaceStyle::Dark => Some(WindowAppearance::Dark),
            UserInterfaceStyle::Unspecified => None,
        });
        set_system_controls_appearance(window, appearance);
        window.refresh();
    }
}
