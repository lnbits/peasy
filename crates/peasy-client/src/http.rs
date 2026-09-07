//! Abort the actual HTTP future, including body reads, when its UI task closes.
use anyhow::{Context, Result, bail};
use peasy_core::cancellation::{Cancellation, Cancelled};
use std::sync::OnceLock;
use std::time::Duration;

pub(crate) fn read(
    request: reqwest::RequestBuilder,
    limit: usize,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> =
        OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("HTTP runtime unavailable: {e}"))?;
    let cancellation = Cancellation::current();
    cancellation.check()?;
    runtime.block_on(async {
        tokio::select! {
            biased;
            _ = async {
                while !cancellation.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } => Err(Cancelled.into()),
            result = async {
                let mut response = request.send().await.context("sending HTTP request")?;
                let status = response.status();
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.context("reading HTTP response")? {
                    if chunk.len() > limit.saturating_sub(bytes.len()) { bail!("Provider returned an oversized response"); }
                    bytes.extend_from_slice(&chunk);
                }
                Ok((status, bytes))
            } => result,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Instant,
    };

    #[test]
    fn cancellation_aborts_waiting_for_headers_and_body() {
        for headers in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let token = Cancellation::default();
            let worker_token = token.clone();
            let worker = thread::spawn(move || {
                worker_token.scope(|| read(reqwest::Client::new().get(url), 1024))
            });
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            if headers {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                    .unwrap();
            }
            let start = Instant::now();
            token.cancel();
            assert!(
                worker
                    .join()
                    .unwrap()
                    .unwrap_err()
                    .downcast_ref::<Cancelled>()
                    .is_some()
            );
            assert!(start.elapsed() < Duration::from_secs(2));
            assert_eq!(
                stream.read(&mut [0]).unwrap(),
                0,
                "HTTP transport must close, not just discard its result"
            );
        }
    }

    #[test]
    fn cancelled_request_never_connects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let token = Cancellation::default();
        token.cancel();
        assert!(
            token
                .scope(|| read(
                    reqwest::Client::new()
                        .get(format!("http://{}", listener.local_addr().unwrap())),
                    100
                ))
                .is_err()
        );
        assert!(matches!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn body_size_limit_is_preserved() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345")
                .unwrap();
        });
        assert!(
            read(reqwest::Client::new().get(url), 4)
                .unwrap_err()
                .to_string()
                .contains("oversized")
        );
        worker.join().unwrap();
    }
}
