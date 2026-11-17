use std::rc::Rc;

use gpui::{App, AppContext, Context, Entity, IntoElement, RenderOnce, Window};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStore;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::qr_code::rise_qr_code_card::RiseQRCodeSubject;
use crate::core::ui::qr_code::rise_qr_code_page::RiseQRCodePage;

pub struct RiseQRCodePresentation {
    subject: Option<RiseQRCodeSubject>,
    is_presented: bool,
    page: Option<Entity<RiseQRCodePage>>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl RiseQRCodePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { subject: None, is_presented: false, page: None, nav_store: None }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    /// nav_store is the stack the screen was opened from, the scanner it offers opens there
    pub fn present(subject: RiseQRCodeSubject, nav_store: Option<Entity<NavigationStore>>, cx: &mut App) {
        Self::shared(cx).update(cx, |presentation, cx| {
            presentation.subject = Some(subject);
            presentation.page = None;
            presentation.nav_store = nav_store;
            presentation.is_presented = true;
            cx.notify();
        });
    }

    pub fn dismiss(cx: &mut App) {
        Self::shared(cx).update(cx, |presentation, cx| {
            if !presentation.is_presented {
                return;
            }
            presentation.is_presented = false;
            cx.notify();
        });
    }

    fn did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.is_presented {
            return;
        }
        self.subject = None;
        self.page = None;
        self.nav_store = None;
        cx.notify();
    }

    fn page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<RiseQRCodePage>> {
        if let Some(page) = self.page.clone() {
            return Some(page);
        }
        let subject = self.subject.clone()?;
        let nav_store = self.nav_store.clone();
        let page = cx.new(|cx| {
            let mut page = RiseQRCodePage::new(subject, window, cx);
            page.present_as_modal(nav_store, Rc::new(Self::dismiss));
            page
        });
        self.page = Some(page.clone());
        Some(page)
    }
}

#[derive(IntoElement, Default)]
pub struct RiseQRCodeModalHost;

impl RiseQRCodeModalHost {
    const HEIGHT_FRACTION: f32 = 0.93;

    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for RiseQRCodeModalHost {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presentation = RiseQRCodePresentation::shared(cx);
        let reader = presentation.clone();
        let content = presentation.clone();
        let finished = presentation;
        BottomSheetUi::new(
            "rise-qr-code-modal",
            Binding::new(
                move |cx| reader.read(cx).is_presented(),
                move |presented, _, cx| {
                    if !presented {
                        RiseQRCodePresentation::dismiss(cx);
                    }
                },
            ),
            move |window, cx| {
                let page = content.update(cx, |presentation, cx| presentation.page(window, cx));
                div().size_full().children(page)
            },
            |_, _| div(),
        )
        .snap_fractions(vec![Self::HEIGHT_FRACTION])
        .content_is_edge_to_edge(true)
        .content_under_handle(true)
        .full_page_hides_footer(true)
        .on_did_dismiss(move |_, cx| finished.update(cx, |presentation, cx| presentation.did_dismiss(cx)))
    }
}
