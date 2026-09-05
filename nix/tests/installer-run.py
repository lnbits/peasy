"""Run the real NixOS Calamares job in a disposable installer VM.

Only the UI storage/progress interface is substituted. All installer processes,
target writes, hardware generation and nixos-install run for real. A test-only
module supplies the VM control console and a local test user's password.
"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import types

desktop, firmware, source = sys.argv[1:]
values = {
    "rootMountPoint": "/mnt",
    "firmwareType": firmware,
    "bootLoader": {"installPath": "/dev/vda"},
    "partitions": [],
    "packagechooser_packagechooser": desktop,
    "username": "peasytest",
    "fullname": "Peasy Test",
    "hostname": "peasy-installed",
    "autoLoginUser": "peasytest",
}


def host_process(argv, unused=None, contents=None):
    subprocess.run(argv, input=contents, text=True, check=True)


def log(message):
    # The VM driver captures stdout until the job exits. Send installer logs to
    # the serial console as they arrive so stalled offline builds are diagnosable.
    print(message, file=sys.stderr, flush=True)


calamares = types.ModuleType("libcalamares")
calamares.globalstorage = types.SimpleNamespace(value=values.get)
calamares.job = types.SimpleNamespace(setprogress=lambda unused: None)
calamares.utils = types.SimpleNamespace(
    gettext_path=lambda: "/nonexistent", gettext_languages=lambda: ["en"],
    warning=log, error=log, debug=log,
    host_env_process_output=host_process,
)
sys.modules["libcalamares"] = calamares
spec = importlib.util.spec_from_file_location("nixos_job", source)
job = importlib.util.module_from_spec(spec)
spec.loader.exec_module(job)
assert Path("/mnt/etc/nixos/test-instrumentation.nix").is_file()
# This fixture import exists only in the test, never in the shipped extension.
job.cfghead = job.cfghead.replace(
    "      ./hardware-configuration.nix\n",
    "      ./hardware-configuration.nix\n      ./test-instrumentation.nix\n",
)
result = job.run()
if result is not None:
    raise RuntimeError(result)
