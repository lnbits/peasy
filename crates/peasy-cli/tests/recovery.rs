use peasy_core::{IpcRequest, IpcResponse, Proposal, ProposalChange, ServiceStatus};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::{Command, Stdio},
    thread,
};

fn run(recover: bool, answer: &str) -> Vec<IpcRequest> {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("ipc");
    let listener = UnixListener::bind(&socket).unwrap();
    let approved = answer == "yes\n";
    let worker = thread::spawn(move || {
        let count = if !recover {
            1
        } else if approved {
            3
        } else {
            2
        };
        let mut requests = vec![];
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: IpcRequest = serde_json::from_str(&line).unwrap();
            let response = match &request {
                IpcRequest::Inspect => IpcResponse::Inspection {
                    status: Box::new(ServiceStatus {
                        version: "test".into(),
                        protocol: 2,
                        executable: "test".into(),
                        nixpkgs: "test".into(),
                        restart_pending: false,
                        applying: false,
                        recovery: None,
                    }),
                },
                IpcRequest::ProposeRecovery => IpcResponse::Proposal {
                    proposal: Box::new(Proposal {
                        id: "review-token".into(),
                        title: "Restore previous generation".into(),
                        change: ProposalChange::Recovery {
                            generation: "/nix/store/test-system".into(),
                        },
                        diff: vec![],
                        packages: vec![],
                    }),
                },
                IpcRequest::ApplyWithProgress { proposal } => {
                    assert!(approved);
                    assert_eq!(proposal, "review-token");
                    IpcResponse::Applied {
                        result: peasy_core::ApplyResult {
                            activated: true,
                            configuration_valid: true,
                            build_successful: true,
                            message: "Restored test generation".into(),
                        },
                    }
                }
                IpcRequest::Cancel { proposal } => {
                    assert_eq!(proposal, "review-token");
                    IpcResponse::Cancelled {
                        activation_started: false,
                    }
                }
                _ => panic!("unexpected request: {request:?}"),
            };
            requests.push(request);
            writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        }
        requests
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_peasy"))
        .env_clear()
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("missing-config"))
        .args([if recover { "--recover" } else { "--status" }, "--socket"])
        .arg(socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.path().join("missing-config").exists());
    worker.join().unwrap()
}

#[test]
fn status_does_not_need_provider_settings_or_a_wasm_engine() {
    assert_eq!(run(false, ""), [IpcRequest::Inspect]);
}

#[test]
fn recovery_decline_and_eof_release_the_review_without_applying() {
    for answer in ["no\n", ""] {
        assert_eq!(
            run(true, answer),
            [
                IpcRequest::ProposeRecovery,
                IpcRequest::Cancel {
                    proposal: "review-token".into()
                }
            ]
        );
    }
}

#[test]
fn recovery_approval_uses_the_reviewed_token_without_an_ai_provider() {
    assert_eq!(
        run(true, "yes\n"),
        [
            IpcRequest::ProposeRecovery,
            IpcRequest::ApplyWithProgress {
                proposal: "review-token".into()
            },
            IpcRequest::Cancel {
                proposal: "review-token".into()
            }
        ]
    );
}
