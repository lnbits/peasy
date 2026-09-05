"""Reassemble a Peasy ISO without overwriting files; verify every SHA-256."""
import hashlib
import json
from pathlib import Path
import re
import sys


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def join(manifest):
    manifest = Path(manifest)
    data = json.loads(manifest.read_text())
    name = data["name"]
    if not re.fullmatch(r"peasy-nixos-v[A-Za-z0-9._-]+-(gnome|plasma)-x86_64\.iso", name):
        raise ValueError("Invalid ISO filename")
    if not re.fullmatch(r"[0-9a-f]{64}", data["sha256"]):
        raise ValueError("Invalid ISO checksum")
    parts = data["parts"]
    if not 1 <= len(parts) <= 999:
        raise ValueError("Invalid part count")
    for index, part in enumerate(parts, 1):
        if part["name"] != f"{name}.part{index:03}":
            raise ValueError("Invalid or out-of-order part filename")
        path = manifest.parent / part["name"]
        if path.is_symlink() or not path.is_file() or path.stat().st_size != part["size"]:
            raise ValueError(f"Missing or invalid part: {path.name}")
    output = manifest.parent / name
    # Exclusive creation: an existing ISO (or symlink) is never overwritten.
    with output.open("xb") as destination:
        try:
            complete = hashlib.sha256()
            for part in parts:
                checksum = hashlib.sha256()
                with (manifest.parent / part["name"]).open("rb") as source:
                    while block := source.read(4 * 1024 * 1024):
                        checksum.update(block)
                        complete.update(block)
                        destination.write(block)
                if checksum.hexdigest() != part["sha256"]:
                    raise ValueError(f"Checksum mismatch: {part['name']}")
            if destination.tell() != data["size"] or complete.hexdigest() != data["sha256"]:
                raise ValueError("Reconstructed ISO checksum mismatch")
        except BaseException:
            # Only remove the new partial output created by this invocation.
            destination.close()
            output.unlink()
            raise
    return output


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("Usage: python3 join_iso.py <downloaded .iso.parts.json>")
    print(f"Verified ISO: {join(sys.argv[1])}")
