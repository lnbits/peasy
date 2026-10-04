# Release checks

Run from the repository root on Linux with KVM:

```sh
bash scripts/check-release.sh
```

This runs Rust/Wasm tests, formatting, Clippy, generated-catalogue verification,
a current dependency audit and Nix checks. It does not switch the host or contact
an AI provider. Network access is needed for uncached inputs and current advisories.

For a development-only Rust check:

```sh
nix develop --command bash scripts/check-rust.sh
```

Before publishing, require passing CI for the exact commit, including sandbox,
networking, PostgreSQL, pea-fetch, desktop and resource tests. The
[ISO workflow](iso.md#ci-and-publication) also requires fresh offline BIOS/UEFI
installations of the actual release image. Reused VM bases are not release evidence.

Check real hardware separately: installer/tray appearance, authentication,
install/remove, cancellation, recovery, rollback, Wi-Fi and Bluetooth. Fixture
success does not establish live-model accuracy or every hardware combination.

Record commit, architecture, results and skipped checks in release evidence.
The historical 2026-09-05 run passed at `2fd8a71767a78e3df34e5102441c3478e50c722a`
on x86_64; it did not test AArch64 or real-host activation and is not evidence for
current source. Older detailed logs remain in Git history.
