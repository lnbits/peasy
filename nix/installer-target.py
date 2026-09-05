"""Stage only the immutable Peasy distribution files in Calamares' target.

This root-only installation helper is not exposed through Peasy or its AI.
Refuse unexpected existing files instead of overwriting an administrator's work.
"""
import hashlib
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile

SOURCE = Path("@source@")
MODULE = Path("@module@")


def trusted_directory(path):
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
        raise ValueError(f"unsafe installation directory: {path}")


def same_file(source, target):
    # Do not cache comparisons by size/mtime across retries.
    with source.open("rb") as left, target.open("rb") as right:
        return hashlib.file_digest(left, "sha256").digest() == hashlib.file_digest(right, "sha256").digest()


def same_tree(source, target):
    """An interrupted installation may be retried, but never merge other data."""
    trusted_directory(target)
    if sorted(os.listdir(source)) != sorted(os.listdir(target)):
        return False
    for entry in source.iterdir():
        copied = target / entry.name
        info = copied.lstat()
        if entry.is_dir():
            if not same_tree(entry, copied):
                return False
        elif (
            not stat.S_ISREG(info.st_mode)
            or info.st_uid != 0
            or info.st_mode & 0o022
            or not same_file(entry, copied)
        ):
            return False
    return True


def install(root, source=SOURCE, module=MODULE):
    if os.geteuid() != 0:
        raise PermissionError("Peasy target installation requires root")
    root = Path(root)
    if not root.is_absolute() or root.resolve() != root or root == Path("/"):
        raise ValueError("target must be a canonical absolute mount point, not /")
    if not root.is_mount():
        raise ValueError("target is not a mounted filesystem")
    if root.samefile("/"):
        raise ValueError("target aliases the running root filesystem")
    for directory in [root, root / "etc", root / "etc/nixos"]:
        trusted_directory(directory)
    destination = root / "etc/nixos"
    tree = destination / "peasy"
    entry = destination / "peasy.nix"
    if os.path.lexists(tree) and not same_tree(source, tree):
        raise ValueError("existing /etc/nixos/peasy differs; refusing to overwrite it")
    if os.path.lexists(entry):
        info = entry.lstat()
        if (
            not stat.S_ISREG(info.st_mode)
            or info.st_uid != 0
            or info.st_mode & 0o022
            or not same_file(module, entry)
        ):
            raise ValueError("existing /etc/nixos/peasy.nix differs; refusing to overwrite it")
    # Only root can modify the validated parent. Publish each complete item
    # atomically; identical existing items are accepted on a retry.
    with tempfile.TemporaryDirectory(prefix=".peasy-install-", dir=destination) as staging:
        staging = Path(staging)
        if not tree.exists():
            shutil.copytree(source, staging / "peasy")
            os.rename(staging / "peasy", tree)
        if not entry.exists():
            shutil.copyfile(module, staging / "peasy.nix")
            os.chmod(staging / "peasy.nix", 0o644)
            os.rename(staging / "peasy.nix", entry)


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("expected exactly one target mount point")
        install(sys.argv[1])
    except (OSError, ValueError) as error:
        print(f"Peasy target installation failed: {error}", file=sys.stderr)
        sys.exit(1)
