"""Prepare ISO assets; publish only after every upload is verified."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from join_iso import digest

ASSET_LIMIT = 2 * 1024**3  # GitHub requires strictly less than 2 GiB.
PART_SIZE = 1024**3


def prepare(source, output, tag, commit, asset_limit=ASSET_LIMIT, part_size=PART_SIZE):
    if not re.fullmatch(r"v[A-Za-z0-9._-]+", tag):
        raise ValueError("Expected a v* tag without slashes or shell metacharacters")
    if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", commit):
        raise ValueError("Expected an exact Git commit")
    if not 0 < part_size < asset_limit:
        raise ValueError("Part size must be below the asset limit")
    images = []
    # Validate BOTH complete images before creating a release or any parts.
    for desktop in ("gnome", "plasma"):
        status = json.loads((source / f"{desktop}-iso-status.json").read_text())
        if any(status.get(key) is not True for key in ("releaseReady", "installedTargetHasPeasy", "installedBootVerified")):
            raise ValueError(f"{desktop}: installed-system verification required")
        image = source / f"peasy-nixos-{tag}-{desktop}-x86_64.iso"
        if image.is_symlink() or not image.is_file() or image.stat().st_size == 0:
            raise ValueError(f"Missing ISO: {image.name}")
        checksum = digest(image)
        if (source / f"{image.name}.sha256").read_text().strip() != f"{checksum}  {image.name}":
            raise ValueError(f"Invalid checksum: {image.name}")
        images.append((image, checksum))
    if shutil.disk_usage(output).free < sum(image.stat().st_size for image, _ in images) + 512 * 1024**2:
        raise ValueError("Not enough free space to prepare release assets")
    assets = []
    for image, checksum in images:
        if image.stat().st_size < asset_limit:
            copied = output / image.name
            shutil.copyfile(image, copied)
            assets.append(copied)
        else:
            parts = []
            with image.open("rb") as stream:
                index = 0
                while stream.tell() < image.stat().st_size:
                    index += 1
                    if index > 999:
                        raise ValueError("Too many ISO parts")
                    part = output / f"{image.name}.part{index:03}"
                    remaining = part_size
                    sha = hashlib.sha256()
                    with part.open("xb") as destination:
                        while remaining and (block := stream.read(min(4 * 1024 * 1024, remaining))):
                            destination.write(block)
                            sha.update(block)
                            remaining -= len(block)
                    parts.append({"name": part.name, "size": part.stat().st_size, "sha256": sha.hexdigest()})
                    assets.append(part)
            manifest = output / f"{image.name}.parts.json"
            manifest.write_text(json.dumps({"name": image.name, "size": image.stat().st_size,
                                           "sha256": checksum, "commit": commit, "parts": parts}, indent=2) + "\n")
            assets.append(manifest)
        checksum_file = output / f"{image.name}.sha256"
        checksum_file.write_text(f"{checksum}  {image.name}\n")
        assets.append(checksum_file)
    helper = output / "join_iso.py"
    shutil.copyfile(Path(__file__).with_name("join_iso.py"), helper)
    assets.append(helper)
    checksums = output / "SHA256SUMS"
    checksums.write_text("".join(f"{digest(asset)}  {asset.name}\n" for asset in assets))
    assets.append(checksums)
    # Includes manifests/helper as well as ISO data; refuse any oversized asset.
    if any(asset.stat().st_size >= ASSET_LIMIT for asset in assets):
        raise ValueError("A prepared asset exceeds GitHub's limit")
    return assets


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True).strip()


def publish(assets, tag, commit, repo, output, run=gh):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("Invalid repository")
    marker = f"<!-- peasy-iso-release:{commit} -->"
    endpoint = f"repos/{repo}/releases"

    def verify_tag():
        obj = json.loads(run("api", f"repos/{repo}/git/ref/tags/{tag}"))["object"]
        for _ in range(16):
            if obj["type"] != "tag":
                break
            obj = json.loads(run("api", f"repos/{repo}/git/tags/{obj['sha']}"))["object"]
        if obj["type"] != "commit" or obj["sha"] != commit:
            raise ValueError("Remote tag does not match the tested commit")

    def find_release():
        pages = json.loads(run("api", endpoint, "--paginate", "--slurp"))
        matches = [release for page in pages for release in page if release["tag_name"] == tag]
        if len(matches) > 1:
            raise ValueError("Ambiguous release tag")
        return matches[0] if matches else None

    def validate(release, require_draft=True):
        if not release or (require_draft and release.get("draft") is not True) or marker not in (release.get("body") or ""):
            raise ValueError("Refusing to modify a published release or an unrelated draft")

    verify_tag()
    release = find_release()  # API/auth errors are NOT treated as absence.
    if release is None:
        notes = output / "release-notes.md"
        notes.write_text(f"""{marker}
# Peasy {tag}

Built from commit `{commit}`. GNOME and Plasma installer/runtime checks passed in CI.
Published automatically only after both desktops' checks and every asset upload
pass verification. Physical-hardware testing is still recommended.

## Download and test

For each oversized ISO, download all its `.partNNN` files, its `.parts.json`
manifest, and `join_iso.py` into one directory. The parts are not bootable alone.
Reconstruct the selected image (Python 3.11+):

```console
python3 join_iso.py peasy-nixos-{tag}-gnome-x86_64.iso.parts.json
python3 join_iso.py peasy-nixos-{tag}-plasma-x86_64.iso.parts.json
```

The helper verifies each part and the complete ISO, and never overwrites an
existing ISO. If an image is provided directly as `.iso`, no reconstruction is
needed. `SHA256SUMS` covers every uploaded payload; each `.iso.sha256` covers
the complete image. Checksums detect corruption; they are not signing keys.

Flash the resulting ISO with your usual image-writing tool, then test boot and
installation on spare hardware or a VM. Peasy is retained after installation;
configure your own AI provider afterward. Live-session API keys are not copied.
Traditional NixOS installation instructions are in `docs/install.md` in the
tagged source. GitHub also provides the tagged source archives.

If uploads fail, the workflow keeps an incomplete draft private. Rerun the failed
job to resume verified uploads; do not manually publish an incomplete draft.
""")
        run("release", "create", tag, "--repo", repo, "--draft", "--verify-tag",
            "--target", commit, "--title", f"Peasy {tag}", "--notes-file", str(notes))
        release = find_release()
    validate(release, require_draft=False)
    release_endpoint = f"{endpoint}/{release['id']}"
    pages = json.loads(run("api", f"{release_endpoint}/assets", "--paginate", "--slurp"))
    remote = {asset["name"]: asset for page in pages for asset in page}
    expected = {asset.name: (asset.stat().st_size, f"sha256:{digest(asset)}") for asset in assets}

    def matches(asset, metadata):
        return (asset.get("size"), asset.get("digest")) == metadata and asset.get("state") == "uploaded"

    # Never replace a differing asset or silently trust a size-only match.
    for name, metadata in expected.items():
        if name in remote and not matches(remote[name], metadata):
            raise ValueError(f"Existing asset differs or has no verified digest: {name}")
    if release.get("draft") is not True:
        if any(name not in remote for name in expected):
            raise ValueError("Refusing to modify an incomplete published release")
        print(f"Release {tag} is already published and all assets match; no changes.")
        return
    for asset in assets:
        if asset.name not in remote:
            validate(json.loads(run("api", release_endpoint)))
            run("release", "upload", tag, str(asset), "--repo", repo)
    validate(json.loads(run("api", release_endpoint)))
    pages = json.loads(run("api", f"{release_endpoint}/assets", "--paginate", "--slurp"))
    remote = {asset["name"]: asset for page in pages for asset in page}
    if any(name not in remote or not matches(remote[name], metadata) for name, metadata in expected.items()):
        raise ValueError("Draft assets are incomplete or failed digest verification; rerun the job")
    # A temporary draft prevents users downloading an incomplete release.
    # Publication is the final write, only after all remote digests match.
    verify_tag()
    validate(json.loads(run("api", release_endpoint)))
    run("release", "edit", tag, "--repo", repo, "--draft=false")
    published = json.loads(run("api", release_endpoint))
    validate(published, require_draft=False)
    if published.get("draft") is not False:
        raise ValueError("GitHub did not confirm publication")
    print(f"Complete verified release published: {tag}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="peasy-release-") as temporary:
        output = Path(temporary)
        assets = prepare(args.artifacts, output, args.tag, args.commit)
        publish(assets, args.tag, args.commit, os.environ["GH_REPO"], output)
