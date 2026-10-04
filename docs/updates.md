# Updates

Open **Settings → Backups and updates → Check for updates**. If a supported stable
release is available, choose **Update Peasy**, review, apply and authenticate.
Reopen Peasy afterward; its system service restarts automatically.

Updates replace Peasy's application and module together, preserving host
configuration, hardware, Nixpkgs and the flake lock. Builds can take several minutes.
Custom `services.peasy.package` overrides may conflict with the updater.

Older installations need one normal host rebuild with an updater-capable version.
Releases without `peasy-update.json` also require a host-managed update.

A failed build restores the previous state. For uncertain activation, use
**System status and recovery**. Backups do not change the destination's Peasy version.

Disable update checks with:

```nix
services.peasy.updates.enable = false;
```

This keeps any already selected release pin. To publish a release, follow
[release validation](release-validation.md) and [ISO publication](iso.md#ci-and-publication).
