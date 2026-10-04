# Release review — 24 September 2026

Fixes covered pea-schema compatibility, PostgreSQL inspection permissions,
backup ownership, stale-build guards, TLS dependencies, headless recovery,
appearance restoration, cancellation cleanup and Wi-Fi selection.

Historical validation passed Rust/Wasm checks (155 tests), dependency audit,
release-script tests, portable-export evaluation and the headless package.
The final Plasma correction had focused coverage. KVM was unavailable; new VM
cases and AArch64 were not run locally. No host activation or release was performed.

These results do not approve later source changes. Run the
[current release gates](release-validation.md), including actual-ISO tests.
Full historical findings remain in Git history.
