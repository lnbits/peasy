//! Private, bounded transcripts. Attachments and executable task state are never persisted.
use super::{Answer, Conversation, Turn};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_CHATS: usize = 50;
pub const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub updated: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTurn {
    user: String,
    message: String,
    sources: Vec<(String, String)>,
    suggested_task: Option<String>,
    earlier_omitted: bool,
    attachments: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transcript {
    version: u8,
    trimmed: bool,
    turns: Vec<SavedTurn>,
}
/// Compact snapshot for a bounded background queue, with no attachment contents.
pub struct Snapshot {
    title: String,
    bytes: Vec<u8>,
}
pub struct Loaded {
    pub conversation: Conversation,
    pub incomplete: bool,
}

impl Snapshot {
    pub fn capture(conversation: &Conversation) -> Result<Option<Self>> {
        let Some(first) = conversation.turns.first() else {
            return Ok(None);
        };
        let title = first
            .user
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(80)
            .collect::<String>();
        let mut transcript = Transcript {
            version: 1,
            trimmed: conversation.turns.len() > 32,
            turns: conversation
                .turns
                .iter()
                .rev()
                .take(32)
                .rev()
                .map(|turn| SavedTurn {
                    user: turn.user.clone(),
                    message: turn.answer.message.clone(),
                    sources: turn.answer.sources.iter().take(32).cloned().collect(),
                    suggested_task: turn.answer.suggested_task.clone(),
                    earlier_omitted: turn.answer.earlier_omitted,
                    attachments: turn
                        .attachments
                        .iter()
                        .take(4)
                        .map(|a| a.name.chars().take(160).collect())
                        .collect(),
                })
                .collect(),
        };
        loop {
            let bytes = serde_json::to_vec(&transcript)?;
            if bytes.len() <= MAX_TRANSCRIPT_BYTES {
                return Ok(Some(Self { title, bytes }));
            }
            if transcript.turns.len() <= 1 {
                bail!("Conversation is too large to save");
            }
            transcript.turns.remove(0);
            transcript.trimmed = true;
        }
    }
}

struct HistoryLock(File);
impl Drop for HistoryLock {
    fn drop(&mut self) {
        // Unlock explicitly: concurrent subprocess forks may briefly inherit
        // the open file description, so close alone can retain the lock.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Clone)]
pub struct HistoryStore {
    directory: PathBuf,
}
impl HistoryStore {
    pub fn discover() -> Result<Self> {
        let base = match std::env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
            Some(path) if path.is_absolute() => path,
            _ => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
                .join(".local/state"),
        };
        Ok(Self::at(base.join("peasy/chats")))
    }
    pub fn at(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn new_id() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!(
            "{nanos:032x}{:08x}{:08x}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 48 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid conversation ID");
        }
        Ok(self.directory.join(format!("{id}.chat")))
    }
    fn lock(&self) -> Result<HistoryLock> {
        fs::create_dir_all(&self.directory)?;
        let metadata = fs::symlink_metadata(&self.directory)?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            bail!("Chat history needs a private directory owned by you");
        }
        fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.directory.join(".lock"))?;
        check_file(&lock, u64::MAX)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("Chat history is busy; try again shortly");
        }
        Ok(HistoryLock(lock))
    }
    fn entries(&self) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for (count, item) in fs::read_dir(&self.directory)?.enumerate() {
            if count > 1024 {
                bail!("Unexpectedly many files in chat history");
            }
            let item = item?;
            let name = item.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".chat")) else {
                continue;
            };
            let path = self.path(id)?;
            let file = open_file(&path)?;
            let entry = read_header(&file)?;
            if entry.id != id || entry.title.len() > 512 {
                bail!("Invalid conversation header");
            }
            entries.push(entry);
        }
        entries.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
        Ok(entries)
    }
    pub fn list(&self) -> Result<Vec<Entry>> {
        let _lock = self.lock()?;
        self.prune()
    }
    fn prune(&self) -> Result<Vec<Entry>> {
        let mut entries = self.entries()?;
        for entry in entries.iter().skip(MAX_CHATS) {
            fs::remove_file(self.path(&entry.id)?)?;
        }
        entries.truncate(MAX_CHATS);
        Ok(entries)
    }
    pub fn save(&self, id: &str, snapshot: Snapshot) -> Result<()> {
        let target = self.path(id)?;
        let _lock = self.lock()?;
        // Read/validate the catalogue before replacing anything.
        let entries = self.entries()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let updated = now.max(entries.first().map_or(0, |e| e.updated.saturating_add(1)));
        let entry = Entry {
            id: id.into(),
            title: entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.title.clone())
                .unwrap_or(snapshot.title),
            updated,
        };
        let temporary = self.directory.join(".pending");
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&temporary)?;
            check_file(&file, u64::MAX)?;
            file.set_len(0)?;
            serde_json::to_writer(&mut file, &entry)?;
            file.write_all(b"\n")?;
            file.write_all(&snapshot.bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, target)?;
            self.prune()?;
            File::open(&self.directory)?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(temporary);
        result
    }
    pub fn load(&self, id: &str) -> Result<Loaded> {
        let _lock = self.lock()?;
        let mut file = BufReader::new(open_file(&self.path(id)?)?);
        let mut header = Vec::new();
        (&mut file)
            .take((MAX_HEADER_BYTES + 1) as u64)
            .read_until(b'\n', &mut header)?;
        if header.len() > MAX_HEADER_BYTES || !header.ends_with(b"\n") {
            bail!("Invalid conversation header");
        }
        let entry: Entry = serde_json::from_slice(&header)?;
        if entry.id != id {
            bail!("Conversation ID does not match its file");
        }
        let mut bytes = Vec::new();
        file.take((MAX_TRANSCRIPT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_TRANSCRIPT_BYTES {
            bail!("Saved conversation exceeds its size limit");
        }
        let transcript: Transcript = serde_json::from_slice(&bytes)?;
        if transcript.version != 1 || transcript.turns.len() > 32 {
            bail!("Unsupported conversation format");
        }
        let mut conversation = Conversation::default();
        let mut incomplete = transcript.trimmed;
        for turn in transcript.turns {
            if turn.sources.len() > 32
                || turn.attachments.len() > 4
                || turn.suggested_task.as_ref().is_some_and(|s| s.len() > 8000)
            {
                bail!("Invalid saved conversation");
            }
            let mut user = turn.user;
            if !turn.attachments.is_empty() {
                incomplete = true;
                user.push_str(&format!(
                    "\n\n{}",
                    peasy_core::i18n::tr_args(
                        "Earlier attachments (contents not saved): {files}",
                        &[("files", &turn.attachments.join(", "))]
                    )
                ));
            }
            let sources = turn
                .sources
                .into_iter()
                .filter(|(_, url)| {
                    reqwest::Url::parse(url).is_ok_and(|u| {
                        matches!(u.scheme(), "http" | "https")
                            && u.host_str().is_some()
                            && u.username().is_empty()
                            && u.password().is_none()
                    })
                })
                .collect();
            conversation.push(Turn {
                user,
                attachments: vec![],
                answer: Answer {
                    message: turn.message,
                    sources,
                    suggested_task: turn.suggested_task,
                    earlier_omitted: turn.earlier_omitted,
                },
            });
        }
        Ok(Loaded {
            conversation,
            incomplete,
        })
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        let _lock = self.lock()?;
        match fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn clear(&self) -> Result<()> {
        let _lock = self.lock()?;
        for entry in self.entries()? {
            fs::remove_file(self.path(&entry.id)?)?;
        }
        Ok(())
    }
}
fn check_file(file: &File, limit: u64) -> Result<()> {
    let m = file.metadata()?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != unsafe { libc::geteuid() }
        || m.permissions().mode() & 0o077 != 0
        || m.len() > limit
    {
        bail!("Invalid or oversized private chat history file");
    }
    Ok(())
}
fn open_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    check_file(&file, (MAX_TRANSCRIPT_BYTES + MAX_HEADER_BYTES) as u64)?;
    Ok(file)
}
fn read_header(file: &File) -> Result<Entry> {
    let mut bytes = Vec::new();
    BufReader::new(file.take((MAX_HEADER_BYTES + 1) as u64)).read_until(b'\n', &mut bytes)?;
    if bytes.len() > MAX_HEADER_BYTES || !bytes.ends_with(b"\n") {
        bail!("Invalid conversation header");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::attachments::Attachment;
    fn conversation(text: &str) -> Conversation {
        Conversation {
            turns: vec![Turn {
                user: text.into(),
                attachments: vec![],
                answer: Answer {
                    message: "An answer".into(),
                    sources: vec![("Source".into(), "https://example.com".into())],
                    suggested_task: Some("install firefox".into()),
                    ..Default::default()
                },
            }],
        }
    }
    fn snapshot(text: &str) -> Snapshot {
        Snapshot::capture(&conversation(text)).unwrap().unwrap()
    }
    #[test]
    fn retains_latest_fifty_across_restart_and_updates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("chats"));
        let ids: Vec<_> = (0..55).map(|_| HistoryStore::new_id()).collect();
        for (i, id) in ids.iter().enumerate() {
            store.save(id, snapshot(&format!("Question {i}"))).unwrap();
        }
        let reopened = HistoryStore::at(dir.path().join("chats"));
        let entries = reopened.list().unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[0].id, ids[54]);
        assert!(
            ids.iter()
                .take(5)
                .all(|id| !store.path(id).unwrap().exists())
        );
        reopened.save(&ids[5], snapshot("Follow-up")).unwrap();
        let entries = reopened.list().unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[0].id, ids[5]);
        assert_eq!(
            reopened.load(&ids[5]).unwrap().conversation.turns[0].user,
            "Follow-up"
        );
        assert_eq!(
            fs::metadata(dir.path().join("chats"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path(&ids[5]).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        reopened.delete(&ids[5]).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 49);
        reopened.clear().unwrap();
        assert!(reopened.list().unwrap().is_empty());
    }
    #[test]
    fn attachments_never_reenter_storage_or_model_context() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("chats"));
        let id = HistoryStore::new_id();
        let mut chat = conversation("Explain this file");
        chat.turns[0].attachments.push(
            Attachment::from_bytes("notes.txt".into(), b"SECRET_ATTACHMENT_MARKER".to_vec())
                .unwrap(),
        );
        store
            .save(&id, Snapshot::capture(&chat).unwrap().unwrap())
            .unwrap();
        let bytes = fs::read_to_string(store.path(&id).unwrap()).unwrap();
        assert!(!bytes.contains("SECRET_ATTACHMENT_MARKER"));
        let loaded = store.load(&id).unwrap();
        assert!(loaded.incomplete);
        assert!(loaded.conversation.turns[0].attachments.is_empty());
        assert!(loaded.conversation.turns[0].user.contains("notes.txt"));
        assert_eq!(
            loaded.conversation.turns[0]
                .answer
                .suggested_task
                .as_deref(),
            Some("install firefox")
        );
    }
    #[test]
    fn transcript_and_metadata_reads_have_hard_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("chats"));
        let id = HistoryStore::new_id();
        let mut chat = Conversation::default();
        for i in 0..32 {
            let mut turn = conversation(&format!("Question {i}")).turns.remove(0);
            turn.answer.message = "界".repeat(16000);
            chat.push(turn);
        }
        let snapshot = Snapshot::capture(&chat).unwrap().unwrap();
        assert!(snapshot.bytes.len() <= MAX_TRANSCRIPT_BYTES);
        store.save(&id, snapshot).unwrap();
        let loaded = store.load(&id).unwrap();
        assert!(loaded.incomplete);
        assert!(loaded.conversation.turns.len() < 32);
        assert_eq!(
            loaded.conversation.turns.last().unwrap().user,
            "Question 31"
        );
        let oversized = OpenOptions::new()
            .write(true)
            .open(store.path(&id).unwrap())
            .unwrap();
        oversized
            .set_len((MAX_TRANSCRIPT_BYTES + MAX_HEADER_BYTES + 1) as u64)
            .unwrap();
        assert!(store.load(&id).is_err());
        assert!(store.list().is_err());
    }
    #[test]
    fn rejects_traversal_symlinks_and_competing_writers() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("chats"));
        assert!(store.load("../../outside").is_err());
        let id = HistoryStore::new_id();
        store.save(&id, snapshot("hello")).unwrap();
        let lock = store.lock().unwrap();
        assert!(
            store
                .save(&HistoryStore::new_id(), snapshot("concurrent"))
                .is_err()
        );
        drop(lock);
        let outside = dir.path().join("outside");
        fs::write(&outside, "Do not change").unwrap();
        let link_id = HistoryStore::new_id();
        std::os::unix::fs::symlink(&outside, store.path(&link_id).unwrap()).unwrap();
        assert!(store.load(&link_id).is_err());
        assert!(store.save(&id, snapshot("changed")).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "Do not change");
    }
}
