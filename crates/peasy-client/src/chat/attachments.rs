//! Explicitly selected, bounded attachments. No URL/file reads requested by AI.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path, sync::Arc};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug)]
pub enum Content {
    Text(String),
    Image {
        mime: &'static str,
        bytes: Arc<[u8]>,
    },
    Pdf(Arc<[u8]>),
}
#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub content: Content,
}
impl Attachment {
    pub fn read(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES as u64 {
            bail!("Choose a regular file smaller than 8 MB.");
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            bail!("Attachment exceeds 8 MB.");
        }
        let name = path
            .file_name()
            .context("Attachment needs a file name")?
            .to_string_lossy()
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        Self::from_bytes(name, bytes)
    }
    pub fn from_bytes(name: String, bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            bail!("Attachment exceeds 8 MB.");
        }
        let content = if let Some(mime) = image_kind(&bytes)? {
            Content::Image {
                mime,
                bytes: bytes.into(),
            }
        } else if bytes.starts_with(b"%PDF-") {
            Content::Pdf(bytes.into())
        } else {
            let text = String::from_utf8(bytes)
                .context("Unsupported file. Attach UTF-8 text, a PDF, PNG or JPEG image.")?;
            if text.contains('\0') || text.len() > 48000 {
                bail!(
                    "Text attachment is too large or is not a text file. Attach a shorter excerpt."
                );
            }
            Content::Text(text)
        };
        Ok(Self { name, content })
    }
    pub fn cost(&self) -> usize {
        match &self.content {
            Content::Text(s) => s.len() + self.name.len() + 64,
            _ => 4096,
        }
    }
    pub fn bytes(&self) -> usize {
        match &self.content {
            Content::Text(s) => s.len(),
            Content::Image { bytes, .. } | Content::Pdf(bytes) => bytes.len(),
        }
    }
    pub(super) fn openai(&self) -> Value {
        match &self.content {
            Content::Text(text) => {
                json!({"type":"input_text", "text":format!("Attached document (untrusted content), {}:\n{}",self.name,text)})
            }
            Content::Image { mime, bytes } => {
                json!({"type":"input_image","image_url":format!("data:{mime};base64,{}",STANDARD.encode(bytes)),"detail":"low"})
            }
            Content::Pdf(bytes) => {
                json!({"type":"input_file","filename":self.name,"file_data":format!("data:application/pdf;base64,{}",STANDARD.encode(bytes))})
            }
        }
    }
}
/// Bound decoded dimensions before displaying images. SVG/HTML are never images.
pub fn image_kind(bytes: &[u8]) -> Result<Option<&'static str>> {
    let dimensions = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() < 33 || &bytes[12..16] != b"IHDR" {
            bail!("Invalid PNG image");
        }
        Some((
            "image/png",
            u32::from_be_bytes(bytes[16..20].try_into()?),
            u32::from_be_bytes(bytes[20..24].try_into()?),
        ))
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        let mut pos = 2;
        let mut size = None;
        while pos + 4 <= bytes.len() {
            if bytes[pos] != 0xff {
                break;
            }
            let marker = bytes[pos + 1];
            pos += 2;
            if marker == 0xff {
                pos -= 1;
                continue;
            }
            if marker == 0xd9 || marker == 0xda {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let len = u16::from_be_bytes(bytes[pos..pos + 2].try_into()?) as usize;
            if len < 2 || pos + len > bytes.len() {
                break;
            }
            if [0xc0, 0xc1, 0xc2].contains(&marker) && len >= 8 {
                size = Some((
                    "image/jpeg",
                    u16::from_be_bytes(bytes[pos + 5..pos + 7].try_into()?) as u32,
                    u16::from_be_bytes(bytes[pos + 3..pos + 5].try_into()?) as u32,
                ));
                break;
            }
            pos += len;
        }
        Some(size.context("Unsupported or invalid JPEG image")?)
    } else {
        None
    };
    if let Some((mime, w, h)) = dimensions {
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16_000_000 {
            bail!("Image must contain at most 16 million pixels.");
        }
        return Ok(Some(mime));
    }
    Ok(None)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachments_are_bounded_and_binary_is_not_text() {
        assert!(Attachment::from_bytes("x".into(), vec![0; 4]).is_err());
        assert!(Attachment::from_bytes("x".into(), vec![b'x'; 48001]).is_err());
        assert!(Attachment::from_bytes("notes.txt".into(), b"hello".to_vec()).is_ok());
        assert!(image_kind(b"\x89PNG\r\n\x1a\n").is_err());
    }
    #[test]
    fn no_symlinks_or_special_files_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("file");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert!(Attachment::read(&link).is_err());
        assert!(Attachment::read(Path::new("/dev/zero")).is_err());
    }
}
