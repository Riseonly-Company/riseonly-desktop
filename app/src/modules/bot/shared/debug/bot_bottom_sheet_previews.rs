#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, Window};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotCommandModel, BotMiniAppModel};
use crate::modules::chat::components::chat::bot_chat_menu_sheet::{BotChatMenuPopoverMetrics, BotChatMenuSheet};

pub enum BotBottomSheetPreviews {}

impl BotBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![BottomSheetDebugRegistration::new(
            "bots.menu",
            BottomSheetDebugRegistrationModule::Bots,
            "bot_menu",
            "BotChatMenuSheet",
            |open: Binding<bool>, _: &mut Window, _: &mut App| {
                let close_on_command = open.clone();
                let close_on_mini_app = open.clone();
                BotChatMenuSheet {
                    id: "bot-preview-chat-menu".into(),
                    commands: vec![
                        BotCommandModel {
                            command: "start".to_owned(),
                            description: "Welcome to the design bot".to_owned(),
                        },
                        BotCommandModel {
                            command: "help".to_owned(),
                            description: "Discover commands and features".to_owned(),
                        },
                        BotCommandModel { command: "settings".to_owned(), description: "Your preferences".to_owned() },
                    ],
                    mini_apps: Vec::new(),
                    has_more_mini_apps: false,
                    is_loading_more_mini_apps: false,
                    pagination_cache_key: "debug-preview".to_owned(),
                    relative_id: None,
                    status: SaiWsSliceStatus::Fulfilled,
                    error: None,
                    starts_expanded: false,
                    on_command: Rc::new(move |_: &BotCommandModel, window: &mut Window, cx: &mut App| {
                        close_on_command.set(false, window, cx);
                    }),
                    on_mini_app: Rc::new(move |_: &BotMiniAppModel, window: &mut Window, cx: &mut App| {
                        close_on_mini_app.set(false, window, cx);
                    }),
                    on_retry: Rc::new(|_: &mut Window, _: &mut App| {}),
                    on_load_more_mini_apps: Rc::new(|_: &mut Window, _: &mut App| {}),
                    is_presented: open,
                    bottom_inset: 0.0,
                    leading_inset: 0.0,
                    available_height: BotChatMenuPopoverMetrics::MAX_HEIGHT,
                }
            },
        )]
    }
}
