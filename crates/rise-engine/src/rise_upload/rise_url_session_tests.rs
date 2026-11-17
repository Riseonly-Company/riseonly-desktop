use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use futures::FutureExt;
use futures::executor::block_on;
use rise_core::foundation::foundation_url::Url;

use super::{URLRequest, URLSession};
use crate::desktop_runtime::rise_account_runtime::{GLOBAL_SLOT, RiseAccountRuntime};

const PATIENCE: Duration = Duration::from_secs(10);

fn request_to(listener: &TcpListener) -> URLRequest {
    let port = listener.local_addr().expect("the listener has an address").port();
    let url = Url::from_string(&format!("http://127.0.0.1:{port}/")).expect("a valid address");
    URLRequest::new(url)
}

#[test]
fn a_request_polled_outside_any_runtime_gets_its_answer() {
    let _slot = GLOBAL_SLOT.blocking_lock();
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let request = request_to(&listener);
    let server = RiseAccountRuntime::current().spawn_blocking(move || {
        let (mut stream, _) = listener.accept().expect("the client connects");
        stream.set_read_timeout(Some(PATIENCE)).expect("a read timeout");
        let mut head = Vec::new();
        let mut piece = [0_u8; 1024];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut piece).expect("the request arrives");
            assert_ne!(read, 0, "the client left before finishing the request");
            head.extend_from_slice(&piece[..read]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .expect("the answer leaves");
    });

    let (data, response) = block_on(URLSession::shared().data_for(request))
        .expect("the transfer moves to a runtime instead of panicking");

    assert_eq!(response.status_code, 200);
    assert_eq!(data, b"ok");
    block_on(server).expect("the server finished");
}

#[test]
fn dropping_a_request_polled_outside_any_runtime_cancels_its_transfer() {
    let _slot = GLOBAL_SLOT.blocking_lock();
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let request = request_to(&listener);
    let (accepted, did_accept) = mpsc::channel();
    let (closed, did_close) = mpsc::channel();
    drop(RiseAccountRuntime::current().spawn_blocking(move || {
        let (mut stream, _) = listener.accept().expect("the client connects");
        stream.set_read_timeout(Some(PATIENCE)).expect("a read timeout");
        let _ = accepted.send(());
        let mut piece = [0_u8; 1024];
        loop {
            match stream.read(&mut piece) {
                Ok(0) => {
                    let _ = closed.send(true);
                    return;
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = closed.send(false);
                    return;
                }
            }
        }
    }));

    let mut exchange = Box::pin(URLSession::shared().data_for(request));
    assert!(exchange.as_mut().now_or_never().is_none());
    did_accept.recv_timeout(PATIENCE).expect("the transfer reached the server");
    drop(exchange);

    assert_eq!(did_close.recv_timeout(PATIENCE * 2), Ok(true));
}
