import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import iso_vm


class VMTests(unittest.TestCase):
    def test_installed_checks_reject_real_provider_and_key_paths(self):
        vm = Mock()
        vm.args.output = Path('/test-output')
        iso_vm.installed_checks(vm, 'plasma')
        commands = '\n'.join(call.args[0] for call in vm.run.call_args_list)
        self.assertIn('test ! -e /home/peasytest/.config/peasy/openai-key;', commands)
        self.assertIn('test ! -e /home/peasytest/.config/peasy/provider.json', commands)
        self.assertNotIn('openai-api-key', commands)

    def test_every_relevant_input_invalidates_base(self):
        identity = dict(iso_sha256='a', firmware='bios', desktop='gnome', memory_mib=8192,
                        cpus=4, online=False, qemu='11.1', harness='b', guest='c',
                        package_test='d', ovmf_code='e', ovmf_vars='f')
        original = iso_vm.cache_key(identity)
        for key in identity:
            with self.subTest(key=key):
                self.assertNotEqual(original, iso_vm.cache_key(identity | {key: 'changed'}))
        self.assertEqual(original, iso_vm.cache_key(dict(reversed(list(identity.items())))))

    def test_cache_must_be_private_and_not_symlinked(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = iso_vm.private_directory(root / 'cache')
            self.assertEqual(cache.stat().st_mode & 0o777, 0o700)
            cache.chmod(0o755)
            with self.assertRaises(ValueError):
                iso_vm.private_directory(cache)
            (root / 'link').symlink_to(cache, target_is_directory=True)
            with self.assertRaises(ValueError):
                iso_vm.private_directory(root / 'link')

    def test_ci_refuses_reused_bases_before_any_vm_or_file_work(self):
        with patch.dict(os.environ, {'CI': 'true'}):
            with self.assertRaisesRegex(ValueError, 'fresh installations'):
                iso_vm.benchmark(types.SimpleNamespace(reuse_base=True))

    def test_digest_checks_contents_not_filename(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'base.qcow2'
            path.write_bytes(b'one')
            first = iso_vm.digest(path)
            path.write_bytes(b'two')
            self.assertNotEqual(first, iso_vm.digest(path))

    def test_reuse_requires_matching_disk_identity_and_firmware(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = {'firmware': 'uefi', 'iso_sha256': 'original'}
            self.assertFalse(iso_vm.valid_base(root, identity))
            disk, firmware = root / 'base.qcow2', root / 'vars.fd'
            disk.write_bytes(b'installed disk')
            firmware.write_bytes(b'firmware')
            (root / 'base.json').write_text(json.dumps({
                'identity': identity, 'disk_sha256': iso_vm.digest(disk),
                'vars_sha256': iso_vm.digest(firmware)}))
            self.assertTrue(iso_vm.valid_base(root, identity))
            self.assertFalse(iso_vm.valid_base(root, identity | {'iso_sha256': 'new'}))
            disk.write_bytes(b'corrupted disk')
            self.assertFalse(iso_vm.valid_base(root, identity))
            disk.write_bytes(b'installed disk')
            firmware.write_bytes(b'changed firmware')
            self.assertFalse(iso_vm.valid_base(root, identity))
            firmware.unlink()
            self.assertFalse(iso_vm.valid_base(root, identity))
            firmware.symlink_to(disk)
            with self.assertRaisesRegex(ValueError, 'symlinked'):
                iso_vm.valid_base(root, identity)

    def test_guest_commands_fail_closed_and_strip_terminal_codes(self):
        vm = object.__new__(iso_vm.VM)
        sent = []
        vm.serial = types.SimpleNamespace(sendall=sent.append)
        nonce = types.SimpleNamespace(hex='test')

        def read():
            vm.buffer = b'\x1b[?2004h/result\r\n__PEASY_test__0\r\n'

        vm.read = read
        with patch.object(iso_vm.uuid, 'uuid4', return_value=nonce):
            self.assertEqual(vm.run('printf /result'), '/result')
        self.assertIn(b'bash -eo pipefail', sent[0])

        def failed_read():
            vm.buffer = b'failure\r\n__PEASY_test__4\r\n'

        vm.read = failed_read
        with patch.object(iso_vm.uuid, 'uuid4', return_value=nonce):
            with self.assertRaisesRegex(RuntimeError, 'Guest command failed'):
                vm.run('false')

    def test_offline_vm_has_no_network_or_host_store_mount(self):
        source = Path(iso_vm.__file__).read_text()
        self.assertIn("['-nic', 'none']", source)
        self.assertNotIn("'-virtfs'", source)
        self.assertNotIn("'-fsdev'", source)

    def test_constructor_failure_stops_qemu(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = types.SimpleNamespace(output=root, qemu='qemu', cpus=2,
                                         memory=4096, online=False, firmware='bios')
            process = Mock()
            process.poll.return_value = None
            with patch.object(iso_vm.subprocess, 'Popen', return_value=process), \
                    patch.object(iso_vm.VM, 'connect', side_effect=TimeoutError('test')):
                with self.assertRaisesRegex(TimeoutError, 'test'):
                    iso_vm.VM(args, root, root / 'disk', None, 'vm.log')
            process.terminate.assert_called_once()
            process.wait.assert_called_once()

    def test_unclean_shutdown_kills_vm_and_refuses_caching(self):
        vm = object.__new__(iso_vm.VM)
        vm.process, vm.serial, vm.qmp, vm.qmp_socket, vm.log, vm.sockets = [Mock() for _ in range(6)]
        vm.process.poll.return_value = None
        vm.wait_for_shutdown = Mock(side_effect=iso_vm.subprocess.TimeoutExpired('qemu', 120))
        vm.process.wait.side_effect = [iso_vm.subprocess.TimeoutExpired('qemu', 20), 0]
        with self.assertRaisesRegex(RuntimeError, 'base must not be cached'):
            vm.stop(graceful=True)
        vm.process.terminate.assert_called_once()
        vm.process.kill.assert_called_once()
        vm.serial.close.assert_called_once()
        vm.sockets.cleanup.assert_called_once()

    def test_shutdown_drains_serial_output(self):
        vm = object.__new__(iso_vm.VM)
        vm.process, vm.serial, vm.log = Mock(), Mock(), Mock()
        vm.process.poll.side_effect = [None, 0]
        vm.process.wait.side_effect = iso_vm.subprocess.TimeoutExpired('qemu', 1)
        vm.serial.recv.return_value = b'shutdown console output'
        with patch.object(iso_vm.select, 'select', return_value=([vm.serial], [], [])):
            vm.wait_for_shutdown()
        vm.log.write.assert_called_once_with(b'shutdown console output')


if __name__ == '__main__':
    unittest.main()
