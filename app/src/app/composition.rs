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

/// Where the transport lives for the life of the process.
///
/// A global rather than a field on the shell: the socket is not a window's, and
/// a second window must not open a second connection. Nothing reads it — holding
/// it IS the point, because dropping the bridge would take the runtime, the
/// socket and every in-flight request with it.
pub struct Transport {
    _bridge: Arc<EngineBridge>,
}

impl gpui::Global for Transport {}

/// The interaction stores a screen reaches for.
///
/// One place a new module joins the composition. A module appears here when a
/// screen calls it; until then its stores are held alive by their own globals,
/// which is ownership rather than API.
#[derive(Clone)]
pub struct Interactions {
    pub auth: AuthInteractionsStore,
}

impl gpui::Global for Interactions {}

pub fn interactions(cx: &App) -> Option<&Interactions> {
    cx.try_global::<Interactions>()
}

/// Opens the transport and every module that exists so far.
///
/// Called once, before the first window. The order matters in one place only:
/// the presence router has to be subscribed before the socket is told to
/// authenticate, or the first pushes after a sign-in land with nobody listening.
pub fn install(endpoints: &Endpoints, data_directory: &Path, cx: &mut App) {
    let bridge = match EngineBridge::new(endpoints) {
        Ok(bridge) => Arc::new(bridge),
        Err(error) => {
            // Not fatal, and deliberately so: the app still draws, every request
            // fails with NotConnected, and the user sees the error state a screen
            // already has rather than a process that will not start.
            tracing::error!(
                target: "riseonly",
                "the engine transport could not start ({error}); the app will run offline"
            );
            return;
        }
    };

    let secrets: Arc<dyn SecureStore> = Arc::new(KeyringSecureStore::new());
    if !secrets.is_available() {
        // Stated rather than hidden. On Linux without a running secret service
        // this is the difference between "signed in" and "signed in until you
        // quit", and the user is about to find out either way.
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

    let session_domain = Arc::new(RiseSessionDomain::open(
        &bridge.handle(),
        Arc::clone(bridge.wire()),
    ));
    let session_actions = SessionActionsStore::new(Arc::clone(&session_domain));
    let session_services = cx.new(|cx| SessionServicesStore::new(Arc::clone(&session_domain), cx));

    route_pushes(&bridge, presence_services.clone(), cx);
    follow_connection(&bridge, presence_services.clone(), cx);
    follow_account(auth_services.clone(), presence_services.clone(), cx);

    cx.set_global(Interactions {
        auth: AuthInteractionsStore::new(auth_actions.clone(), auth_services.clone()),
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

    // A clean close rather than a dropped socket: the gateway then takes the
    // user offline immediately instead of waiting out its sixty-second silence
    // reaper, which is the difference between a contact going offline when the
    // app quits and a minute later.
    cx.on_app_quit(move |_| {
        bridge.shutdown();
        async {}
    })
    .detach();

    // The cold start: read the account registry, refresh if the access token has
    // aged out, and point the socket at whoever it finds.
    auth_domain.dispatch(AuthCommand::Restore);
}

/// The one place an inbound push becomes a module's business.
///
/// The gateway has no subscribe protocol — it fans out by session and the client
/// dispatches by event-type string — so this is the router the reference builds
/// out of `SaiWsEventHandlerRegistration`. Each module answers whether the type
/// was its own, which is what makes an unrouted event visible instead of silent.
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
                // The bus is bounded, so a burst can outrun this task. Losing a
                // presence push is recoverable — the next one corrects it — but
                // it must be visible rather than silent, because for a module
                // with durable state it would not be.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(target: "riseonly::events", "dropped {missed} push(es)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .detach();
}

/// Re-subscribes presence when the socket comes back.
///
/// Subscriptions live on the CONNECTION. A reconnect silently drops every one of
/// them, and without this presence freezes at whatever it last showed — which
/// looks exactly like everybody going quiet.
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

/// Keeps presence pointed at the right account.
///
/// Two things change together on a switch: who "me" is, so nothing subscribes to
/// its own presence, and everything the previous account was watching, which
/// must not survive into the next one.
fn follow_account(
    auth: gpui::Entity<AuthServicesStore>,
    presence: gpui::Entity<PresenceServicesStore>,
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
        previous = active;
    })
    .detach();
}
