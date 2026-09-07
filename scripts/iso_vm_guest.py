"""Test driver sent to a disposable VM, never bundled into the release ISO.

Uses the ISO's real Calamares NixOS job. Only wizard choices and progress
callbacks are supplied; no host Nix store, packages, or installer code is used.
"""
import glob
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import types


def command(*args):
    result = subprocess.run(args, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f'{args}: {result.stderr}')
    return result.stdout


def install(desktop, firmware):
    serials = [Path('/sys/block/vda/serial'), Path('/sys/block/vda/device/serial')]
    if not any(p.exists() and p.read_text().strip() == 'PEASY_TEST_DISK' for p in serials):
        raise RuntimeError('Refusing installation: not the explicitly labelled disposable VM disk')
    if '/dev/vda' in command('findmnt', '-n', '-o', 'SOURCE', '/'):
        raise RuntimeError('Refusing to partition the running root disk')
    command('parted', '--script', '/dev/vda', 'mklabel', 'gpt')
    if firmware == 'uefi':
        command('parted', '--script', '/dev/vda', 'mkpart', 'ESP', 'fat32', '1MiB', '257MiB',
                'set', '1', 'esp', 'on', 'mkpart', 'root', '257MiB', '100%')
        command('udevadm', 'settle')
        command('mkfs.vfat', '-n', 'ESP', '/dev/vda1')
    else:
        command('parted', '--script', '/dev/vda', 'mkpart', 'bios', '1MiB', '3MiB',
                'set', '1', 'bios_grub', 'on', 'mkpart', 'root', '3MiB', '100%')
    command('udevadm', 'settle')
    command('mkfs.ext4', '-L', 'nixos', '/dev/vda2')
    command('udevadm', 'settle')
    command('mount', '-t', 'ext4', '/dev/vda2', '/mnt')
    Path('/mnt/etc/nixos').mkdir(parents=True)
    Path('/mnt/boot').mkdir()
    if firmware == 'uefi':
        command('mount', '-t', 'vfat', '/dev/vda1', '/mnt/boot')

    candidates = [p for p in glob.glob('/nix/store/*-calamares-nixos-extensions-*/lib/calamares/modules/nixos/main.py')
                  if './peasy.nix' in Path(p).read_text()]
    if len(candidates) != 1:
        raise RuntimeError(f'Expected exactly one shipped Peasy installer, found {candidates}')
    values = {
        'rootMountPoint': '/mnt', 'firmwareType': 'efi' if firmware == 'uefi' else 'bios',
        'bootLoader': {'installPath': '/dev/vda'}, 'partitions': [],
        'packagechooser_packagechooser': 'plasma6' if desktop == 'plasma' else 'gnome',
        'username': 'peasytest', 'fullname': 'Peasy VM Test', 'hostname': 'peasy-vm-test',
        'autoLoginUser': 'peasytest',
    }

    def host_process(argv, unused=None, contents=None):
        # Serial console is test transport, not a source of cached dependencies.
        if contents and './hardware-configuration.nix' in contents:
            contents = contents.replace('./hardware-configuration.nix',
                                        './hardware-configuration.nix ./vm-console.nix')
        subprocess.run(argv, input=contents, text=True, check=True)

    Path('/mnt/etc/nixos/vm-console.nix').write_text('''{ ... }: {
      boot.kernelParams = [ "console=ttyS0,115200n8" ];
      users.users.root.initialPassword = "peasy-vm-test";
      users.users.peasytest.initialPassword = "peasy-vm-test";
    }\n''')
    module = types.ModuleType('libcalamares')
    module.globalstorage = types.SimpleNamespace(value=values.get)
    module.job = types.SimpleNamespace(setprogress=lambda value: None)
    module.utils = types.SimpleNamespace(
        gettext_path=lambda: '/nonexistent', gettext_languages=lambda: ['en'],
        warning=print, error=print, debug=lambda text: print(text, flush=True),
        host_env_process_output=host_process,
    )
    sys.modules['libcalamares'] = module
    spec = importlib.util.spec_from_file_location('shipped_installer', candidates[0])
    job = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(job)
    result = job.run()
    if result is not None:
        raise RuntimeError(result)
    print('PEASY_INSTALL_COMPLETE', flush=True)


if __name__ == '__main__':
    install(*sys.argv[1:])
