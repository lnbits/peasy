# Reliability changes — September 2026

The September review led to portable backups, controlled daemon upgrades,
host-package identity checks, interruption journalling and real progress stages.
Read-only requests retry briefly across daemon restarts; mutations are never
replayed automatically.

Current instructions: [backups](backups.md), [updates](updates.md),
[recovery](install.md#troubleshooting) and [architecture](architecture.md).

Historical native checks passed. Some VM runs were partial, and final fixes
required fresh security/ISO CI; they were not all validated in one local run.
Detailed chronology remains in Git history. Use the
[release checks](release-validation.md) for the current commit.
