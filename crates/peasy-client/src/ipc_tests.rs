use super::*;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Instant;

fn accept(listener: &UnixListener) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "client did not connect");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept: {error}"),
        }
    }
}

fn read_request(stream: &UnixStream) -> IpcRequest {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn read_reconnects_after_updating_and_a_truncated_response() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ipc");
    let listener = UnixListener::bind(&path).unwrap();
    let worker = thread::spawn(move || {
        let replies = [
            serde_json::to_string(&IpcResponse::Error {
                message: peasy_core::IPC_RESTARTING_MESSAGE.into(),
            })
            .unwrap()
                + "\n",
            "{\"response\":\"packages\"".into(),
            "{\"response\":\"packages\",\"packages\":[\"patchelf\"]}\n".into(),
        ];
        for reply in replies {
            let mut stream = accept(&listener);
            assert_eq!(read_request(&stream), IpcRequest::GetPackages);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let response = IpcClient::new(path)
        .request(&IpcRequest::GetPackages)
        .unwrap();
    assert_eq!(
        response,
        IpcResponse::Packages {
            packages: vec!["patchelf".into()]
        }
    );
    worker.join().unwrap();
}

#[test]
fn read_waits_for_a_missing_socket_to_return() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ipc");
    let client_path = path.clone();
    let worker = thread::spawn(move || IpcClient::new(client_path).request(&IpcRequest::Status));
    thread::sleep(Duration::from_millis(150));
    let listener = UnixListener::bind(path).unwrap();
    let mut stream = accept(&listener);
    assert_eq!(read_request(&stream), IpcRequest::Status);
    stream
        .write_all(b"{\"response\":\"status\",\"ready\":true,\"applying\":false}\n")
        .unwrap();
    assert!(worker.join().unwrap().is_ok());
}

#[test]
fn lost_mutation_responses_are_never_replayed() {
    for request in [
        IpcRequest::Apply {
            proposal: "reviewed".into(),
        },
        IpcRequest::ApplyWithProgress {
            proposal: "reviewed".into(),
        },
        IpcRequest::ProposeInstall {
            package: "hello".into(),
        },
        IpcRequest::ProposeRecovery,
        IpcRequest::Cancel {
            proposal: "reviewed".into(),
        },
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ipc");
        let listener = UnixListener::bind(&path).unwrap();
        let expected = request.clone();
        let worker = thread::spawn(move || {
            let stream = accept(&listener);
            assert_eq!(read_request(&stream), expected);
            // The operation may have committed, but its response was lost.
            drop(stream);
            drop(listener);
        });
        let error = IpcClient::new(path)
            .request_with_restart_grace(&request, &mut |_| {}, Duration::from_millis(300))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert!(!error.to_string().contains("reconnect"));
        worker.join().unwrap();
    }
}

#[test]
fn application_and_protocol_errors_are_not_retried() {
    for (reply, expected) in [
        (
            "{\"response\":\"error\",\"message\":\"permission denied\"}\n",
            "permission denied",
        ),
        ("not json\n", "invalid system response"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ipc");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&stream);
            stream.write_all(reply.as_bytes()).unwrap();
        });
        let error = IpcClient::new(path)
            .request(&IpcRequest::GetPackages)
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        worker.join().unwrap();
    }
}

#[test]
fn unavailable_daemon_has_a_bounded_retry_window() {
    let temp = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let error = IpcClient::new(temp.path().join("absent"))
        .request_with_restart_grace(
            &IpcRequest::GetPackages,
            &mut |_| {},
            Duration::from_millis(200),
        )
        .unwrap_err();
    assert!(error.to_string().contains("did not reconnect"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn restart_grace_does_not_limit_a_healthy_package_search() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ipc");
    let listener = UnixListener::bind(&path).unwrap();
    let request = IpcRequest::SearchPackages {
        query: "hello".into(),
    };
    let expected = request.clone();
    let worker = thread::spawn(move || {
        let mut stream = accept(&listener);
        assert_eq!(read_request(&stream), expected);
        thread::sleep(Duration::from_millis(150));
        stream
            .write_all(b"{\"response\":\"search_results\",\"candidates\":[]}\n")
            .unwrap();
    });
    let result = IpcClient::new(path)
        .request_with_restart_grace(&request, &mut |_| {}, Duration::from_millis(50))
        .unwrap();
    assert_eq!(result, IpcResponse::SearchResults { candidates: vec![] });
    worker.join().unwrap();
}

#[test]
fn cancellation_interrupts_reconnection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ipc");
    let listener = UnixListener::bind(&path).unwrap();
    let token = Cancellation::default();
    let client_token = token.clone();
    let worker = thread::spawn(move || {
        client_token.scope(|| IpcClient::new(path).request(&IpcRequest::GetPackages))
    });
    let stream = accept(&listener);
    read_request(&stream);
    token.cancel();
    drop(stream);
    assert!(
        worker
            .join()
            .unwrap()
            .unwrap_err()
            .downcast_ref::<peasy_core::cancellation::Cancelled>()
            .is_some()
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
