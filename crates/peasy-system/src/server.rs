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
}

impl Server {
    pub fn new(socket: PathBuf, backend: Arc<NixBackend>) -> Result<Self> {
        if let Some(parent) = socket.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            socket,
            backend,
            proposals: Arc::new(Mutex::new(HashMap::new())),
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
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let backend = Arc::clone(&self.backend);
                    let proposals = Arc::clone(&self.proposals);
                    thread::spawn(move || {
                        if let Err(error) = handle(stream, backend, proposals) {
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
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let credentials = getsockopt(&stream, sockopt::PeerCredentials)?;
    let uid = credentials.uid();
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .take(64 * 1024)
        .read_line(&mut line)?;
    let response = match serde_json::from_str::<IpcRequest>(&line) {
        Ok(request) => dispatch(request, uid, &backend, &proposals).unwrap_or_else(|error| {
            IpcResponse::Error {
                message: format!("{error:#}"),
            }
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
    uid: u32,
    backend: &NixBackend,
    proposals: &Mutex<HashMap<String, PendingProposal>>,
) -> Result<IpcResponse> {
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
            let pending = proposals
                .lock()
                .expect("proposal mutex poisoned")
                .remove(&proposal)
                .context("unknown or already-used proposal")?;
            if pending.uid != uid || pending.expires < Instant::now() {
                bail!("proposal is expired or belongs to another user");
            }
            Ok(IpcResponse::Applied {
                result: backend.apply(&pending.change, &pending.before, &proposal)?,
            })
        }
        IpcRequest::Status => Ok(IpcResponse::Status {
            ready: true,
            applying: false,
        }),
    }
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
    map.insert(id.clone(), pending);
    Ok(IpcResponse::Proposal {
        proposal: Proposal {
            id,
            title: preview.title,
            change: preview.change,
            diff: preview.diff,
        },
    })
}
