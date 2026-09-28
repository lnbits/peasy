"""Release pins come from the downloaded tagged source, never the CI checkout."""
import base64
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import update_metadata

COMMIT = 'a' * 40
TAG = 'v1.2.3'


class UpdateMetadata(unittest.TestCase):
    def prepare(self, cargo='1.2.3', module=True, digest=None):
        def fetch(args, **kwargs):
            self.assertEqual(args, ['nix', 'store', 'prefetch-file', '--unpack', '--json',
                                   f'https://codeload.github.com/lnbits/peasy/tar.gz/{COMMIT}'])
            return json.dumps({'storePath': '/nix/store/test-source', 'hash': digest or
                               'sha256-' + base64.b64encode(bytes(range(32))).decode()})
        with patch.object(Path, 'read_text', return_value=f'[workspace.package]\nversion = "{cargo}"'), \
                patch.object(Path, 'is_file', return_value=module):
            return update_metadata.prepare(TAG, COMMIT, run=fetch)

    def test_pin_matches_downloaded_source_and_uses_nar_hash(self):
        self.assertEqual(self.prepare(), {'format': 1, 'tag': TAG, 'version': '1.2.3',
                                         'revision': COMMIT, 'sha256': bytes(range(32)).hex()})

    def test_wrong_source_version_or_missing_module_blocks_publication(self):
        with self.assertRaisesRegex(ValueError, 'Cargo.toml'):
            self.prepare(cargo='1.2.2')
        with self.assertRaisesRegex(ValueError, 'self-updates'):
            self.prepare(module=False)
        with self.assertRaises(ValueError):
            self.prepare(digest='sha256-AA==')

    def test_untrusted_identifiers_rejected_before_fetch(self):
        for tag, commit in [('v1.2.3-rc.1', COMMIT), ('v01.2.3', COMMIT), ('v1/other', COMMIT),
                            (TAG, 'main'), (TAG, 'B' * 40)]:
            with self.subTest(tag=tag, commit=commit):
                with self.assertRaises(ValueError):
                    update_metadata.prepare(tag, commit, run=lambda *_args, **_kw: self.fail('unexpected fetch'))

    def test_altered_metadata_is_not_publishable(self):
        valid = self.prepare()
        for field, value in [('tag', 'v1.2.4'), ('version', '1.2.4'), ('revision', 'b' * 40),
                             ('sha256', 'z' * 64), ('format', True), ('url', 'https://elsewhere')]:
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    update_metadata.validate(valid | {field: value}, TAG, COMMIT)
