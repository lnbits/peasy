# Backup and restore

Use **Settings → Backups and updates → Export backup** to save Peasy's installed
software selection, appearance and pea instructions.

This is a configuration backup. Personal files, databases and credentials need
separate backups. Keep a copy off the original machine.

## Restore

1. Install Peasy on the destination, using the backup's version or newer.
2. Choose **Restore backup** and select the exported folder. No AI account is needed.
3. Choose **Merge** to add the saved selection, or **Replace** to use it instead.
   Saved appearance takes precedence in both modes.
4. Review, apply and authenticate.
5. Recreate listed service setups, network profiles and AppImages for the new machine.

Both modes preserve destination hardware, accounts, host configuration, flake lock
and existing service/network/resource settings. Missing packages or incompatible
pea pins stop the restore; verifying new pea pins requires Internet access.

`RESTORE-REVIEW.json` lists deferred settings. `host-reference/` contains optional
original configuration for reference, never automatic import. `INVENTORY.json`
lists exclusions. Archived host source may contain inline secrets: review before sharing.

For an interrupted restore, use **System status and recovery** or `peasy --recover`.
Generation rollback does not undo file or database changes.
