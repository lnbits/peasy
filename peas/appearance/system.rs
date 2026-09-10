//! appearance pea: privileged, validated proposals; activation stays in NixBackend.
use super::{NixBackend, Preview};
use anyhow::{Result, bail};
use peasy_core::{ProposalChange, ThemeSettings, module_diff};

impl NixBackend {
    pub fn preview_theme(&self, theme: ThemeSettings) -> Result<Preview> {
        let before = self.current_state()?;
        let after = before.with_theme(&theme)?;
        if before == after {
            bail!("that appearance is already selected");
        }
        let mut details = Vec::new();
        if let Some(color) = theme.accent_color {
            details.push(format!("{color} accent"));
        }
        if let Some(scheme) = theme.color_scheme {
            details.push(format!("{scheme} mode"));
        }
        Ok(Preview {
            packages: vec![],
            diff: module_diff(&before, &after)?,
            before,
            change: ProposalChange::Theme { theme },
            title: format!("Change appearance to {}", details.join(" and ")),
        })
    }
}
