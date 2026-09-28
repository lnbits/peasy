//! Narrow, short-lived helper. The main daemon keeps its existing sandbox.
use anyhow::{Context, Result, bail};
use peasy_core::{
    ResourceChange,
    resource_native::{self, ResourceRunner, SessionRunner, Tool},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    InspectStorage {},
    ListGenerations {},
    DeleteGenerations {
        generations: Vec<u64>,
        snapshot: Value,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub output: Option<String>,
    pub error: Option<String>,
}
pub fn run(runtime: &Path) -> Result<()> {
    let directory = runtime.join("resource-helper");
    let path = directory.join("request.json");
    let result_path = directory.join("result.json");
    let _ = fs::remove_file(&result_path);
    let result = (|| -> Result<String> {
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.permissions().mode() & 0o077 != 0
            || meta.len() > 48 * 1024
        {
            bail!("invalid private resource request");
        }
        let request: Request = serde_json::from_slice(&fs::read(&path)?)?;
        fs::remove_file(&path)?;
        match request {
            Request::ListGenerations {} => SessionRunner.run(
                Tool::NixEnv,
                &[
                    "--profile",
                    "/nix/var/nix/profiles/system",
                    "--list-generations",
                ],
            ),
            Request::InspectStorage {} => SessionRunner.run(
                Tool::Lsblk,
                &[
                    "--json",
                    "--bytes",
                    "--paths",
                    "--output",
                    "NAME,TYPE,RM,RO,SIZE,FSTYPE,UUID,MOUNTPOINTS,TRAN,MAJ:MIN,SERIAL",
                ],
            ),
            Request::DeleteGenerations {
                generations,
                snapshot,
            } => {
                resource_native::apply_live(
                    &ResourceChange::NixDeleteGenerations { generations },
                    &snapshot,
                    &SessionRunner,
                )?;
                Ok("Generation references removed".into())
            }
        }
    })();
    let response = match &result {
        Ok(output) => Response {
            output: Some(output.clone()),
            error: None,
        },
        Err(error) => Response {
            output: None,
            error: Some(format!("{error:#}")),
        },
    };
    crate::activation::write_private_json(&result_path, &response)?;
    result.map(|_| ()).context("resource helper failed")
}
