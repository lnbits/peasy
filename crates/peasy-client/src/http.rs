//! Abort the actual HTTP future, including body reads, when its UI task closes.
use anyhow::{Context, Result, bail};
use peasy_core::cancellation::{Cancellation, Cancelled};
use std::sync::OnceLock;
use std::time::Duration;

/// Stream into a private temporary file. Neither untrusted release metadata nor
/// Content-Length is sufficient: enforce the byte budget on every body chunk.
pub(crate) fn download(
    request: reqwest::RequestBuilder,
    expected_size: u64,
    maximum: u64,
) -> Result<(tempfile::NamedTempFile, String)> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    if expected_size == 0 || expected_size > maximum {
        bail!("download size is outside the allowed range");
    }
    let cancellation = Cancellation::current();
    cancellation.check()?;
    runtime()?.block_on(async {
        tokio::select! {
            biased;
            _ = async {
                while !cancellation.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } => Err(Cancelled.into()),
            result = async {
                let mut response = request.send().await.context("downloading release asset")?
                    .error_for_status().context("release download failed")?;
                if response.content_length().is_some_and(|size| size != expected_size) {
                    bail!("release download size differs from the reviewed metadata");
                }
                let mut file = tempfile::NamedTempFile::new().context("creating private download file")?;
                let mut hash = Sha256::new();
                let mut size = 0_u64;
                while let Some(chunk) = response.chunk().await.context("reading release asset")? {
                    cancellation.check()?;
                    if chunk.len() as u64 > expected_size.saturating_sub(size) {
                        bail!("release download exceeds the reviewed size limit");
                    }
                    file.write_all(&chunk).context("saving release asset")?;
                    hash.update(&chunk);
                    size += chunk.len() as u64;
                }
                if size != expected_size {
                    bail!("release download is shorter than the reviewed size");
                }
                file.flush()?;
                let hash = base64::engine::general_purpose::STANDARD.encode(hash.finalize());
                Ok((file, format!("sha256-{hash}")))
            } => result,
        }
    })
}

pub(crate) fn read(
    request: reqwest::RequestBuilder,
    limit: usize,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let runtime = runtime()?;
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

fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> =
        OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("HTTP runtime unavailable: {e}"))
}

/// Consume bounded NDJSON records without retaining a download's progress log.
/// The callback must explicitly recognise the terminal success record.
pub(crate) fn read_lines(
    request: reqwest::RequestBuilder,
    mut record: impl FnMut(&[u8]) -> Result<bool>,
) -> Result<()> {
    let cancellation = Cancellation::current();
    cancellation.check()?;
    runtime()?.block_on(async {
        tokio::select! {
            biased;
            _ = async {
                while !cancellation.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } => Err(Cancelled.into()),
            result = async {
                let mut response = request.send().await.context("contacting local Ollama")?;
                let status = response.status();
                let mut pending = Vec::new();
                while let Some(chunk) = response.chunk().await.context("reading Ollama download progress")? {
                    for byte in chunk {
                        if pending.len() >= 64 * 1024 { bail!("Ollama progress record is too large"); }
                        if byte == b'\n' {
                            if !pending.is_empty() {
                                if !status.is_success() { return Err(pull_http_error(status, &pending)); }
                                if record(&pending)? { return Ok(()); }
                                pending.clear();
                            }
                        } else { pending.push(byte); }
                    }
                }
                if !status.is_success() { return Err(pull_http_error(status, &pending)); }
                if !pending.is_empty() && record(&pending)? { return Ok(()); }
                bail!("Ollama download ended before confirming success")
            } => result,
        }
    })
}

fn pull_http_error(status: reqwest::StatusCode, bytes: &[u8]) -> anyhow::Error {
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    anyhow::anyhow!(
        "Ollama HTTP {status}: {}",
        super::safe_provider_error(value["error"].as_str().unwrap_or("model download failed"))
    )
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

    fn download_response(
        response: &'static [u8],
        expected: u64,
    ) -> Result<(tempfile::NamedTempFile, String)> {
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
            stream.write_all(response).unwrap();
        });
        let result = download(reqwest::Client::new().get(url), expected, 10);
        worker.join().unwrap();
        result
    }

    #[test]
    fn download_caps_actual_bytes_even_without_content_length() {
        for response in [
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n12345".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n12345\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345",
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n123",
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\n1234",
        ] {
            assert!(download_response(response, 4).is_err());
        }
        let (file, hash) =
            download_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc", 3).unwrap();
        assert_eq!(std::fs::read(file.path()).unwrap(), b"abc");
        assert_eq!(hash, "sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
        let path = file.path().to_owned();
        drop(file);
        assert!(!path.exists());
    }

    #[test]
    fn cancellation_aborts_waiting_for_headers_and_body() {
        for (headers, to_file) in [(false, false), (true, false), (false, true), (true, true)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let token = Cancellation::default();
            let worker_token = token.clone();
            let worker = thread::spawn(move || {
                worker_token.scope(|| {
                    let request = reqwest::Client::new().get(url);
                    if to_file {
                        download(request, 100, 1024).map(|_| ())
                    } else {
                        read(request, 1024).map(|_| ())
                    }
                })
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
