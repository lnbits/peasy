//! Resource operations enter the existing UID-bound proposal and authorization flow.
use super::{NixBackend, Preview};
use anyhow::{Context, Result, bail};
use nix::unistd::{Uid, User};
use peasy_core::resource_native::{self, ResourceRunner, Tool};
use peasy_core::resources::CallerGroups;
use peasy_core::{
    ApplyResult, DiffKind, DiffLine, PackageState, ProposalChange, ResourceChange, ResourceDomain,
    ResourceQuery, module_diff,
};
use serde_json::{Value, json};
use std::ffi::OsString;

struct Runner<'a>(&'a NixBackend);
impl ResourceRunner for Runner<'_> {
    fn run(&self, tool: Tool, args: &[&str]) -> Result<String> {
        // This adapter is reachable only for system-domain operations. Session
        // tools never run as root through the daemon.
        if !matches!(
            tool,
            Tool::Systemctl | Tool::Nix | Tool::NixEnv | Tool::Df | Tool::Ip | Tool::Lsblk
        ) {
            bail!("session tool is not part of the daemon API");
        }
        if tool == Tool::Lsblk {
            return self
                .0
                .resource_helper(&crate::resource_helper::Request::InspectStorage {});
        }
        if tool == Tool::NixEnv {
            // Even listing generations takes Nix's profile write lock. Keep
            // profile access in the existing confined maintenance helper.
            if args
                != [
                    "--profile",
                    "/nix/var/nix/profiles/system",
                    "--list-generations",
                ]
            {
                bail!("unsupported profile inspection");
            }
            return self
                .0
                .resource_helper(&crate::resource_helper::Request::ListGenerations {});
        }
        let path = match tool {
            Tool::Systemctl => self.0.config.systemctl.clone(),
            Tool::Nix => self.0.config.nix.clone(),
            _ => tool.path()?,
        };
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let result = self.0.runner.run(&path, &args, None)?;
        if !result.status.success() {
            bail!("{} failed ({})", tool.name(), result.status);
        }
        resource_native::bounded_output(&result.stdout)
    }
}
impl NixBackend {
    fn resource_helper(&self, request: &crate::resource_helper::Request) -> Result<String> {
        let _lock = self
            .resource_helper_lock
            .try_lock()
            .map_err(|_| anyhow::anyhow!("resource helper is already running"))?;
        let dir = self.config.runtime_dir.join("resource-helper");
        std::fs::create_dir_all(&dir)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::remove_file(dir.join("result.json"));
        crate::activation::write_private_json(&dir.join("request.json"), request)?;
        let output = self.runner.run(
            &self.config.systemctl,
            &["start".into(), "peasy-resource-helper.service".into()],
            None,
        )?;
        let result: crate::resource_helper::Response = serde_json::from_slice(
            &std::fs::read(dir.join("result.json"))
                .context("resource helper did not return a result")?,
        )?;
        if let Some(error) = result.error {
            bail!("{error}");
        }
        if !output.status.success() {
            bail!("resource helper failed");
        }
        result.output.context("resource helper omitted its output")
    }

    fn resource_config(&self, expression: &str) -> Result<Value> {
        let _lock = self
            .evaluation_lock
            .try_lock()
            .map_err(|_| anyhow::anyhow!("a Nix operation is already running"))?;
        let expression = format!("let host = {}; in {expression}", self.host_expression()?);
        let output = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--impure".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                expression.into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!("could not inspect host resource configuration");
        }
        serde_json::from_str(&resource_native::bounded_output(&output.stdout)?)
            .context("invalid host resource configuration")
    }
    pub fn inspect_resources(&self, query: &ResourceQuery) -> Result<String> {
        query.validate()?;
        if query.domain.session() {
            bail!("inspect this resource in the desktop session");
        }
        let mut data = resource_native::inspect(query, &Runner(self))?;
        if query.domain == ResourceDomain::Firewall {
            data["nixos_policy"]=self.resource_config("{ inherit (host.config.networking.firewall) enable allowedTCPPorts allowedUDPPorts trustedInterfaces; interfaces = builtins.mapAttrs (_: v: { inherit (v) allowedTCPPorts allowedUDPPorts; }) host.config.networking.firewall.interfaces; }")?;
            data["limits"] = json!(
                "Declared NixOS policy; custom rules, containers and listening sockets can differ. This is not an external reachability test."
            );
        }
        data["peasy_managed"] = serde_json::to_value(self.current_state()?.resources)?;
        let text = serde_json::to_string(&data)?;
        if text.len() > 56 * 1024 {
            bail!("resource inspection is too large");
        }
        Ok(text)
    }
    pub fn preview_resources(&self, plan: ResourceChange, uid: u32) -> Result<Preview> {
        plan.validate()?;
        if !plan.privileged() {
            bail!("this is a session operation");
        }
        let before = self.current_state()?;
        let caller = if matches!(
            plan,
            ResourceChange::UserGroups { .. } | ResourceChange::UserDisabled { .. }
        ) {
            let user =
                User::from_uid(Uid::from_raw(uid))?.context("requesting user disappeared")?;
            if uid < 1000 || uid == 65534 {
                bail!("normal user account required");
            }
            Some(CallerGroups {
                name: user.name,
                uid,
                groups: vec![],
            })
        } else if let ResourceChange::UserCreate { name } = &plan {
            let accounts =
                self.resource_config("builtins.mapAttrs (_: u: u.uid) host.config.users.users")?;
            let next = (1000..60000)
                .find(|id| {
                    User::from_uid(Uid::from_raw(*id)).ok().flatten().is_none()
                        && !accounts
                            .as_object()
                            .is_some_and(|a| a.values().any(|v| v.as_u64() == Some(*id as u64)))
                })
                .context("no free normal user ID")?;
            Some(CallerGroups {
                name: name.clone(),
                uid: next,
                groups: vec![],
            })
        } else {
            None
        };
        self.validate_resource_ownership(&before, &plan, caller.as_ref())?;
        let snapshot = serde_json::to_string(&resource_native::snapshot(&plan, &Runner(self))?)?;
        let mut diff = if plan.persistent() {
            let after = self.resource_state(&before, &plan, caller.as_ref())?;
            if before == after {
                bail!("resources are already in the requested state");
            }
            module_diff(&before, &after)?
        } else {
            vec![
                DiffLine {
                    kind: DiffKind::Add,
                    text: plan.summary(),
                },
                DiffLine {
                    kind: DiffKind::Context,
                    text: resource_native::review_identity(
                        &plan,
                        &serde_json::from_str(&snapshot)?,
                    ),
                },
            ]
        };
        diff.push(DiffLine {
            kind: DiffKind::Context,
            text: plan.note().into(),
        });
        Ok(Preview {
            packages: vec![],
            before,
            title: plan.summary(),
            diff,
            change: ProposalChange::Resources {
                plan,
                caller,
                snapshot,
            },
        })
    }
    fn validate_resource_ownership(
        &self,
        state: &PackageState,
        plan: &ResourceChange,
        caller: Option<&CallerGroups>,
    ) -> Result<()> {
        match plan {
            ResourceChange::ServiceEnabled {
                service,
                enabled: false,
            } if !state.resources.services.contains(service) => {
                bail!("Peasy does not own this service enablement")
            }
            ResourceChange::PersistentMount { name, present, .. } => {
                let path = format!("/mnt/peasy-{name}");
                if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                    bail!("mount path is a symlink");
                }
                let existing = self.resource_config(&format!(
                    "builtins.hasAttr {} host.config.fileSystems",
                    peasy_core::nix_string(&path)
                ))?;
                let owned = state.resources.mounts.iter().any(|m| &m.name == name);
                if existing == true && !owned {
                    bail!("mount belongs to administrator configuration");
                }
                if !*present && !owned {
                    bail!("Peasy does not own this mount");
                }
            }
            ResourceChange::UserCreate { name } => {
                let binding = caller.context("missing assigned account identity")?;
                if User::from_name(name)?.is_some()
                    || User::from_uid(Uid::from_raw(binding.uid))?.is_some()
                {
                    bail!("account name or assigned UID is already used");
                }
                if self.resource_config(&format!(
                    "builtins.hasAttr {} host.config.users.users",
                    peasy_core::nix_string(name)
                ))? == true
                {
                    bail!("account is already declared by the administrator");
                }
            }
            ResourceChange::UserDisabled { name, .. } => {
                let account = state
                    .resources
                    .users
                    .iter()
                    .find(|u| &u.name == name)
                    .context("Peasy does not own this account")?;
                let user = User::from_name(name)?.context("account disappeared")?;
                if user.uid.as_raw() != account.uid || caller.is_some_and(|c| c.uid == account.uid)
                {
                    bail!("account identity changed or this is the requesting account");
                }
            }
            ResourceChange::UserGroups { groups } => {
                let caller = caller.context("missing caller identity")?;
                if User::from_name(&caller.name)?.is_none_or(|u| u.uid.as_raw() != caller.uid) {
                    bail!("caller account changed");
                }
                for name in groups {
                    if nix::unistd::Group::from_name(name)?.is_none() {
                        bail!("group {name} does not exist; enable its service first");
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn resource_state(
        &self,
        before: &PackageState,
        plan: &ResourceChange,
        caller: Option<&CallerGroups>,
    ) -> Result<PackageState> {
        self.validate_resource_ownership(before, plan, caller)?;
        let mut after = before.clone();
        after.resources = before
            .resources
            .changed(plan, caller.map(|c| (c.name.as_str(), c.uid)))?;
        after.normalize()?;
        Ok(after)
    }
    pub(super) fn check_resource_snapshot(
        &self,
        plan: &ResourceChange,
        snapshot: &str,
    ) -> Result<()> {
        let expected: Value = serde_json::from_str(snapshot)?;
        if resource_native::snapshot(plan, &Runner(self))? != expected {
            bail!("resource identity changed; review again");
        }
        Ok(())
    }
    pub(super) fn apply_live_resource(
        &self,
        plan: &ResourceChange,
        snapshot: &str,
    ) -> Result<ApplyResult> {
        if !plan.privileged() || plan.persistent() {
            bail!("invalid privileged live resource operation");
        }
        if let ResourceChange::NixDeleteGenerations { generations } = plan {
            self.check_resource_snapshot(plan, snapshot)?;
            peasy_core::cancellation::Cancellation::current().protect()?;
            self.resource_helper(&crate::resource_helper::Request::DeleteGenerations {
                generations: generations.clone(),
                snapshot: serde_json::from_str(snapshot)?,
            })?;
        } else {
            resource_native::apply_live(plan, &serde_json::from_str(snapshot)?, &Runner(self))?;
        }
        Ok(ApplyResult {
            configuration_valid: true,
            build_successful: true,
            activated: true,
            message: format!("Resource operation completed. {}", plan.note()),
        })
    }
}
