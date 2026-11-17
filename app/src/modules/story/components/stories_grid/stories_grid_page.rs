use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::story::components::stories_grid::stories_grid_shared_element_origin::StoriesGridSharedElementOrigin;
use crate::modules::story::components::stories_grid::stories_grid_view::{
    StoriesGridPermissions, StoriesGridStateUi, StoriesGridView,
};
use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SingleStoryConfirmationKind {
    Archive,
    Unarchive,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SingleStoryConfirmation {
    kind: SingleStoryConfirmationKind,
    story_id: i64,
}

/// one stories grid wired to the store, shared by profile tab and chat profile tab and archive
/// they differ only in scope and viewer
#[derive(IntoElement)]
pub struct StoriesGridPage {
    pub owner_user_id: String,
    pub is_owner: bool,
    pub scope: StoryProfileScope,
    pub empty_icon: String,
    pub empty_text: String,
}

impl StoriesGridPage {
    pub fn new(owner_user_id: impl Into<String>, is_owner: bool) -> Self {
        Self {
            owner_user_id: owner_user_id.into(),
            is_owner,
            scope: StoryProfileScope::Profile,
            empty_icon: "play.rectangle.on.rectangle".to_owned(),
            empty_text: tr("profile_stories_empty"),
        }
    }

    pub fn scope(mut self, scope: StoryProfileScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn empty_icon(mut self, empty_icon: impl Into<String>) -> Self {
        self.empty_icon = empty_icon.into();
        self
    }

    pub fn empty_text(mut self, empty_text: impl Into<String>) -> Self {
        self.empty_text = empty_text.into();
        self
    }

    fn id(&self) -> ElementId {
        ElementId::Name(SharedString::from(format!(
            "stories-grid-page-{}-{}",
            self.owner_user_id,
            self.scope.raw_value()
        )))
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id()), SharedString::new_static(name))
    }

    fn alert(confirmation: SingleStoryConfirmation, owner_user_id: &str, cx: &mut App) {
        let owner = owner_user_id.to_owned();
        let story_id = confirmation.story_id;
        match confirmation.kind {
            SingleStoryConfirmationKind::Archive => NativeConfirmAlert::present_delete(
                &tr("story_archive_confirm_title"),
                &tr("story_archive_confirm_message"),
                Some(tr("story_action_archive")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.archive_stories(&[story_id], true, Some(&owner), None, cx);
                    });
                },
                cx,
            ),
            SingleStoryConfirmationKind::Unarchive => NativeConfirmAlert::present_delete(
                &tr("story_unarchive_confirm_title"),
                &tr("story_unarchive_confirm_message"),
                Some(tr("story_action_unarchive")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.archive_stories(&[story_id], false, Some(&owner), None, cx);
                    });
                },
                cx,
            ),
            SingleStoryConfirmationKind::Delete => NativeConfirmAlert::present_delete(
                &tr("story_delete_confirm_title"),
                &tr("story_delete_confirm_message"),
                Some(tr("ctx_delete")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.delete_story(story_id, Some(&owner), cx);
                    });
                },
                cx,
            ),
        }
    }

    fn present_pin_limit(cx: &mut App) {
        let limit = StoryInteractionsStore::pinned_stories_limit(cx);
        NativeConfirmAlert::present_notice(
            &tr("story_pin_limit_title"),
            &tr_args("story_pin_limit_message", &[FormatArg::Int(limit)]),
            None,
            None,
            |_| {},
            cx,
        );
    }
}

impl RenderOnce for StoriesGridPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let owner_user_id = self.owner_user_id.clone();
        let scope = self.scope;
        let service = StoryServicesStore::shared(cx);
        let items = Rc::new(service.update(cx, |service, cx| service.profile_stories(&owner_user_id, scope, cx)));
        let permissions = StoriesGridPermissions::new(self.is_owner, scope);

        let return_scroller = env::<SharedElementReturnScrollerKey>(cx);
        let shared_element_origin = window
            .use_keyed_state(self.child_id("origin"), cx, |_, _| StoriesGridSharedElementOrigin::new())
            .read(cx)
            .clone();
        let active_shared_element_origin = (SharedElementNavigationPolicy::stories_enabled()
            && scope == StoryProfileScope::Profile
            && return_scroller.is_some()
            && !owner_user_id.is_empty())
        .then(|| shared_element_origin.clone());
        shared_element_origin.configure(&owner_user_id, return_scroller);
        shared_element_origin.record_latest_stories(&items);

        let has_active = service.read(cx).has_active_stories(Some(&owner_user_id), cx);
        let active_owner = owner_user_id.clone();
        on_change(self.child_id("has-active"), has_active, false, window, cx, move |_, active, _, cx| {
            if scope != StoryProfileScope::Profile || active_owner.is_empty() {
                return;
            }
            let force = !*active;
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_profile_stories(Some(&active_owner), scope, force, cx);
            });
        });

        let task_owner = owner_user_id.clone();
        use_task(self.child_id("load"), (owner_user_id.clone(), scope), window, cx, async move |cx| {
            if task_owner.is_empty() {
                return;
            }
            if scope == StoryProfileScope::Profile {
                let surface = StoriesGridSharedElementOrigin::surface(&task_owner);
                let settled = cx.update(|_, cx| SharedElementTransitionCoordinator::wait_until_settled(surface, 1.0, cx));
                if let Ok(settled) = settled {
                    settled.await;
                }
            }
            let _ = cx.update(|_, cx| {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_profile_stories(Some(&task_owner), scope, scope == StoryProfileScope::Archive, cx);
                });
            });
        });

        let is_loaded = trimming_whitespaces_and_newlines(&owner_user_id).is_empty()
            || service.update(cx, |service, cx| service.profile_stories_loaded(&owner_user_id, scope, cx));
        let has_more = service.update(cx, |service, cx| service.profile_stories_has_more(&owner_user_id, scope, cx));
        let is_empty = items.is_empty();

        let nav_store = env::<NavigationStoreKey>(cx);
        let content = {
            let owner = owner_user_id.clone();
            let items = items.clone();
            move |_: &mut Window, _: &mut App| {
                let load_owner = owner.clone();
                let load_origin = active_shared_element_origin.clone();
                let open_owner = owner.clone();
                let open_origin = active_shared_element_origin.clone();
                let archive_owner = owner.clone();
                let pin_owner = owner.clone();
                let delete_owner = owner.clone();
                StoriesGridView::new(
                    owner.clone(),
                    permissions,
                    items,
                    has_more,
                    Rc::new(move |cx: &mut App| {
                        let owner = load_owner.clone();
                        let load = move |cx: &mut App| {
                            StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.load_more_profile_stories(Some(&owner), scope, cx);
                            });
                        };
                        match &load_origin {
                            None => load(cx),
                            Some(origin) => SharedElementTransitionCoordinator::run_or_defer_until_settled(
                                SharedElementOriginContainer::surface(&**origin),
                                load,
                                cx,
                            ),
                        }
                    }),
                    Rc::new(move |story: &StoryDTO, cx: &mut App| {
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.open_profile_story(story, &open_owner, scope, open_origin.as_ref(), cx);
                        });
                    }),
                    Rc::new(move |story: &StoryDTO, cx: &mut App| {
                        if let Some(nav_store) = &nav_store {
                            let route = RootRoute::EditStory { story_id: story.id };
                            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                        }
                    }),
                    Rc::new(|story: &StoryDTO, cx: &mut App| {
                        let story = story.clone();
                        ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_story(story, cx));
                    }),
                    Rc::new(move |ids: &[i64], archived: bool, cx: &mut App| {
                        if let [story_id] = ids {
                            let kind = if archived {
                                SingleStoryConfirmationKind::Archive
                            } else {
                                SingleStoryConfirmationKind::Unarchive
                            };
                            StoriesGridPage::alert(
                                SingleStoryConfirmation { kind, story_id: *story_id },
                                &archive_owner,
                                cx,
                            );
                            return;
                        }
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.archive_stories(ids, archived, Some(&archive_owner), None, cx);
                        });
                    }),
                    Rc::new(move |ids: &[i64], pinned: bool, cx: &mut App| {
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.pin_stories(
                                ids,
                                pinned,
                                Some(&pin_owner),
                                Some(Rc::new(|cx: &mut App| StoriesGridPage::present_pin_limit(cx))),
                                None,
                                cx,
                            );
                        });
                    }),
                    Rc::new(move |ids: &[i64], cx: &mut App| {
                        if let [story_id] = ids {
                            StoriesGridPage::alert(
                                SingleStoryConfirmation { kind: SingleStoryConfirmationKind::Delete, story_id: *story_id },
                                &delete_owner,
                                cx,
                            );
                            return;
                        }
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.delete_stories(ids, Some(&delete_owner), None, cx);
                        });
                    }),
                )
                .shared_element_origin(active_shared_element_origin.clone())
                .into_any_element()
            }
        };

        let disappear_owner = owner_user_id.clone();
        StoriesGridStateUi::new(
            self.child_id("state"),
            is_loaded,
            is_empty,
            self.empty_icon.clone(),
            self.empty_text.clone(),
            content,
        )
        .on_disappear(self.child_id("disappear"), move |_, cx| {
            let selection = StoryGridSelection::shared(cx);
            if selection.read(cx).is_active_for(&disappear_owner, scope) {
                selection.update(cx, |selection, cx| selection.end(cx));
            }
        })
    }
}
