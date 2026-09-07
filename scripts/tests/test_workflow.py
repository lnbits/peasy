"""Keep the release gates and credential boundaries wired into GitHub Actions."""
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]


class ReleaseWorkflow(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # BaseLoader avoids YAML 1.1 treating GitHub's `on` key as a boolean.
        cls.workflow = yaml.load((ROOT / '.github/workflows/iso.yml').read_text(), Loader=yaml.BaseLoader)
        cls.jobs = cls.workflow['jobs']

    def commands(self, job):
        return '\n'.join(step.get('run', '') for step in self.jobs[job]['steps'])

    def test_checks_use_pinned_tools_before_builds(self):
        self.assertIn('.#checks.x86_64-linux.release', self.commands('release-tests'))
        self.assertNotIn('python3 -B -m unittest', self.commands('release-tests'))
        self.assertEqual(self.jobs['build']['needs'], 'release-tests')
        self.assertEqual(self.jobs['security']['needs'], 'release-tests')
        config_step = next(step for step in self.jobs['release-tests']['steps']
                           if 'r2_isos.configuration()' in step.get('run', ''))
        self.assertIn("github.ref_type == 'tag'", config_step['if'])
        self.assertEqual(set(config_step['env']), {'R2_BUCKET', 'R2_ENDPOINT_URL', 'R2_PUBLIC_URL'})
        publish = self.commands('publish')
        self.assertIn('nix build .#iso-release-tools', publish)
        self.assertIn('./result-release-tools/bin/python3 -B scripts/release_isos.py', publish)
        sdk_step = next(step for step in self.jobs['publish']['steps'] if 'test_r2_sdk.py' in step.get('run', ''))
        self.assertEqual(sdk_step['env']['PEASY_REQUIRE_R2_SDK'], '1')

    def test_both_desktops_and_security_gate_publication(self):
        self.assertEqual(set(self.jobs['build']['strategy']['matrix']['desktop']), {'gnome', 'plasma'})
        self.assertEqual(set(self.jobs['publish']['needs']), {'build', 'security'})
        self.assertIn("github.event_name == 'push' && github.ref_type == 'tag'", self.jobs['publish']['if'])
        self.assertEqual(self.workflow['on']['push']['tags'], ['v*'])
        for check in ['sandbox', 'sandbox-fixture', 'system-configuration', 'core-package']:
            self.assertIn(f'.#checks.x86_64-linux.{check}', self.commands('security').split())
        build = self.commands('build')
        for check in ['iso-config', 'installer-target', 'wasm-imports', 'desktop-config']:
            self.assertIn(f'.#checks.x86_64-linux.{check}', build.split())
        self.assertIn('".#checks.x86_64-linux.${DESKTOP}-tray"', build)
        self.assertIn('scripts/iso_vm.py --iso', build)
        self.assertIn('--firmware bios', build)
        self.assertIn('--firmware uefi', build)
        # Ignore explanatory comments when checking execution flags.
        active = '\n'.join(line for line in build.splitlines() if not line.lstrip().startswith('#'))
        self.assertNotIn('--reuse-base', active)
        self.assertNotIn('--online', active)

    def test_actions_pinned_and_credentials_limited_to_publishing(self):
        self.assertEqual(self.workflow['permissions'], {'contents': 'read'})
        for name, job in self.jobs.items():
            self.assertNotIn('secrets.', str(job.get('env', {})))
            self.assertNotIn('write-all', str(job.get('permissions', {})))
            for step in job['steps']:
                if 'uses' in step:
                    self.assertRegex(step['uses'], r'^[\w./-]+@[0-9a-f]{40}$')
                if step.get('uses', '').startswith('actions/checkout@'):
                    self.assertEqual(step['with']['ref'], '${{ github.sha }}')
                    self.assertEqual(step['with']['persist-credentials'], 'false')
                if 'secrets.' in str(step):
                    self.assertEqual(name, 'publish')
                    self.assertIn('scripts/release_isos.py', step.get('run', ''))
        self.assertEqual(self.jobs['publish']['permissions'], {'contents': 'write'})
        self.assertEqual(self.jobs['publish']['concurrency']['group'], 'peasy-iso-publication')
        self.assertEqual(self.jobs['publish']['concurrency']['cancel-in-progress'], 'false')
