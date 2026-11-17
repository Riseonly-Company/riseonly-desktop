use std::future::Future;
use std::io;
use std::pin::Pin;

use rise_media::rtc_media::signal_stream::{SignalStreamOpener, set_signal_stream_opener};
use tokio::net::TcpStream;

use super::network_transport_config::connect_routed_stream;

fn open_signal_stream(
    host: String,
    port: u16,
    secure: bool,
) -> Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send>> {
    Box::pin(connect_routed_stream(host, port, secure))
}

pub(crate) fn install_rtc_signal_hooks() {
    let opener: SignalStreamOpener = open_signal_stream;
    set_signal_stream_opener(opener);
}
