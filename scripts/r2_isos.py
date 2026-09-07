"""Full ISO delivery through a bucket-scoped R2 client; no credentials in metadata."""
import hashlib
import json
import os
import re
from urllib.parse import urlsplit
from urllib.request import build_opener, HTTPRedirectHandler, Request

from join_iso import digest

PREFIX = "peasy/releases/"
PUBLIC_USER_AGENT = "Peasy-ReleaseVerifier/1.0 (+https://github.com/lnbits/peasy)"
TAG = r"v[A-Za-z0-9._-]+"
COMMIT = r"(?:[0-9a-f]{40}|[0-9a-f]{64})"
OWNED_KEY = re.compile(
    rf"{PREFIX}(?P<tag>{TAG})/(?P<commit>{COMMIT})/"
    r"peasy-nixos-(?P=tag)-(?P<desktop>gnome|plasma)-x86_64\.iso"
)


def origin(value):
    if not value or any(char.isspace() for char in value):
        raise ValueError("R2 URLs must not be empty or contain whitespace")
    url = urlsplit(value)
    if (url.scheme != "https" or not url.hostname or url.username or url.password
            or url.port is not None or url.path not in ("", "/") or url.query or url.fragment):
        raise ValueError("R2 URLs must be HTTPS origins, without a bucket path or credentials")
    return value.rstrip("/")


def configuration(env=os.environ):
    endpoint = origin(env["R2_ENDPOINT_URL"])
    if not re.fullmatch(r"[0-9a-f]{32}(?:\.(?:eu|us|fedramp))?\.r2\.cloudflarestorage\.com",
                        urlsplit(endpoint).hostname):
        raise ValueError("Expected the Cloudflare R2 S3 endpoint")
    bucket = env["R2_BUCKET"]
    if not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket):
        raise ValueError("Invalid R2 bucket")
    return endpoint, bucket, origin(env["R2_PUBLIC_URL"])


def client(endpoint):
    # All dependencies are pinned by flake.lock, not installed from mutable pip indexes.
    import boto3
    from botocore.config import Config
    return boto3.client(
        "s3", endpoint_url=endpoint, region_name="auto",
        aws_access_key_id=os.environ["R2_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["R2_SECRET_ACCESS_KEY"],
        config=Config(signature_version="s3v4", connect_timeout=15, read_timeout=120,
                      retries={"mode": "standard", "max_attempts": 5},
                      s3={"addressing_style": "path"},
                      request_checksum_calculation="when_required",
                      response_checksum_validation="when_required"),
    )


def prepare(source, output, tag, commit, public_url):
    if not re.fullmatch(TAG, tag) or not re.fullmatch(COMMIT, commit):
        raise ValueError("Expected a safe v* tag and exact Git commit")
    public_url = origin(public_url)
    manifest = {"schema": 1, "tag": tag, "commit": commit, "images": []}
    # Publish one GNOME live image; the installer offers GNOME or XFCE.
    # Keep old Plasma keys recognizable above for latest-only retention.
    for desktop in ("gnome",):
        status = json.loads((source / f"{desktop}-iso-status.json").read_text())
        if any(status.get(key) is not True for key in
               ("releaseReady", "installedTargetHasPeasy", "installedBootVerified")):
            raise ValueError(f"{desktop}: installed-system verification required")
        name = f"peasy-nixos-{tag}-{desktop}-x86_64.iso"
        image = source / name
        if image.is_symlink() or not image.is_file() or image.stat().st_size == 0:
            raise ValueError(f"Missing ISO: {name}")
        checksum = digest(image)
        if (source / f"{name}.sha256").read_text().strip() != f"{checksum}  {name}":
            raise ValueError(f"Invalid checksum: {name}")
        manifest["images"].append({"desktop": desktop, "name": name,
                                   "size": image.stat().st_size, "sha256": checksum,
                                   "url": f"{public_url}/{PREFIX}{tag}/{commit}/{name}"})
    assets = []
    for item in manifest["images"]:
        checksum_file = output / f"{item['name']}.sha256"
        checksum_file.write_text(f"{item['sha256']}  {item['name']}\n")
        assets.append(checksum_file)
    checksums = output / "SHA256SUMS"
    checksums.write_text("".join(f"{item['sha256']}  {item['name']}\n" for item in manifest["images"]))
    metadata = output / "iso-downloads.json"
    metadata.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest, assets + [checksums, metadata]


def marker(manifest):
    return "<!-- peasy-iso-downloads:" + json.dumps(manifest, separators=(",", ":"), sort_keys=True) + " -->"


def notes(manifest):
    tag, commit = manifest["tag"], manifest["commit"]
    labels = {"gnome": "GNOME", "plasma": "KDE Plasma"}
    links = "\n".join(f"- [{labels[item['desktop']]} ISO]({item['url']}) "
                      f"({item['size'] / 1024**3:.2f} GiB, x86_64)" for item in manifest["images"])
    return f"""<!-- peasy-iso-release:{commit} -->
{marker(manifest)}
# Peasy {tag}

Built from `{commit}`. The GNOME ISO passed CI checks and fresh offline
GNOME installation/boot tests with BIOS and UEFI. Physical-hardware testing
is still recommended.

## Download

{links}

Download the complete, bootable ISO, verify it, then boot it in a VM or write it
to a USB drive. The live desktop is GNOME. Choose GNOME or lightweight XFCE in
the installer; XFCE installation requires an Internet connection.
Peasy is included in either installed desktop.
Configure your own AI provider afterward; no API keys are bundled.

Download the matching `.iso.sha256` asset below into the same folder and run:

```console
sha256sum --check peasy-nixos-{tag}-gnome-x86_64.iso.sha256
```

`SHA256SUMS` contains the whole-image hash.
Checksums detect corruption; they are not independent proof of publisher
trust. Source and build instructions are available in the tagged repository.

Only the latest release's ISOs are retained in R2. Older ISO links expire after
a newer release is verified and published; GitHub checksums and sources remain.
Already running NixOS? See [traditional installation](https://github.com/lnbits/peasy#install-on-nixos).
"""


def key_for(manifest, item):
    return f"{PREFIX}{manifest['tag']}/{manifest['commit']}/{item['name']}"


def metadata_for(manifest, item):
    return {"sha256": item["sha256"], "peasy-commit": manifest["commit"],
            "peasy-tag": manifest["tag"]}


def head(s3, bucket, key):
    try:
        return s3.head_object(Bucket=bucket, Key=key)
    except Exception as error:
        # Authentication/network failures must never be mistaken for an absent object.
        code = getattr(error, "response", {}).get("Error", {}).get("Code")
        if code not in ("404", "NoSuchKey", "NotFound"):
            raise
        return None


def upload_file(s3, bucket, key, path, metadata):
    from boto3.s3.transfer import TransferConfig
    s3.upload_file(str(path), bucket, key,
                   ExtraArgs={"Metadata": metadata, "ContentType": "application/octet-stream",
                              "ContentDisposition": f'attachment; filename="{path.name}"',
                              "CacheControl": "public, max-age=31536000, immutable"},
                   Config=TransferConfig(multipart_threshold=64 * 1024**2,
                                         multipart_chunksize=64 * 1024**2, max_concurrency=4))


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def verify_public(item):
    # Public-domain access is separate from authenticated S3 access; do not publish
    # a release with a disabled/misconfigured download domain. Never send credentials.
    # Identify this anonymous verifier honestly. Cloudflare can block urllib's
    # generic Python user-agent even when the uploaded object is publicly readable.
    request = Request(item["url"], method="HEAD", headers={"User-Agent": PUBLIC_USER_AGENT})
    with build_opener(NoRedirect).open(request, timeout=60) as response:
        if response.status != 200 or int(response.headers.get("Content-Length", -1)) != item["size"]:
            raise ValueError("Public ISO URL is not serving the complete image")


def deliver(s3, bucket, source, manifest, already_public=False,
            upload=upload_file, public_check=verify_public):
    for item in manifest["images"]:
        key = key_for(manifest, item)
        expected = metadata_for(manifest, item)
        info = head(s3, bucket, key)
        if info is None:
            if already_public:
                raise ValueError("Published ISO is missing (older releases may have expired); refusing to restore it")
            print(f"Uploading complete {item['desktop']} ISO to R2", flush=True)
            upload(s3, bucket, key, source / item["name"], expected)
            info = head(s3, bucket, key)
        if not info or info.get("ContentLength") != item["size"] or info.get("Metadata") != expected:
            raise ValueError(f"R2 object differs; refusing to overwrite: {item['name']}")
        print(f"Verifying complete {item['desktop']} ISO from R2", flush=True)
        response = s3.get_object(Bucket=bucket, Key=key, IfMatch=info["ETag"])
        body = response["Body"]
        sha, size = hashlib.sha256(), 0
        try:
            while block := body.read(4 * 1024**2):
                sha.update(block)
                size += len(block)
        finally:
            body.close()
        if size != item["size"] or sha.hexdigest() != item["sha256"]:
            raise ValueError(f"R2 whole-image SHA-256 verification failed: {item['name']}")
        public_check(item)


def prune(s3, bucket, manifest, repo, run):
    """Delete only owned old ISO objects, after confirming this is the public latest."""
    def is_latest():
        latest = json.loads(run("api", f"repos/{repo}/releases/latest"))
        return (latest.get("draft") is False and latest.get("prerelease") is False
                and latest.get("tag_name") == manifest["tag"]
                and marker(manifest) in (latest.get("body") or ""))

    if not is_latest():
        print("Not GitHub's latest release; leaving all R2 ISOs untouched.")
        return
    keep = {key_for(manifest, item) for item in manifest["images"]}
    candidates = []
    for page in s3.get_paginator("list_objects_v2").paginate(Bucket=bucket, Prefix=PREFIX):
        for obj in page.get("Contents", []):
            key = obj["Key"]
            match = OWNED_KEY.fullmatch(key)
            if key in keep or not match:
                continue
            info = head(s3, bucket, key)
            metadata = (info or {}).get("Metadata", {})
            if (metadata.get("peasy-tag") == match["tag"]
                    and metadata.get("peasy-commit") == match["commit"]
                    and re.fullmatch(r"[0-9a-f]{64}", metadata.get("sha256", ""))):
                candidates.append(key)
    # Keep listing and deleting separate so pagination cannot skip shifted objects.
    for key in candidates:
        if not is_latest():
            raise ValueError("Latest release changed during cleanup; stopping without further deletion")
        s3.delete_object(Bucket=bucket, Key=key)
        print(f"Removed superseded ISO: {key}", flush=True)
