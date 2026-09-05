use crate::authorization::{Authorizer, Peer};
use crate::nix_backend::NixBackend;
use anyhow::{Context, Result, bail};
use nix::sys::socket::{getsockopt, sockopt};
use peasy_core::{
    IpcRequest, IpcResponse, PackageOperation, PackageState, Proposal, ProposalChange,
    ThemeSettings,
};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CONNECTIONS: usize = 16;
const MAX_CONNECTIONS_PER_UID: usize = 4;
const MAX_REQUESTS_PER_MINUTE: usize = 120;
const MAX_PROPOSALS: usize = 64;
const MAX_PROPOSALS_PER_UID: usize = 8;

#[derive(Default)]
struct Connections {
    active: usize,
    users: HashMap<u32, (usize, usize, Instant)>,
}

struct ConnectionGuard {
    limits: Arc<Mutex<Connections>>,
    uid: u32,
}

impl ConnectionGuard {
    fn acquire(limits: &Arc<Mutex<Connections>>, uid: u32) -> Result<Self> {
        let mut limits_guard = limits.lock().expect("connection mutex poisoned");
        limits_guard.users.retain(|_, (active, _, start)| {
            *active > 0 || start.elapsed() < Duration::from_secs(60)
        });
        if limits_guard.active >= MAX_CONNECTIONS
            || limits_guard.users.len() >= 256 && !limits_guard.users.contains_key(&uid)
        {
            bail!("Peasy is busy; try again shortly");
        }
        let (active, requests, start) =
            limits_guard
                .users
                .entry(uid)
                .or_insert((0, 0, Instant::now()));
        if start.elapsed() >= Duration::from_secs(60) {
            *requests = 0;
            *start = Instant::now();
        }
        if *active >= MAX_CONNECTIONS_PER_UID || *requests >= MAX_REQUESTS_PER_MINUTE {
            bail!("Peasy request limit reached; try again shortly");
        }
        *active += 1;
        *requests += 1;
        limits_guard.active += 1;
        Ok(Self {
            limits: Arc::clone(limits),
            uid,
        })
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut limits = self.limits.lock().expect("connection mutex poisoned");
        limits.active -= 1;
        if let Some((active, _, _)) = limits.users.get_mut(&self.uid) {
            *active -= 1;
        }
    }
}

struct PendingProposal {
    uid: u32,
    change: ProposalChange,
    before: PackageState,
    expires: Instant,
}

pub struct Server {
    socket: PathBuf,
    backend: Arc<NixBackend>,
    proposals: Arc<Mutex<HashMap<String, PendingProposal>>>,
    authorizer: Arc<dyn Authorizer>,
}

impl Server {
    pub fn new(
        socket: PathBuf,
        backend: Arc<NixBackend>,
        authorizer: Arc<dyn Authorizer>,
    ) -> Result<Self> {
        if let Some(parent) = socket.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            socket,
            backend,
            proposals: Arc::new(Mutex::new(HashMap::new())),
            authorizer,
        })
    }

    pub fn run(self) -> Result<()> {
        match fs::symlink_metadata(&self.socket) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(&self.socket)?,
            Ok(_) => bail!(
                "refusing to replace non-socket path {}",
                self.socket.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(&self.socket)
            .with_context(|| format!("binding {}", self.socket.display()))?;
        fs::set_permissions(&self.socket, fs::Permissions::from_mode(0o660))?;
        let connections = Arc::new(Mutex::new(Connections::default()));
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let credentials = match getsockopt(&stream, sockopt::PeerCredentials) {
                        Ok(credentials) => credentials,
                        Err(_) => continue,
                    };
                    let guard = match ConnectionGuard::acquire(&connections, credentials.uid()) {
                        Ok(guard) => guard,
                        Err(_) => {
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                            let _ = stream.write_all(b"{\"response\":\"error\",\"message\":\"Peasy is busy; try again shortly\"}\n");
                            continue;
                        }
                    };
                    let backend = Arc::clone(&self.backend);
                    let proposals = Arc::clone(&self.proposals);
                    let authorizer = Arc::clone(&self.authorizer);
                    // Fallible spawn: hitting the system thread limit must not
                    // panic and restart the service during a running apply.
                    let _ = thread::Builder::new()
                        .name("peasy-ipc".into())
                        .spawn(move || {
                            let _guard = guard;
                            if let Err(error) = handle(stream, backend, proposals, authorizer) {
                                eprintln!("Peasy IPC request failed: {error:#}");
                            }
                        });
                }
                Err(error) => eprintln!("Peasy IPC accept failed: {error}"),
            }
        }
        Ok(())
    }
}

fn handle(
    mut stream: UnixStream,
    backend: Arc<NixBackend>,
    proposals: Arc<Mutex<HashMap<String, PendingProposal>>>,
    authorizer: Arc<dyn Authorizer>,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let credentials = getsockopt(&stream, sockopt::PeerCredentials)?;
    let uid = credentials.uid();
    let peer = Peer::capture(uid, credentials.pid())?;
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .take(64 * 1024)
        .read_line(&mut line)?;
    let response = match serde_json::from_str::<IpcRequest>(&line) {
        Ok(request) => dispatch(request, &peer, &backend, &proposals, authorizer.as_ref())
            .unwrap_or_else(|error| IpcResponse::Error {
                message: format!("{error:#}"),
            }),
        Err(_) => IpcResponse::Error {
            message: "invalid typed IPC request".into(),
        },
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn dispatch(
    request: IpcRequest,
    peer: &Peer,
    backend: &NixBackend,
    proposals: &Mutex<HashMap<String, PendingProposal>>,
    authorizer: &dyn Authorizer,
) -> Result<IpcResponse> {
    let uid = peer.uid;
    match request {
        IpcRequest::SearchPackages { query } => Ok(IpcResponse::SearchResults {
            candidates: backend.search(&query)?,
        }),
        IpcRequest::GetPackages => Ok(IpcResponse::Packages {
            packages: backend.packages()?,
        }),
        IpcRequest::GetTheme => Ok(IpcResponse::Theme {
            theme: backend.theme()?,
        }),
        IpcRequest::GetManagedModule => Ok(IpcResponse::ManagedModule {
            module: backend.managed_module()?,
        }),
        IpcRequest::ProposeInstall { package } => {
            propose_package(backend, proposals, uid, PackageOperation::Install, &package)
        }
        IpcRequest::ProposeAppImageInstall { package } => {
            let preview = backend.preview_appimage_install(package)?;
            store_proposal(proposals, uid, preview)
        }
        IpcRequest::ProposeRemove { package } => {
            if !backend.packages()?.iter().any(|item| item == &package) {
                bail!("Peasy does not manage `{package}`");
            }
            propose_package(backend, proposals, uid, PackageOperation::Remove, &package)
        }
        IpcRequest::ProposeTheme { theme } => propose_theme(backend, proposals, uid, theme),
        IpcRequest::Apply { proposal } => {
            if proposal.len() != 48 || !proposal.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("invalid proposal token");
            }
            let pending = take_proposal(proposals, &proposal, uid)?;
            authorizer.authorize(peer)?;
            if pending.expires < Instant::now() {
                bail!("proposal expired during authorization; review the change again");
            }
            Ok(IpcResponse::Applied {
                result: backend.apply(&pending.change, &pending.before, &proposal)?,
            })
        }
        IpcRequest::Status => Ok(IpcResponse::Status {
            ready: true,
            applying: backend.is_applying(),
        }),
    }
}

fn take_proposal(
    proposals: &Mutex<HashMap<String, PendingProposal>>,
    token: &str,
    uid: u32,
) -> Result<PendingProposal> {
    let mut map = proposals.lock().expect("proposal mutex poisoned");
    let pending = map.get(token).context("unknown or already-used proposal")?;
    if pending.uid != uid || pending.expires < Instant::now() {
        bail!("proposal is expired or belongs to another user");
    }
    Ok(map.remove(token).expect("proposal checked under lock"))
}

fn propose_package(
    backend: &NixBackend,
    proposals: &Mutex<HashMap<String, PendingProposal>>,
    uid: u32,
    operation: PackageOperation,
    package: &str,
) -> Result<IpcResponse> {
    let preview = backend.preview_package(operation, package)?;
    store_proposal(proposals, uid, preview)
}

fn propose_theme(
    backend: &NixBackend,
    proposals: &Mutex<HashMap<String, PendingProposal>>,
    uid: u32,
    theme: ThemeSettings,
) -> Result<IpcResponse> {
    let preview = backend.preview_theme(theme)?;
    store_proposal(proposals, uid, preview)
}

fn store_proposal(
    proposals: &Mutex<HashMap<String, PendingProposal>>,
    uid: u32,
    preview: crate::nix_backend::Preview,
) -> Result<IpcResponse> {
    let id = hex::encode(rand::random::<[u8; 24]>());
    let pending = PendingProposal {
        uid,
        change: preview.change.clone(),
        before: preview.before,
        expires: Instant::now() + Duration::from_secs(300),
    };
    let mut map = proposals.lock().expect("proposal mutex poisoned");
    map.retain(|_, proposal| proposal.expires >= Instant::now());
    if map.len() >= MAX_PROPOSALS
        || map.values().filter(|pending| pending.uid == uid).count() >= MAX_PROPOSALS_PER_UID
    {
        bail!("Too many pending changes; wait for old proposals to expire");
    }
    map.insert(id.clone(), pending);
    Ok(IpcResponse::Proposal {
        proposal: Box::new(Proposal {
            id,
            title: preview.title,
            change: preview.change,
            diff: preview.diff,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nix_backend::{BackendConfig, CommandRunner, RebuildTarget};
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Output;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RefuseCommands(AtomicUsize);
    impl CommandRunner for RefuseCommands {
        fn run(&self, _: &Path, _: &[OsString], _: Option<&Path>) -> Result<Output> {
            self.0.fetch_add(1, Ordering::SeqCst);
            bail!("test build stopped");
        }
    }
    struct Authorization(bool);
    impl Authorizer for Authorization {
        fn authorize(&self, _: &Peer) -> Result<()> {
            if !self.0 {
                bail!("authorization denied");
            }
            Ok(())
        }
    }
    fn preview() -> crate::nix_backend::Preview {
        crate::nix_backend::Preview {
            before: PackageState::default(),
            change: ProposalChange::Theme {
                theme: ThemeSettings {
                    accent_color: Some(peasy_core::AccentColor::Blue),
                    color_scheme: None,
                },
            },
            title: "Blue theme".into(),
            diff: vec![],
        }
    }
    fn proposal(map: &Mutex<HashMap<String, PendingProposal>>, uid: u32) -> Proposal {
        let IpcResponse::Proposal { proposal } = store_proposal(map, uid, preview()).unwrap()
        else {
            panic!()
        };
        *proposal
    }

    #[test]
    fn wrong_user_cannot_consume_token_and_replay_and_expiry_are_rejected() {
        let map = Mutex::new(HashMap::new());
        let p = proposal(&map, 1000);
        assert!(take_proposal(&map, &p.id, 1001).is_err());
        assert!(take_proposal(&map, &p.id, 1000).is_ok());
        assert!(take_proposal(&map, &p.id, 1000).is_err());
        let p = proposal(&map, 1000);
        map.lock().unwrap().get_mut(&p.id).unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(take_proposal(&map, &p.id, 1000).is_err());
    }

    #[test]
    fn direct_ipc_apply_cannot_build_or_write_without_authorization() {
        let temp = tempfile::tempdir().unwrap();
        let runner = Arc::new(RefuseCommands(AtomicUsize::new(0)));
        let backend = NixBackend::new(
            BackendConfig {
                runtime_dir: temp.path().join("run"),
                nix: "/trusted/nix".into(),
                systemctl: "/trusted/systemctl".into(),
                nixpkgs: "/nix/store/test-nixpkgs".into(),
                system: "x86_64-linux".into(),
                managed_module: temp.path().join("source/peasy.nix"),
                appimage_policy: temp.path().join("policy.json"),
                rebuild_target: RebuildTarget::Configuration {
                    path: "/etc/nixos/configuration.nix".into(),
                },
            },
            runner.clone(),
        )
        .unwrap();
        let before = backend.managed_module().unwrap();
        let map = Mutex::new(HashMap::new());
        let p = proposal(&map, 1000);
        let peer = Peer {
            uid: 1000,
            pid: 42,
            start_time: 1,
        };
        let error = dispatch(
            IpcRequest::Apply {
                proposal: p.id.clone(),
            },
            &peer,
            &backend,
            &map,
            &Authorization(false),
        )
        .unwrap_err();
        assert!(error.to_string().contains("authorization denied"));
        assert_eq!(runner.0.load(Ordering::SeqCst), 0);
        assert_eq!(backend.managed_module().unwrap(), before);
        assert!(take_proposal(&map, &p.id, 1000).is_err());
        let p = proposal(&map, 1000);
        assert!(
            dispatch(
                IpcRequest::Apply { proposal: p.id },
                &peer,
                &backend,
                &map,
                &Authorization(true)
            )
            .is_err()
        );
        assert_eq!(runner.0.load(Ordering::SeqCst), 1);
        assert_eq!(backend.managed_module().unwrap(), before);
    }

    #[test]
    fn connections_and_proposals_are_bounded_per_user() {
        let limits = Arc::new(Mutex::new(Connections::default()));
        let guards: Vec<_> = (0..MAX_CONNECTIONS_PER_UID)
            .map(|_| ConnectionGuard::acquire(&limits, 1000).unwrap())
            .collect();
        assert!(ConnectionGuard::acquire(&limits, 1000).is_err());
        assert!(ConnectionGuard::acquire(&limits, 1001).is_ok());
        drop(guards);
        assert!(ConnectionGuard::acquire(&limits, 1000).is_ok());
        let map = Mutex::new(HashMap::new());
        for _ in 0..MAX_PROPOSALS_PER_UID {
            proposal(&map, 1000);
        }
        assert!(store_proposal(&map, 1000, preview()).is_err());
        assert!(store_proposal(&map, 1001, preview()).is_ok());
        for _ in 0..MAX_REQUESTS_PER_MINUTE {
            let _ = ConnectionGuard::acquire(&limits, 1000);
        }
        assert!(ConnectionGuard::acquire(&limits, 1000).is_err());
    }
}
