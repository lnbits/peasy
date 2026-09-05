"""Target helper checks without root, mounts or host system mutations."""
import importlib.util
import os
from pathlib import Path
import stat
import sys
import tempfile
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("installer_helper", sys.argv[1])
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
real_lstat = Path.lstat


def fixture_lstat(path, *args, **kwargs):
    info = real_lstat(path, *args, **kwargs)
    values = list(info)
    values[4] = 0  # Simulate root ownership, preserving real type/mode bits.
    return os.stat_result(values)


def rejected(action):
    try:
        action()
    except (OSError, ValueError):
        return
    raise AssertionError("unsafe operation unexpectedly accepted")


with tempfile.TemporaryDirectory() as temporary:
    base = Path(temporary)
    source = base / "source"
    source.mkdir()
    (source / "module.nix").write_text("{ ... }: {}\n")
    module = base / "entry.nix"
    module.write_text("{ imports = [ ./peasy/module.nix ]; }\n")
    root = base / "target"
    (root / "etc/nixos").mkdir(parents=True)
    outside = base / "outside"
    outside.mkdir()
    sentinel = outside / "sentinel"
    sentinel.write_text("do not change")
    with (
        patch.object(helper.os, "geteuid", return_value=0),
        patch.object(Path, "lstat", fixture_lstat),
        patch.object(Path, "is_mount", lambda path: path == root),
    ):
        rejected(lambda: helper.install("/", source, module))
        rejected(lambda: helper.install("relative", source, module))
        rejected(lambda: helper.install(base, source, module))
        with patch.object(helper.os, "geteuid", return_value=1000):
            rejected(lambda: helper.install(root, source, module))
        with patch.object(Path, "samefile", return_value=True):
            rejected(lambda: helper.install(root, source, module))
        (root / "etc/nixos").chmod(0o777)
        rejected(lambda: helper.install(root, source, module))
        (root / "etc/nixos").chmod(0o755)
        entry = root / "etc/nixos/peasy.nix"
        entry.symlink_to(sentinel)
        rejected(lambda: helper.install(root, source, module))
        assert sentinel.read_text() == "do not change"
        entry.unlink()
        tree = root / "etc/nixos/peasy"
        tree.symlink_to(outside, target_is_directory=True)
        rejected(lambda: helper.install(root, source, module))
        tree.unlink()
        helper.install(root, source, module)
        helper.install(root, source, module)  # Safe retry, no data changes.
        assert entry.read_bytes() == module.read_bytes()
        assert (tree / "module.nix").read_bytes() == (source / "module.nix").read_bytes()
        assert stat.S_IMODE(entry.stat().st_mode) == 0o644
        copied = tree / "module.nix"
        copied.unlink()
        copied.symlink_to(sentinel)
        rejected(lambda: helper.install(root, source, module))
        copied.unlink()
        copied.write_bytes((source / "module.nix").read_bytes())
        entry.chmod(0o666)
        rejected(lambda: helper.install(root, source, module))
        entry.chmod(0o644)
        entry.write_text("administrator changes")
        rejected(lambda: helper.install(root, source, module))
        assert entry.read_text() == "administrator changes"
        entry.write_bytes(module.read_bytes())
        (tree / "module.nix").write_text("administrator changes")
        rejected(lambda: helper.install(root, source, module))
        assert not list((root / "etc/nixos").glob(".peasy-install-*"))
        assert sentinel.read_text() == "do not change"
print("target helper safety and retry checks passed")
