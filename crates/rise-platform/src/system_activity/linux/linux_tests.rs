use std::sync::Arc;

use super::NativeSystemActivity;
use crate::system_activity::*;

fn ignore_events() -> EventSink {
    Arc::new(|_event| {})
}

#[test]
fn without_a_runtime_on_the_caller_both_halves_start_or_name_the_refusal() {
    let source = NativeSystemActivity;
    for started in [
        source.watch_sleep(ignore_events()),
        source.watch_network_path(ignore_events()),
    ] {
        match started {
            Ok(_watch) => {}
            Err(SystemActivityUnavailable::SystemRefused { detail, .. }) => {
                assert!(!detail.is_empty());
            }
            other => panic!("expected a named refusal, got {other:?}"),
        }
    }
}

#[test]
#[ignore = "needs a system D-Bus with NetworkManager or a session bus with the portal: stage 2"]
fn the_first_network_snapshot_arrives_on_the_engine_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let sink: EventSink = Arc::new(move |event| {
            let _ = sender.send(event);
        });
        let _watch = NativeSystemActivity
            .watch_network_path(sink)
            .expect("network path watch");
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
            .await
            .expect("a snapshot within 5 s");
        assert!(matches!(first, Some(SystemActivityEvent::NetworkPath { .. })));
    });
}

#[test]
fn a_dbus_connection_opens_on_a_thread_without_a_tokio_runtime() {
    let (server_end, client_end) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    let (answer, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outside_tokio = tokio::runtime::Handle::try_current().is_err();
        let opened = futures::executor::block_on(async move {
            let server = zbus::connection::Builder::unix_stream(server_end)
                .server(zbus::Guid::generate())
                .expect("a generated guid")
                .p2p()
                .build();
            let client = zbus::connection::Builder::unix_stream(client_end).p2p().build();
            let (server, client) = futures::join!(server, client);
            server.is_ok() && client.is_ok()
        });
        let _ = answer.send(outside_tokio && opened);
    });
    let opened = answered.recv_timeout(std::time::Duration::from_secs(10));
    assert_eq!(opened, Ok(true));
}
