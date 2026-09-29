# Updating Peasy

Open **Settings**. Peasy checks the latest published stable release from
[lnbits/peasy](https://github.com/lnbits/peasy/releases) in the background.
Successful checks are cached for six hours. **Check for updates** refreshes the
result, with a 30-second minimum interval between successful checks.
No AI account is needed, and checking adds no steps to software installation.

When a newer release has verified updater metadata, **Update Peasy** appears.
Click it, review the version and changes, then **Apply** and authenticate.
Peasy downloads the pinned source, builds a NixOS generation and activates it.
Close and reopen Peasy afterward. Its system service restarts automatically.
Building from source can take several minutes.

The update replaces Peasy's application and NixOS module together. It keeps your
host configuration, hardware configuration, Nixpkgs and flake lock unchanged.
Traditional, flake-based and Peasy ISO-installed hosts use the same flow.
A custom `services.peasy.package` override can conflict with the selected release;
remove the override or continue managing Peasy through your host configuration.
If a release requires newer Nixpkgs, update your host first.
After an in-app update, Peasy's managed release pin takes precedence over the
original Peasy import or flake input. Disabling update checks leaves that
installed version selected.

A failed build restores the previous Peasy state. If activation fails, use
**System status and recovery**. Previous NixOS generations remain available for
rollback until garbage-collected. A portable **Restore backup** preserves the
destination's Peasy version; it does not install the source computer's version.

Offline checks and GitHub rate limits show an error and allow retrying. They do
not mean Peasy is up to date. Drafts and prereleases are excluded. Releases
without `peasy-update.json` need a normal host-managed update. Existing installs
need one normal rebuild with the updater-capable module before this button works.

To turn off release checks and updates:

```nix
services.peasy.updates.enable = false;
```

## Publishing an updatable release

Set `workspace.package.version` in `Cargo.toml` and the corresponding workspace
package versions in `Cargo.lock` to the release version, then use a stable
`vMAJOR.MINOR.PATCH` tag. `cargo update --workspace` refreshes the lockfile's
workspace versions. Run `python3 scripts/update_metadata.py --tag vX.Y.Z --check`
with your intended tag and the release checks described in the README.
The release workflow generates `peasy-update.json` from the exact tagged GitHub
source before loading upload credentials. It verifies the source's version and
updater module, records the commit and unpacked NAR SHA-256, and publishes the
metadata with the other verified release assets and checksums. A tag/version
mismatch blocks publication.

The client accepts only the official repository's published asset and checks
that the tag still resolves to its recorded commit. The daemon repeats that
check at review and after authorization. Nix verifies the source hash while
fetching. The source is fetched through a Nix daemon build and retained by the
generation, allowing Peasy’s own privileged service to keep its network sandbox.
Updates execute native code, so approval trusts the Peasy release
maintainers and GitHub's release distribution. This is separate from installing
data-only peas.
