#!/usr/bin/env python3
"""Hash generated pea manifests; --check fails on a stale catalogue."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parent.parent
entries = []
for path in sorted((root / "peas").glob("*/pea.json")):
    raw = path.read_bytes()
    manifest = json.loads(raw)
    entries.append({
        "package": {key: manifest[key] for key in ("id", "version", "host_api", "permissions")}
        | {"hash": hashlib.sha256(raw).hexdigest()},
        "capabilities": manifest["capabilities"],
    })
text = json.dumps({"format": 1, "peas": entries}, indent=2) + "\n"
path = root / "peas/catalogue.json"
if "--check" in sys.argv:
    if path.read_text() != text:
        raise SystemExit("pea catalogue is stale; regenerate manifests and catalogue")
else:
    path.write_text(text)
