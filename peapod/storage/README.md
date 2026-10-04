# Storage pea

Inspect disks, filesystem UUIDs and mount state. Mount, unmount or explicitly format only a discovered writable removable leaf filesystem whose entire disk is non-system. Formatting erases data and requires explicit user intent and review. Persistent mounts use discovered UUIDs under /mnt/peasy-NAME. No partition editing, internal-disk mutations, LUKS operations or secret input.

[Native adapter](native.rs) · [Shared resource contract](../../docs/resources.md)
