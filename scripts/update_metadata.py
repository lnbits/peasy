"""Build the authenticated updater asset from the exact tagged GitHub source."""
import argparse
import base64
import json
from pathlib import Path
import re
import subprocess
import tomllib

ASSET = 'peasy-update.json'


def validate(metadata, tag, commit):
    if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('Self-updates require a stable vMAJOR.MINOR.PATCH release tag')
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Release commit must be a full lowercase SHA')
    if set(metadata) != {'format', 'tag', 'version', 'revision', 'sha256'}:
        raise ValueError('Invalid updater metadata fields')
    if type(metadata['format']) is not int or metadata['format'] != 1 or metadata['tag'] != tag or metadata['revision'] != commit or metadata['version'] != tag[1:]:
        raise ValueError('Updater metadata does not match the release')
    if not isinstance(metadata['sha256'], str) or not re.fullmatch(r'[0-9a-f]{64}', metadata['sha256']):
        raise ValueError('Invalid source NAR hash')
    return metadata


def prepare(tag, commit, run=subprocess.check_output):
    placeholder = {'format': 1, 'tag': tag, 'version': tag[1:], 'revision': commit, 'sha256': '0' * 64}
    validate(placeholder, tag, commit)
    url = f'https://codeload.github.com/lnbits/peasy/tar.gz/{commit}'
    prefetched = json.loads(run(['nix', 'store', 'prefetch-file', '--unpack', '--json', url], text=True))
    source = Path(prefetched['storePath'])
    if not source.is_absolute() or not str(source).startswith('/nix/store/'):
        raise ValueError('Invalid prefetched release store path')
    cargo = tomllib.loads((source / 'Cargo.toml').read_text())
    if cargo['workspace']['package']['version'] != tag[1:]:
        raise ValueError('Release tag must match Cargo.toml workspace.package.version')
    if not (source / 'nix/update-module.nix').is_file():
        raise ValueError('Release does not support reviewed self-updates')
    kind, value = prefetched['hash'].split('-', 1)
    raw = base64.b64decode(value, validate=True)
    if kind != 'sha256' or len(raw) != 32:
        raise ValueError('Unexpected source hash format')
    return validate(placeholder | {'sha256': raw.hex()}, tag, commit)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.write_text(json.dumps(prepare(args.tag, args.commit), indent=2) + '\n')
