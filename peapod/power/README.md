# Power pea

Inspect battery status and available power profiles. Select an existing power-profiles-daemon profile with desktop authorization. Configure persistent logind lid handling and idle suspension through NixOS; desktop inhibitors may override it. Do not claim hibernation works without host support. No automatic installation of competing power managers, sleep inhibition or forced shutdown.

Specify `lid`, `idle_minutes`, or both. Null/omitted fields preserve existing values;
both absent is invalid. Zero disables idle suspension.

[Native adapter](native.rs) · [Shared resource contract](../../docs/resources.md)
