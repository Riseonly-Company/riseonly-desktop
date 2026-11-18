use std::collections::HashMap;

use futures::channel::oneshot;
use futures::future::{select, Either};
use futures::StreamExt as _;
use zbus::message::Type as MessageType;
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, MessageStream};

use super::system_clock::*;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, system_bus_socket};
use crate::system_activity::ActivityWatch;

pub(super) struct NativeSystemClock;

impl SystemClockSource for NativeSystemClock {
    fn watch(&self, sink: SystemClockSink) -> Result<ActivityWatch, SystemClockUnavailable> {
        let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").ok();
        if let Some(detail) = missing_bus_detail("system", &system_bus_socket(address.as_deref())) {
            return Err(SystemClockUnavailable::NoService(detail));
        }
        let (stop, stopped) = oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("rise-system-clock".into())
            .spawn(move || {
                let ended = crate::dbus_block_on::block_on(async move {
                    let follow = std::pin::pin!(follow_time_zone(&sink));
                    match select(follow, stopped).await {
                        Either::Left((ended, _)) => ended,
                        Either::Right(_) => Ok(()),
                    }
                });
                if let Err(error) = ended {
                    tracing::warn!(target: "riseonly", %error, "time zone changes are no longer heard; activation still re-reads");
                }
            })
            .map_err(|error| {
                SystemClockUnavailable::NoService(format!("no thread for the time zone watch: {error}"))
            })?;
        Ok(ActivityWatch::new(StopOnDrop(Some(stop))))
    }
}

struct StopOnDrop(Option<oneshot::Sender<()>>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            let _ = stop.send(());
        }
    }
}

async fn follow_time_zone(sink: &SystemClockSink) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(DBUS_PROPERTIES_INTERFACE)?
        .member(DBUS_PROPERTIES_CHANGED_SIGNAL)?
        .path(TIMEDATE1_OBJECT_PATH)?
        .arg(0, TIMEDATE1_INTERFACE)?
        .build();
    let mut changes = MessageStream::for_match_rule(rule, &connection, None).await?;
    while let Some(message) = changes.next().await {
        if message_touches_zone(&message?) {
            sink(SystemClockChange::TimeZone);
        }
    }
    Err(zbus::Error::Failure(
        "the system bus closed the timedate1 stream".to_owned(),
    ))
}

fn message_touches_zone(message: &zbus::Message) -> bool {
    let body = message.body();
    match body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>() {
        Ok((_interface, changed, invalidated)) => {
            timedate_change_touches_zone(changed.keys().map(String::as_str), &invalidated)
        }
        Err(error) => {
            tracing::debug!(target: "riseonly", %error, "system clock: malformed PropertiesChanged");
            false
        }
    }
}
