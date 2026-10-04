# Firewall pea

Inspect structured NixOS firewall configuration and interfaces. Replace Peasy-owned TCP/UDP port and trusted-interface contributions through reviewed NixOS state. Preserve unrelated Peasy entries and administrator rules. Trusted interfaces allow all incoming traffic; never substitute them for source-restricted rules. No arbitrary nftables expressions, source-limited rules or guarantee that a removed port is closed.

[Native adapter](native.rs) · [Shared resource contract](../../docs/resources.md)
