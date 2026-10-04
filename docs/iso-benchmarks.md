# Historical ISO measurements

Measured 2026-09-06 on an Intel Core Ultra 7 255H with KVM/QEMU 11.1.0.
VMs used four CPUs, 8 GiB RAM and a fresh 32 GiB disk, with no network or host store.

| Corrected image | Install job | Complete fresh test | Reused-base test |
| --- | ---: | ---: | ---: |
| GNOME / BIOS | 3m 54s | 11m 19s | 3m 23s |
| Plasma / UEFI | 5m 20s | 14m 53s | 5m 53s |

These were successful offline installs and package lifecycle checks. Runs partly
overlapped; timings are not controlled desktop comparisons or hardware requirements.
The earlier v0.1.4 baseline failed installation, so no speedup against it is claimed.

[Raw results](iso-benchmark-results.json) contain image hashes and timings.
These old snapshots do not validate current releases. Follow the
[current ISO procedure](iso.md#verification) for new measurements.
