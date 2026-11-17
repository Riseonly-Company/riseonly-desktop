use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Subscription};
use rise_core::foundation::foundation_date::Date;
use rise_engine::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription,
};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SAI_WS_DID_CONNECT, SAI_WS_DID_DISCONNECT, SaiWebSocketManager,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::sai_auth_hooks::sai_auth_access_token;

/// read only view of the engine transport for stores that must tell offline from still loading
/// stores never own the socket, they only ask and nudge it
pub enum RiseTransportStatus {}

impl RiseTransportStatus {
    pub fn is_connected(cx: &mut App) -> bool {
        SaiWs::manager(cx).read(cx).is_connected()
    }

    pub fn is_connecting(cx: &mut App) -> bool {
        SaiWs::manager(cx).read(cx).is_connecting()
    }

    /// true when the device has no network path at all, waiting for the socket cant help
    pub fn is_network_path_down(cx: &mut App) -> bool {
        SaiWs::manager(cx).read(cx).is_network_path_down()
    }

    pub fn is_offline(cx: &mut App) -> bool {
        !Self::is_connected(cx) && !Self::is_connecting(cx)
    }

    /// the chats header waiting for network state, a retry after a lost link still counts
    pub fn is_waiting_for_network(cx: &mut App) -> bool {
        let entity = SaiWs::manager(cx);
        let manager = entity.read(cx);
        !manager.is_connected()
            && (!manager.is_connecting() || manager.is_reconnecting_after_failure())
    }

    pub fn wake_if_offline(cx: &mut App) {
        if !Self::is_offline(cx) {
            return;
        }
        let access_token = sai_auth_access_token(cx);
        SaiWs::connect_if_needed(access_token.as_deref(), cx);
    }

    pub fn begin_reconnect_demand(cx: &mut App) -> RiseRequestID {
        let demand = RiseRequestIDAllocator::shared().next(Date::now());
        SaiWs::manager(cx).update(cx, |manager, cx| manager.note_durable_outbound_waiting(demand, cx));
        Self::wake_if_offline(cx);
        demand
    }

    pub fn end_reconnect_demand(demand: RiseRequestID, cx: &mut App) {
        SaiWs::manager(cx).update(cx, |manager, _| manager.note_durable_outbound_settled(demand));
    }

    /// what a view shows about the transport, for reads inside render
    /// reading the socket manager there would redraw on each notify
    pub fn shown(cx: &mut App) -> RiseTransportShown {
        RiseTransportShownStore::shared(cx).read(cx).shown
    }

    /// store behind shown for observers that redraw a view, it notifies only on a change
    pub fn shown_store(cx: &mut App) -> Entity<RiseTransportShownStore> {
        RiseTransportShownStore::shared(cx)
    }

    /// calls handler once on the next transport drop, pass the token to stop to cancel
    pub fn on_next_disconnect(
        handler: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> RiseTransportDropWatch {
        Self::on_next(SAI_WS_DID_DISCONNECT, handler, cx)
    }

    /// calls handler once on the next transport connect, pass the token to stop to cancel
    pub fn on_next_connect(
        handler: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> RiseTransportDropWatch {
        Self::on_next(SAI_WS_DID_CONNECT, handler, cx)
    }

    fn on_next(
        name: RiseNotificationName,
        handler: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> RiseTransportDropWatch {
        let (fired, mut on_fired) = mpsc::unbounded::<()>();
        let token = RiseAppEvents::default_center().observe(name, move |_| {
            let _ = fired.unbounded_send(());
        });
        let watch = RiseTransportDropWatch { token: Rc::new(RefCell::new(Some(token))) };
        let weak_token = Rc::downgrade(&watch.token);
        cx.spawn(async move |cx| {
            if on_fired.next().await.is_none() {
                return;
            }
            let Some(token) = weak_token.upgrade() else {
                return;
            };
            let stopped = token.borrow_mut().take();
            drop(token);
            if stopped.is_none() {
                return;
            }
            drop(stopped);
            cx.update(handler);
        })
        .detach();
        watch
    }
}

/// observer token of one on_next call, dropping it stops the observer
pub struct RiseTransportDropWatch {
    token: Rc<RefCell<Option<RiseSubscription>>>,
}

impl RiseTransportDropWatch {
    pub fn stop(&self) {
        self.token.borrow_mut().take();
    }
}

/// transport as a screen states it, online or first connect or waiting for network
/// a retry after a lost link still waits
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseTransportLink {
    Connected,
    Connecting,
    WaitingForNetwork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseTransportShown {
    pub link: RiseTransportLink,
    pub is_network_path_down: bool,
}

impl RiseTransportShown {
    fn of(manager: &SaiWebSocketManager) -> Self {
        let link = if manager.is_connected() {
            RiseTransportLink::Connected
        } else if manager.is_connecting() && !manager.is_reconnecting_after_failure() {
            RiseTransportLink::Connecting
        } else {
            RiseTransportLink::WaitingForNetwork
        };
        Self { link, is_network_path_down: manager.is_network_path_down() }
    }

    pub fn is_connected(&self) -> bool {
        self.link == RiseTransportLink::Connected
    }

    pub fn is_waiting_for_network(&self) -> bool {
        self.link == RiseTransportLink::WaitingForNetwork
    }
}

/// holds the shown transport and notifies only when it changes
pub struct RiseTransportShownStore {
    shown: RiseTransportShown,
    _observation: Subscription,
}

impl RiseTransportShownStore {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let manager = SaiWebSocketManager::shared(cx);
        let shown = RiseTransportShown::of(manager.read(cx));
        let observation = cx.observe(&manager, |store, manager, cx| {
            let next = RiseTransportShown::of(manager.read(cx));
            if next != store.shown {
                store.shown = next;
                cx.notify();
            }
        });
        Self { shown, _observation: observation }
    }
}
