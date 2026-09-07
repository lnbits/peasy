"""Boot a real Peasy ISO, measure installation, and test disposable disk overlays.

No host disk or host Nix store is exposed. Offline is the default. Reusable
bases are a developer convenience, never accepted when CI=true.
"""
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import socket
import subprocess
import tempfile
import time
import uuid

HERE = Path(__file__).resolve().parent


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def cache_key(identity):
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def private_directory(path):
    path = Path(path).absolute()
    if path != path.resolve():
        raise ValueError('Cache/output directory must not contain symlinks')
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.stat()
    if info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError(f'Directory must be private and owned by this user: {path}')
    return path


def valid_base(entry, identity):
    base, metadata, firmware = entry / 'base.qcow2', entry / 'base.json', entry / 'vars.fd'
    if any(p.is_symlink() for p in (base, metadata, firmware)):
        raise ValueError('Refusing symlinked cache files')
    if not metadata.is_file() or not base.is_file():
        return False
    saved = json.loads(metadata.read_text())
    if saved.get('identity') != identity or saved.get('disk_sha256') != digest(base):
        return False
    return identity['firmware'] != 'uefi' or (
        firmware.is_file() and saved.get('vars_sha256') == digest(firmware))


class VM:
    def __init__(self, args, work, disk, firmware, log_name, iso=None):
        self.args, self.work = args, work
        self.process = self.serial = self.qmp_socket = self.qmp = None
        self.log = (args.output / log_name).open('wb')
        # Unix sockets have a short path limit; cache identities are long.
        self.sockets = tempfile.TemporaryDirectory(prefix='peasy-vm-sock-')
        sockets = Path(self.sockets.name)
        self.qmp_path, self.serial_path = sockets / 'qmp', sockets / 'serial'
        command = [args.qemu, '-enable-kvm', '-cpu', 'host', '-smp', str(args.cpus),
                   '-m', str(args.memory), '-display', 'none', '-device', 'virtio-vga',
                   '-drive', f'file={disk},format=qcow2,if=none,id=disk',
                   '-device', 'virtio-blk-pci,drive=disk,serial=PEASY_TEST_DISK',
                   '-qmp', f'unix:{self.qmp_path},server=on,wait=off',
                   '-serial', f'unix:{self.serial_path},server=on,wait=off']
        command += ['-nic', 'user,model=virtio-net-pci'] if args.online else ['-nic', 'none']
        if args.firmware == 'uefi':
            command += ['-machine', 'q35',
                        '-drive', f'if=pflash,format=raw,readonly=on,file={args.ovmf_code}',
                        '-drive', f'if=pflash,format=raw,file={firmware}']
        if iso:
            command += ['-cdrom', str(iso), '-boot', 'once=d']
        try:
            self.process = subprocess.Popen(command, stdout=self.log, stderr=self.log)
            self.serial = self.connect(self.serial_path)
            self.serial.setblocking(False)
            self.qmp_socket = self.connect(self.qmp_path)
            self.qmp = self.qmp_socket.makefile('rwb', buffering=0)
            self.qmp.readline()
            self.control('qmp_capabilities')
        except BaseException:
            self.stop()
            raise
        self.buffer = b''

    def connect(self, path):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                connection = socket.socket(socket.AF_UNIX)
                connection.connect(str(path))
                connection.settimeout(30)
                return connection
            except OSError:
                connection.close()
                if self.process.poll() is not None:
                    raise RuntimeError('QEMU exited; inspect the VM log')
                time.sleep(.1)
        raise TimeoutError(f'QEMU socket unavailable: {path}')

    def control(self, command, **arguments):
        self.qmp.write(json.dumps({'execute': command, 'arguments': arguments}).encode() + b'\n')
        while True:
            result = json.loads(self.qmp.readline())
            if 'error' in result:
                raise RuntimeError(result['error'])
            if 'return' in result:
                return result['return']

    def key(self, keys):
        self.control('human-monitor-command', **{'command-line': f'sendkey {keys} 10'})
        time.sleep(.03)

    def type(self, text):
        punctuation = {' ': 'spc', '-': 'minus', '.': 'dot', '/': 'slash',
                       '@': 'shift-2', ':': 'shift-semicolon', '_': 'shift-minus', '\n': 'ret'}
        for char in text:
            self.key(punctuation.get(char, 'shift-' + char.lower() if char.isupper() else char))

    def read(self, timeout=1):
        if select.select([self.serial], [], [], timeout)[0]:
            data = self.serial.recv(65536)
            if not data:
                raise RuntimeError('VM serial connection closed')
            self.log.write(data)
            self.log.flush()
            self.buffer = (self.buffer + data)[-131072:]
        if self.process.poll() is not None:
            raise RuntimeError('QEMU exited unexpectedly')

    def login(self, installed=False):
        deadline, last_attempt = time.monotonic() + 240, time.monotonic() + 30
        selected_serial = False
        while time.monotonic() < deadline:
            self.read()
            if b'No bootable option or device was found' in self.buffer:
                raise RuntimeError('UEFI could not boot the supplied image; inspect the boot log')
            if not installed and not selected_serial and b'ISOLINUX' in self.buffer:
                self.serial.sendall(b'\x1b')
                self.buffer = b''
                boot_deadline = time.monotonic() + 10
                while b'boot:' not in self.buffer and time.monotonic() < boot_deadline:
                    self.read()
                self.serial.sendall(b'boot-serial\r')
                selected_serial = True
            elif installed and b'login:' in self.buffer:
                self.serial.sendall(b'root\n')
                self.buffer = b''
            elif installed and b'Password:' in self.buffer:
                self.serial.sendall(b'peasy-vm-test\n')
                self.buffer = b''
            elif re.search(rb'(?:\$|#)\s*(?:\x1b\[[0-9;]*[a-zA-Z])*\s*$', self.buffer):
                self.serial.sendall(b'sudo -i\n' if not installed else b'\n')
                time.sleep(1)
                self.serial.sendall(b'stty -echo; PS1=""; export PS1; bind "set enable-bracketed-paste off"\n')
                time.sleep(.2)
                self.read(.2)
                self.run('test "$(id -u)" -eq 0')
                return
            elif not installed and time.monotonic() - last_attempt > 20:
                # Use the normal live user's autologin console. This also works
                # with the ISO's unmodified BIOS and UEFI boot menus.
                # GNOME can occupy tty2; tty3 is a normal live autologin getty.
                self.key('ctrl-alt-f3')
                time.sleep(1)
                self.type('sudo systemctl start serial-getty@ttyS0.service\n')
                self.control('screendump', filename=str(self.args.output / 'live-console.ppm'))
                last_attempt = time.monotonic()
        raise TimeoutError('No guest console; inspect boot log/screenshot')

    def run(self, command, timeout=120):
        marker = '__PEASY_' + uuid.uuid4().hex + '__'
        encoded = base64.b64encode(command.encode()).decode()
        self.buffer = b''
        self.serial.sendall((f"printf %s {shlex.quote(encoded)} | base64 -d | bash -eo pipefail; "
                             f"printf '\\n{marker}%s\\n' \"$?\"\n").encode())
        deadline, heartbeat = time.monotonic() + timeout, time.monotonic()
        pattern = re.compile(rb'\r?\n' + marker.encode() + rb'(\d+)\r?\n')
        while time.monotonic() < deadline:
            self.read()
            found = pattern.search(self.buffer)
            if found:
                output = self.buffer[:found.start()].decode(errors='replace')
                output = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', output).replace('\r', '')
                if int(found[1]):
                    raise RuntimeError(f'Guest command failed ({found[1].decode()}): {output[-12000:]}')
                return output
            if time.monotonic() - heartbeat > 30:
                print('  VM operation still running; details in ' + self.log.name, flush=True)
                heartbeat = time.monotonic()
        raise TimeoutError(f'Guest command timed out: {command[:160]}')

    def send_file(self, source, destination):
        data = base64.b64encode(Path(source).read_bytes()).decode()
        # Keep individual serial lines below canonical terminal input limits.
        self.run(f': > {shlex.quote(destination)}.b64')
        for offset in range(0, len(data), 2000):
            self.run(f'printf %s {shlex.quote(data[offset:offset + 2000])} >> {shlex.quote(destination)}.b64')
        self.run(f'base64 -d {shlex.quote(destination)}.b64 > {shlex.quote(destination)}')

    def wait_for_shutdown(self, timeout=120):
        # QEMU's serial socket can apply backpressure. Keep consuming the
        # console while systemd shuts down, just as we do while commands run.
        deadline = time.monotonic() + timeout
        while self.process.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired('QEMU shutdown', timeout)
            try:
                self.process.wait(timeout=min(1, remaining))
            except subprocess.TimeoutExpired:
                if select.select([self.serial], [], [], 0)[0]:
                    data = self.serial.recv(65536)
                    self.log.write(data)
                    self.log.flush()

    def stop(self, graceful=False):
        clean = True
        try:
            if graceful and self.process and self.process.poll() is None:
                try:
                    self.serial.sendall(b'systemctl poweroff\n')
                    self.wait_for_shutdown()
                except (OSError, subprocess.TimeoutExpired):
                    clean = False
            if self.process and self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=20)
        finally:
            for resource in (self.serial, self.qmp, self.qmp_socket, self.log):
                if resource:
                    resource.close()
            self.sockets.cleanup()
        if not clean:
            raise RuntimeError('VM did not shut down cleanly; base must not be cached')


def installed_checks(vm, desktop):
    vm.run('systemctl is-active peasy-system.service; test -x /run/current-system/sw/bin/peasy-ui')
    vm.run('for i in $(seq 1 90); do pgrep -u peasytest -x peasy-tray && exit 0; sleep 1; done; exit 1')
    vm.run('pkaction --action-id io.github.peasy.apply --verbose | grep auth_admin')
    vm.run("if su - peasytest -c 'pkcheck --action-id io.github.peasy.apply --process $$'; then exit 1; fi")
    vm.run('test -f /etc/peasy/wallpaper.png; test -f /etc/nixos/peasy.nix; ! id nixos')
    vm.run('test ! -e /etc/peasy/ISO-README.txt; test ! -e /home/peasytest/.config/peasy/openai-api-key')
    vm.control('screendump', filename=str(vm.args.output / f'{desktop}-installed.ppm'))


def benchmark(args):
    benchmark_started = time.monotonic()
    if args.reuse_base and os.environ.get('CI', '').lower() == 'true':
        raise ValueError('CI must test fresh installations, not reuse development bases')
    args.iso = args.iso.resolve(strict=True)
    if not args.iso.is_file() or ',' in str(args.iso):
        raise ValueError('Expected a regular ISO file with no comma in its path')
    args.output = private_directory(args.output)
    if any(args.output.iterdir()):
        raise ValueError('Choose a new output directory; previous results are never overwritten')
    cache = private_directory(args.cache_dir)
    if any(',' in str(p) for p in (cache, args.output, args.ovmf_code, args.ovmf_vars)):
        raise ValueError('QEMU file paths must not contain commas')
    identity = {'iso_sha256': digest(args.iso), 'desktop': args.desktop, 'firmware': args.firmware,
                'memory_mib': args.memory, 'cpus': args.cpus, 'online': args.online,
                'qemu': subprocess.check_output([args.qemu, '--version'], text=True).splitlines()[0],
                'harness': digest(__file__), 'guest': digest(HERE / 'iso_vm_guest.py'),
                'package_test': digest(HERE.parent / 'nix/tests/installer-package.py')}
    if args.firmware == 'uefi':
        identity.update(ovmf_code=digest(args.ovmf_code), ovmf_vars=digest(args.ovmf_vars))
    entry = private_directory(cache / cache_key(identity))
    result = {'identity': identity, 'iso_bytes': args.iso.stat().st_size, 'reused_base': False,
              'passed': False, 'timings_seconds': {}}
    vm = None
    lock_fd = os.open(entry / 'lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock_fd, 'w') as lock, tempfile.TemporaryDirectory(prefix='run-', dir=entry) as temporary:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(temporary)
        base, metadata = entry / 'base.qcow2', entry / 'base.json'
        firmware = work / 'vars.fd'
        try:
            reused = False
            if any(p.is_symlink() for p in (base, metadata, entry / 'vars.fd')):
                raise ValueError('Refusing symlinked cache files')
            if args.reuse_base:
                reused = valid_base(entry, identity)
            if not reused:
                disk = work / 'install.qcow2'
                subprocess.run([args.qemu_img, 'create', '-f', 'qcow2', str(disk), '32G'], check=True)
                if args.firmware == 'uefi':
                    shutil.copyfile(args.ovmf_vars, firmware)
                print('Booting actual ISO (fresh install, no host store exposed)', flush=True)
                start = time.monotonic()
                vm = VM(args, work, disk, firmware, 'installer.log', args.iso)
                vm.login()
                result['timings_seconds']['live_boot_console'] = time.monotonic() - start
                vm.send_file(HERE / 'iso_vm_guest.py', '/tmp/peasy-iso-vm-guest.py')
                python = vm.run('command -v python3 || printf "%s\\n" /nix/store/*-python3-*/bin/python3 | head -1').strip()
                print('Running shipped Calamares installation job', flush=True)
                start = time.monotonic()
                try:
                    vm.run(f'{shlex.quote(python)} /tmp/peasy-iso-vm-guest.py {args.desktop} {args.firmware}', timeout=1800)
                finally:
                    result['timings_seconds']['install_attempt'] = time.monotonic() - start
                result['timings_seconds']['install'] = result['timings_seconds']['install_attempt']
                vm.run('umount -R /mnt; sync')
                vm.stop(graceful=True)
                vm = None
                start = time.monotonic()
                vm = VM(args, work, disk, firmware, 'installed-boot.log')
                vm.login(installed=True)
                installed_checks(vm, args.desktop)
                result['timings_seconds']['installed_boot_checks'] = time.monotonic() - start
                vm.stop(graceful=True)
                vm = None
                os.replace(disk, base)
                saved = {'identity': identity, 'disk_sha256': digest(base)}
                if args.firmware == 'uefi':
                    shutil.copyfile(firmware, entry / 'vars.fd')
                    saved['vars_sha256'] = digest(entry / 'vars.fd')
                pending = work / 'base.json'
                pending.write_text(json.dumps(saved, indent=2) + '\n')
                os.replace(pending, metadata)
            else:
                result['reused_base'] = True
                print('Reusing verified development base; this is NOT a fresh-install result', flush=True)
            overlay = work / 'test.qcow2'
            subprocess.run([args.qemu_img, 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', str(base), str(overlay)], check=True)
            if args.firmware == 'uefi':
                shutil.copyfile(entry / 'vars.fd', firmware)
            start = time.monotonic()
            vm = VM(args, work, overlay, firmware, 'runtime.log')
            vm.login(installed=True)
            installed_checks(vm, args.desktop)
            vm.send_file(HERE.parent / 'nix/tests/installer-package.py', '/tmp/peasy-package.py')
            python = vm.run('command -v python3 || printf "%s\\n" /nix/store/*-python3-*/bin/python3 | head -1').strip()
            # patchelf is retained by normal configuration builders, but is not
            # a default global command. Its complete package (including manual)
            # is cached, unlike jq's executable-only transitive dependency.
            # Do not inject a test-only cache or enable networking.
            vm.run('test ! -e /run/current-system/sw/bin/patchelf')
            for operation in ('install', 'remove'):
                vm.run(f'{shlex.quote(python)} /tmp/peasy-package.py {operation} patchelf', timeout=900)
                if operation == 'install':
                    vm.run('/run/current-system/sw/bin/patchelf --version')
                    vm.run('nixos-rebuild build --no-flake && test -x result/sw/bin/patchelf', timeout=900)
            vm.run('test ! -e /run/current-system/sw/bin/patchelf')
            result['timings_seconds']['overlay_runtime_checks'] = time.monotonic() - start
            result['passed'] = True
        except Exception as error:
            result['error'] = str(error)
            if vm:
                try:
                    vm.control('screendump', filename=str(args.output / 'failure.ppm'))
                except Exception:
                    pass  # Preserve the original failure even if QEMU has exited.
                diagnostics = {
                    'guest-journal.txt':
                        'systemctl --failed --no-pager || true; '
                        'journalctl -b -u peasy-system -u peasy-activate --no-pager -n 80 || true',
                }
                for name in ('configuration.nix', 'hardware-configuration.nix'):
                    diagnostics[f'guest-{name}.txt'] = (
                        f'if test -f /mnt/etc/nixos/{name}; then cat /mnt/etc/nixos/{name}; '
                        f'elif test -f /etc/nixos/{name}; then cat /etc/nixos/{name}; fi')
                for name, command in diagnostics.items():
                    try:
                        (args.output / name).write_text(vm.run(command, timeout=20))
                    except Exception:
                        pass
            raise
        finally:
            try:
                if vm:
                    vm.stop()
            except Exception as error:
                result['passed'] = False
                result['cleanup_error'] = str(error)
                raise
            finally:
                result['timings_seconds']['total'] = time.monotonic() - benchmark_started
                (args.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
                print(json.dumps(result, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--iso', type=Path, required=True)
    parser.add_argument('--desktop', choices=['gnome', 'plasma'], required=True)
    parser.add_argument('--firmware', choices=['bios', 'uefi'], default='bios')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cache-dir', type=Path, default=Path.home() / '.cache/peasy/iso-vm')
    parser.add_argument('--reuse-base', action='store_true')
    parser.add_argument('--online', action='store_true', help='Allow downloads; default is physically disconnected networking')
    parser.add_argument('--memory', type=int, default=8192)
    parser.add_argument('--cpus', type=int, default=4)
    parser.add_argument('--qemu', default='qemu-system-x86_64')
    parser.add_argument('--qemu-img', default='qemu-img')
    parser.add_argument('--ovmf-code', type=Path)
    parser.add_argument('--ovmf-vars', type=Path)
    args = parser.parse_args()
    if args.firmware == 'uefi' and not (args.ovmf_code and args.ovmf_vars):
        parser.error('UEFI requires --ovmf-code and --ovmf-vars')
    if args.cpus < 1 or args.memory < 4096:
        parser.error('Use at least one CPU and 4096 MiB RAM')
    benchmark(args)


if __name__ == '__main__':
    main()
