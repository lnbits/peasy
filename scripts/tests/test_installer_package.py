import contextlib
import errno
import importlib.util
import io
import json
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    'installer_package', Path(__file__).resolve().parents[2] / 'nix/tests/installer-package.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class InstallerPackageTests(unittest.TestCase):
    @contextlib.contextmanager
    def daemon(self, replies):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'ipc')
            requests, errors = [], []
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(path)
                listener.listen()
                listener.settimeout(3)

                def serve():
                    try:
                        for reply in replies:
                            connection, _ = listener.accept()
                            with connection:
                                connection.settimeout(3)
                                with connection.makefile('rb') as reader:
                                    requests.append(json.loads(reader.readline()))
                                if reply is not None:
                                    connection.sendall(json.dumps(reply).encode() + b'\n')
                    except Exception as error:
                        errors.append(error)

                worker = threading.Thread(target=serve, daemon=True)
                worker.start()
                try:
                    yield path, requests
                finally:
                    worker.join(timeout=5)
                    self.assertFalse(worker.is_alive(), 'test daemon did not finish')
                    self.assertEqual(errors, [])

    def test_install_and_remove_verify_state_across_restart(self):
        replies = []
        for installed in (True, False):
            replies.extend([
                {'response': 'proposal', 'proposal': {'id': 'reviewed', 'title': 'Patchelf'}},
                {'response': 'applied', 'result': {'activated': True, 'message': 'Done'}},
                None,  # Apply succeeded, then the next connection was lost.
                {'response': 'error', 'message': package.RESTARTING_MESSAGE},
                {'response': 'packages', 'packages': ['patchelf'] if installed else []},
            ])
        original = package.request
        with self.daemon(replies) as (path, requests), \
                patch.object(package, 'request', side_effect=lambda message: original(message, path)), \
                contextlib.redirect_stdout(io.StringIO()):
            for operation in ('install', 'remove'):
                with patch.object(package.sys, 'argv', ['installer-package.py', operation, 'patchelf']):
                    package.main()
            self.assertEqual([r['request'] for r in requests], [
                'propose_install', 'apply', 'get_packages', 'get_packages', 'get_packages',
                'propose_remove', 'apply', 'get_packages', 'get_packages', 'get_packages'])

    def test_mutations_are_not_replayed_after_transport_loss_or_updating(self):
        for operation in ('propose_install', 'propose_remove', 'apply'):
            for error in (ConnectionResetError(errno.ECONNRESET, 'lost'),
                          ConnectionAbortedError(errno.ECONNABORTED, package.RESTARTING_MESSAGE)):
                with self.subTest(operation=operation, error=error), \
                        patch.object(package, 'request_once', side_effect=error) as attempt:
                    with self.assertRaises(type(error)):
                        package.request({'request': operation})
                    attempt.assert_called_once()

    def test_absent_or_refused_socket_has_a_bounded_wait(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'ipc')
            for stale in (False, True):
                if stale:
                    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                        listener.bind(path)
                started = time.monotonic()
                with self.assertRaisesRegex(TimeoutError, 'did not reconnect'):
                    package.request({'request': 'get_packages'}, path, restart_timeout=0.15)
                self.assertLess(time.monotonic() - started, 2)

    def test_read_failures_do_not_hide_application_or_protocol_errors(self):
        for error in (RuntimeError('permission denied'),
                      json.JSONDecodeError('invalid response', 'x', 0),
                      PermissionError(errno.EACCES, 'socket permission denied')):
            with self.subTest(error=error), \
                    patch.object(package, 'request_once', side_effect=error) as attempt:
                with self.assertRaises(type(error)):
                    package.request({'request': 'get_packages'})
                attempt.assert_called_once()


if __name__ == '__main__':
    unittest.main()
