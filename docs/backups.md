# Backup and restore

Peasy's **Export backup** restores its software selection and appearance settings
onto an already-installed NixOS machine. The destination keeps its disks,
bootloader, drivers, CPU architecture, user accounts and host configuration.
Traditional and flake hosts use the same portable managed module.

## What's included

| File | Purpose |
| --- | --- |
| `peasy-managed.nix` | Portable standalone packages, appearance preferences and pinned pea instructions from the active generation |
| `RESTORE-REVIEW.json` | Full active Peasy state, including service setups, network profiles, resource settings and AppImages to recreate after destination review |
| `host-reference/` | Original host source, including hardware modules and flake files, when readable; reference only |
| `peasy/` | Bundled Peasy source and installation guide |
| `README.txt` | Restore commands, counts of deferred items and any unavailable host archive |
| `INVENTORY.json` | Included files, exclusions and archive availability |

Pending changes that have not activated are not part of the portable state.
Service setups may depend on existing services or user identities, network
profiles on interface names, and AppImages on CPU architecture. They are retained
in the review file, not applied unchanged. Recreate them through Peasy on the new
machine so its normal discovery, review and authorization checks run there.

Resource settings include account identities, mounts, firewall policy and power
policy. Portable restore preserves the destination's resource settings; the
original values remain reference data in `RESTORE-REVIEW.json`.

The original host archive can contain arbitrary Nix code, external imports and
inline secrets. Peasy cannot automatically separate all hardware dependencies
from that code. Its files are never imported by the portable module. Migrate
wanted administrator-managed settings separately. An unreadable or unsupported
source tree is reported as unavailable; no partial archive is presented as complete.

## Restore

1. On the destination, open Peasy settings → **Backups and updates → Restore backup** and select the
   exported folder. Peasy must already be installed and configured; no AI provider
   is needed. Use the backup's Peasy version or a newer compatible release.
2. Choose **Merge** (default) or **Replace**. Merge adds the saved standalone
   packages and pea instructions, retaining the existing selection. Saved
   appearance choices take precedence. Replace uses the backup's complete
   standalone package, appearance and pea selection. Both modes preserve the
   destination's service setups, network profiles and AppImages.
3. Read the list of saved items that require destination-specific choices, then
   click **Review restore**. Review the configuration diff, resolved package
   versions and any pea permissions. Nothing changes before Apply.
4. Click **Apply** and authenticate. Peasy builds and activates using its existing
   transaction, cancellation and recovery flow. The destination's hardware,
   host configuration, flake lock and package set remain authoritative.
5. Recreate the listed service setups, network profiles and AppImages through
   Peasy with the destination's user, interface and release choices.

A conflicting pea version in Merge stops the proposal; use Replace to restore the
saved selection. Newly restored pea pins must be compatible, allowed by policy
and verified as published in the official repository's main history. Verification
needs network access. Packages unavailable for the destination stop the proposal;
Peasy does not silently omit them or partially activate a restore.

The folder's portable module and review record must agree. Peasy reads neither
archived host modules nor bundled executables during restore. Older exports
without the portable format need to be exported again with a current Peasy.

Cancel before activation to leave the prior configuration in place. Failed builds
restore the previous managed state; uncertain activation uses **System status and
recovery**, also available as `peasy --recover`. Normal NixOS generation rollback
remains available after activation.

## Data and privacy

Personal files, databases, provider credentials and Wi-Fi passwords need separate
backups. A NixOS rollback does not reverse file or database changes. Store backups
off the original machine to protect against disk failure.

Exports are private. Common secret filenames, Git metadata and recovery files are
excluded, but inline secrets in host source can remain. Review before sharing;
restore excluded secrets separately from your secure backup.
