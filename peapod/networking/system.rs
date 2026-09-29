use super::{NixBackend, Preview};
use anyhow::{Result, bail};
use peasy_core::{DiffKind, DiffLine, NetworkPlan, ProposalChange, module_diff};
impl NixBackend {
    pub(super) fn validate_network_ownership(
        &self,
        before: &peasy_core::PackageState,
        plan: &NetworkPlan,
    ) -> Result<()> {
        for p in &plan.profiles {
            if !before.networks.iter().any(|owned| owned.id == p.id)
                && self
                    .config
                    .network_profiles_dir
                    .as_path()
                    .join(format!("{}.nmconnection", p.connection_id()))
                    .symlink_metadata()
                    .is_ok()
            {
                bail!("Peasy cannot replace an existing administrator-owned connection file");
            }
        }
        Ok(())
    }
    pub fn preview_network(&self, plan: NetworkPlan) -> Result<Preview> {
        let before = self.current_state()?;
        self.validate_network_ownership(&before, &plan)?;
        let after = before.with_network(&plan)?;
        if before == after {
            bail!("network configuration is already managed by Peasy");
        }
        let mut diff = module_diff(&before, &after)?;
        diff.push(DiffLine { kind: DiffKind::Context, text: "Persistent NetworkManager profiles. Enables NetworkManager; IPv6 is disabled on these profiles. Shared IPv4 opens DNS and DHCP on the specified interface. May interrupt connectivity. Passwords are requested locally at activation; they are never stored in Nix. Active connections may need deactivation and activation to use changed profiles.".into() });
        if let Some(id) = &plan.activate {
            diff.push(DiffLine {
                kind: DiffKind::Context,
                text: format!(
                    "After saving, prepare a separate local activation review for profile {id}."
                ),
            });
        }
        Ok(Preview {
            packages: vec![],
            before,
            change: ProposalChange::Network { plan },
            title: "Configure network profiles".into(),
            diff,
        })
    }
}
