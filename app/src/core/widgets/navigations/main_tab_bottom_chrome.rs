use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::app::router::main_tab_chrome::MainTabChrome;
use crate::core::widgets::navigations::bottom_chrome_insets::{BottomChromeInsetsModifiers, BottomChromeOwner};
use crate::modules::chat::components::chat::video_note::chat_video_note_player_bar::ChatVoiceFloatingPlayerBar;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;

#[derive(IntoElement)]
pub struct MainTabBottomChrome {
    hides_mini_player: bool,
}

/// what stands in the player slot, a change is what gets animated
#[derive(Clone, Debug, PartialEq, Eq)]
enum MainTabBottomChromeSlot {
    Voice,
}

struct MainTabBottomChromeState {
    slot: Presence<MainTabBottomChromeSlot>,
}

impl MainTabBottomChrome {
    pub fn new() -> Self {
        Self { hides_mini_player: false }
    }

    pub fn hides_mini_player(mut self, hides_mini_player: bool) -> Self {
        self.hides_mini_player = hides_mini_player;
        self
    }

    fn slot(&self, cx: &mut App) -> Option<MainTabBottomChromeSlot> {
        if self.hides_mini_player {
            return None;
        }
        ChatVoiceMessagePlayer::shared(cx).read(cx).is_active_in_chat().then_some(MainTabBottomChromeSlot::Voice)
    }
}

impl Default for MainTabBottomChrome {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MainTabBottomChrome {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let wanted = self.slot(cx);
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::new_static("main-tab-bottom-chrome")),
            cx,
            |_, _| MainTabBottomChromeState {
                slot: Presence::new(None)
                    .with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
            },
        );
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            let present = if state.slot.is_present() { state.slot.value(now).cloned() } else { None };
            if present != wanted {
                state.slot.set_in(wanted, spring(0.35, 0.85), now, cx);
            }
        });

        let content = state.read(cx).slot.render_transitioned(now, window, |slot| {
            let player = match slot {
                MainTabBottomChromeSlot::Voice => ChatVoiceFloatingPlayerBar::new().into_any_element(),
            };
            div().pb(px(MainTabChrome::MINI_PLAYER_GAP_ABOVE_TAB_BAR)).child(player)
        });

        div()
            .flex()
            .flex_col()
            .w_full()
            .px(px(MainTabChrome::FLOATING_BAR_HORIZONTAL_INSET))
            .child(div().w_full().children(content).reports_bottom_chrome(BottomChromeOwner::TabBarMiniPlayer))
    }
}
