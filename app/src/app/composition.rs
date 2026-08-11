use std::path::Path;
use std::sync::Arc;

use gpui::{App, AppContext};
use rise_platform::{KeyringSecureStore, SecureStore};

use crate::core::config::Endpoints;
use crate::core::engine_bridge::EngineBridge;
use crate::modules::auth::engine::rise_auth_domain::RiseAuthDomain;
use crate::modules::auth::engine::rise_auth_repository::AuthCommand;
use crate::modules::auth::stores::auth_actions::AuthActionsStore;
use crate::modules::auth::stores::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::{AuthServicesStore, AuthStores};
use crate::modules::post::engine::rise_post_domain::RisePostDomain;
use crate::modules::post::stores::post_actions::PostActionsStore;
use crate::modules::post::stores::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::{PostServicesStore, PostStores};
use crate::modules::presence::engine::wire::{LivePresenceWire, PresenceWire};
use crate::modules::presence::stores::presence::presence_actions::PresenceActionsStore;
use crate::modules::presence::stores::presence::presence_interactions::PresenceInteractionsStore;
use crate::modules::presence::stores::presence::presence_services::{
    PresenceServicesStore, PresenceStores,
};
use crate::modules::session::engine::rise_session_domain::RiseSessionDomain;
use crate::modules::session::stores::session_actions::SessionActionsStore;
use crate::modules::session::stores::session_interactions::SessionInteractionsStore;
use crate::modules::session::stores::session_services::{SessionServicesStore, SessionStores};
use crate::modules::user::engine::rise_user_domain::RiseUserDomain;
use crate::modules::user::stores::user::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::{UserServicesStore, UserStores};

pub struct Transport {
    _bridge: Arc<EngineBridge>,
}

impl gpui::Global for Transport {}

#[derive(Clone)]
pub struct Interactions {
    pub auth: AuthInteractionsStore,
    pub post: PostInteractionsStore,
    pub user: UserInteractionsStore,
}

impl gpui::Global for Interactions {}

pub fn interactions(cx: &App) -> Option<&Interactions> {
    cx.try_global::<Interactions>()
}

pub fn install(endpoints: &Endpoints, data_directory: &Path, cx: &mut App) {
    let bridge = match EngineBridge::new(endpoints) {
        Ok(bridge) => Arc::new(bridge),
        Err(error) => {
            tracing::error!(
                target: "riseonly",
                "the engine transport could not start ({error}); the app will run offline"
            );
            return;
        }
    };

    let secrets: Arc<dyn SecureStore> = Arc::new(KeyringSecureStore::new());
    if !secrets.is_available() {
        tracing::warn!(
            target: "riseonly",
            "no OS credential store is available; sessions will not survive a restart"
        );
    }

    let auth_domain = Arc::new(RiseAuthDomain::open(
        &bridge,
        data_directory,
        Arc::clone(&secrets),
    ));
    let auth_actions = AuthActionsStore::new(Arc::clone(&auth_domain));
    let auth_services = cx.new(|cx| AuthServicesStore::new(Arc::clone(&auth_domain), cx));

    let presence_wire: Arc<dyn PresenceWire> = Arc::new(LivePresenceWire::new(
        Arc::clone(bridge.wire()),
        bridge.handle(),
    ));
    let presence_actions = PresenceActionsStore::new(presence_wire);
    let presence_services = cx.new(|_| PresenceServicesStore::new(presence_actions.clone()));

    let post_domain = Arc::new(RisePostDomain::open(
        &bridge.handle(),
        Arc::clone(bridge.wire()),
    ));
    let post_actions = PostActionsStore::new(Arc::clone(&post_domain));
    let post_services = cx.new(|cx| PostServicesStore::new(Arc::clone(&post_domain), cx));

    let user_domain = Arc::new(RiseUserDomain::open(
        &bridge.handle(),
        Arc::clone(bridge.wire()),
    ));
    let user_actions = UserActionsStore::new(Arc::clone(&user_domain));
    let user_services = cx.new(|cx| UserServicesStore::new(Arc::clone(&user_domain), cx));

    let session_domain = Arc::new(RiseSessionDomain::open(
        &bridge.handle(),
        Arc::clone(bridge.wire()),
    ));
    let session_actions = SessionActionsStore::new(Arc::clone(&session_domain));
    let session_services = cx.new(|cx| SessionServicesStore::new(Arc::clone(&session_domain), cx));

    // Must be subscribed before the socket authenticates, or the first pushes land unrouted.
    route_pushes(&bridge, presence_services.clone(), cx);
    follow_connection(&bridge, presence_services.clone(), cx);
    follow_account(
        auth_services.clone(),
        presence_services.clone(),
        post_actions.clone(),
        user_actions.clone(),
        cx,
    );

    let post_interactions = PostInteractionsStore::new(post_actions.clone(), post_services.clone());
    let user_interactions = UserInteractionsStore::new(user_actions.clone(), user_services.clone());

    cx.set_global(Interactions {
        auth: AuthInteractionsStore::new(auth_actions.clone(), auth_services.clone()),
        post: post_interactions.clone(),
        user: user_interactions.clone(),
    });
    cx.set_global(UserStores {
        interactions: user_interactions,
        actions: user_actions,
        services: user_services,
    });
    cx.set_global(PostStores {
        interactions: post_interactions,
        actions: post_actions,
        services: post_services,
    });
    cx.set_global(SessionStores {
        interactions: SessionInteractionsStore::new(session_actions, session_services),
    });
    cx.set_global(AuthStores {
        services: auth_services,
        actions: auth_actions,
    });
    cx.set_global(PresenceStores {
        interactions: PresenceInteractionsStore::new(presence_actions, presence_services),
    });
    cx.set_global(Transport {
        _bridge: Arc::clone(&bridge),
    });

    cx.on_app_quit(move |_| {
        bridge.shutdown();
        async {}
    })
    .detach();

    auth_domain.dispatch(AuthCommand::Restore);
}

fn route_pushes(
    bridge: &Arc<EngineBridge>,
    presence: gpui::Entity<PresenceServicesStore>,
    cx: &mut App,
) {
    let mut pushes = bridge.wire().subscribe_pushes();

    cx.spawn(async move |cx| {
        loop {
            match pushes.recv().await {
                Ok(event) => {
                    let handled = presence.update(cx, |store, cx| {
                        store.apply_event(&event.event_type, &event.payload, cx)
                    });

                    if !handled {
                        tracing::debug!(
                            target: "riseonly::events",
                            "no module claims {}",
                            event.event_type
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(target: "riseonly::events", "dropped {missed} push(es)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .detach();
}

// Subscriptions live on the connection: a reconnect silently drops every one of them.
fn follow_connection(
    bridge: &Arc<EngineBridge>,
    presence: gpui::Entity<PresenceServicesStore>,
    cx: &mut App,
) {
    let mut connection = bridge.subscribe_connection();

    cx.spawn(async move |cx| {
        let mut was_connected = false;
        while connection.changed().await.is_ok() {
            let state = *connection.borrow_and_update();
            let is_connected = state.is_connected();

            if is_connected && !was_connected {
                presence.update(cx, |store, _| store.resubscribe_all());
            }
            was_connected = is_connected;
        }
    })
    .detach();
}

fn follow_account(
    auth: gpui::Entity<AuthServicesStore>,
    presence: gpui::Entity<PresenceServicesStore>,
    posts: PostActionsStore,
    users: UserActionsStore,
    cx: &mut App,
) {
    let mut previous: Option<String> = None;

    cx.observe(&auth, move |auth, cx| {
        let active = auth.read(cx).profile().map(|profile| profile.id.clone());

        if active == previous {
            return;
        }

        presence.update(cx, |store, cx| {
            store.reset_account_state(cx);
            store.set_self_id(active.clone(), cx);
        });

        posts.set_identity_action(active.clone());
        users.set_viewer_action(active.clone());

        previous = active;
    })
    .detach();
}
