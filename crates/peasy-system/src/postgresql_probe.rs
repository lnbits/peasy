//! Fixed, read-only inventory. The main daemon never gains database access.
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

pub const REPORT: &str = "postgresql/retained.json";

pub fn scan(root: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).context("inspecting retained PostgreSQL directories"),
    };
    let mut versions = vec![];
    for (count, entry) in entries.enumerate() {
        if count >= 128 {
            bail!("PostgreSQL directory inventory exceeds its limit");
        }
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if name == "PG_VERSION" || (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())) {
            if name.len() > 32 {
                bail!("invalid PostgreSQL version directory");
            }
            versions.push(name.into_owned());
        }
    }
    versions.sort();
    Ok(versions)
}

pub fn run(runtime: &Path) -> Result<()> {
    let report = runtime.join(REPORT);
    if report.exists() {
        fs::remove_file(&report)?;
    }
    let versions = scan(Path::new("/var/lib/postgresql"))?;
    crate::activation::write_private_json(&report, &versions)
}

pub fn read(runtime: &Path) -> Result<Vec<String>> {
    use std::io::Read;
    let mut bytes = vec![];
    fs::File::open(runtime.join(REPORT))?
        .take(8193)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        bail!("PostgreSQL directory inventory exceeds its limit");
    }
    serde_json::from_slice(&bytes).context("reading PostgreSQL directory inventory")
}
