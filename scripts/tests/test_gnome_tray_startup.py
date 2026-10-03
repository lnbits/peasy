"""Exercise the actual bounded autostart script without a desktop or VM."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'nix/gnome-tray-compatibility.sh'


class GnomeTrayStartupTests(unittest.TestCase):
    def run_helper(self, failures):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            helper = root / 'extensions'
            helper.write_text('''#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$CALLS"
if [[ "$1" == disable ]]; then exit 1; fi
n=0
if [[ -f "$COUNT" ]]; then read -r n < "$COUNT"; fi
n=$((n + 1))
printf '%s\\n' "$n" > "$COUNT"
(( n > FAILURES ))
''')
            helper.chmod(0o755)
            env = dict(os.environ, extensions=str(helper), extension_uuid='test-tray',
                       timeout=shutil.which('timeout'), sleep=shutil.which('true'),
                       CALLS=str(root/'calls'), COUNT=str(root/'count'), FAILURES=str(failures))
            result = subprocess.run(['bash', str(SCRIPT)], env=env, capture_output=True,
                                    text=True, timeout=5)
            return result, (root/'calls').read_text().splitlines()

    def test_late_shell_recovers_without_repeated_legacy_disable(self):
        result, calls = self.run_helper(2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['disable peasy@peasy-nixos.github.io'] + ['enable test-tray'] * 3)

    def test_permanent_failure_is_bounded_and_reported(self):
        result, calls = self.run_helper(100)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 16)
        self.assertIn('GNOME tray support could not be enabled', result.stderr)
