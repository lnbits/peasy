"""Exercise a running GNOME Shell or XFCE App Finder, not a fresh query.

The disposable VM uses shallow copies of its system generation to separate
profile selection from activation deterministically, including a slow switch.
No host profiles are exposed or modified. No app executable is launched.
"""

import ast
import shlex
import time


def shell_eval(expression):
    command = (
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell "
        "-m org.gnome.Shell.Eval " + shlex.quote(expression)
    )
    response = machine.succeed("su - alice -c " + shlex.quote(command)).strip()
    success, encoded = ast.literal_eval(
        response.replace("(true,", "(True,", 1).replace("(false,", "(False,", 1)
    )
    assert success, response
    return json.loads(encoded)


def shell_apps():
    if desktop_under_test == "xfce":
        # Inspect the real, already-open App Finder. Search for "fixture", not
        # the display name, so the entry field cannot fake a positive result.
        text = machine.get_screen_text()
        assert "Application Finder" in text, text
        return ["peasy-search-fixture.desktop"] * text.count("Peasy Search Fixture")
    apps = shell_eval(
        "imports.gi.Shell.AppSystem.get_default().get_installed()"
        ".filter(a => a.get_id().includes('peasy-search-fixture'))"
        ".map(a => a.get_id())"
    )
    assert isinstance(apps, list), repr(apps)
    return apps


def wait_apps(expected):
    deadline = time.monotonic() + 30
    actual = None
    while time.monotonic() < deadline:
        actual = shell_apps()
        if actual == expected:
            return
        time.sleep(1)
    for command in [
        "systemctl status peasy-applications-refresh.service peasy-applications-refresh.path --no-pager",
        "journalctl -b -u peasy-applications-refresh.service --no-pager -n 30",
        "namei -l /run/current-system/sw/bin/peasy-search-fixture",
        "su - alice -c 'test -x /run/current-system/sw/bin/peasy-search-fixture && cat /run/current-system/sw/share/applications/peasy-search-fixture.desktop'",
    ]:
        print(command, machine.execute(command))
    if desktop_under_test == "gnome":
        print("Shell PATH", shell_eval("imports.gi.GLib.getenv('PATH')"))
        print(
            "GIO applications",
            shell_eval(
                "imports.gi.Gio.AppInfo.get_all().filter(a => a.get_id().includes('peasy-search-fixture')).map(a => a.get_id())"
            ),
        )
    machine.screenshot("application-search-failure")
    raise AssertionError(
        f"{desktop_under_test} cache: expected {expected!r}, got {actual!r}"
    )


fixture = r"""
from pathlib import Path
import json

original = Path('/run/current-system').resolve()
base = Path('/run/peasy-launcher-test')
base.mkdir()
(base / 'original').symlink_to(original)
generation = base / 'with-app'

def shallow_copy(source, target):
    target.mkdir()
    for child in source.iterdir():
        (target / child.name).symlink_to(child)

shallow_copy(original, generation)
for relative in ['sw', 'sw/bin', 'sw/share', 'sw/share/applications']:
    target = generation / relative
    target.unlink()
    shallow_copy(original / relative, target)
executable = generation / 'sw/bin/peasy-search-fixture'
executable.write_text('#!/bin/sh\nexit 0\n')
executable.chmod(0o755)
desktop = generation / 'sw/share/applications/peasy-search-fixture.desktop'
desktop.write_text('[Desktop Entry]\nType=Application\nName=Peasy Search Fixture\n'
                   'Exec=peasy-search-fixture\nTryExec=peasy-search-fixture\nCategories=Utility;\n')
profile = Path('/nix/var/nix/profiles/system')
(base / 'previous-profile.json').write_text(json.dumps(
    str(profile.readlink()) if profile.is_symlink() else None))
"""
machine.succeed("python3 -c " + shlex.quote(fixture))
base = "/run/peasy-launcher-test"
expected = ["peasy-search-fixture.desktop"]
if desktop_under_test == "gnome":
    shell_pid_command = user(
        "gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus "
        "--method org.freedesktop.DBus.GetConnectionUnixProcessID org.gnome.Shell"
    )
else:
    machine.succeed(
        "su - alice -c "
        + shlex.quote(
            "XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
            "systemd-run --user --collect --no-block xfce4-appfinder"
        )
    )
    machine.wait_for_text("Application Finder", timeout=30)
    machine.send_chars("fixture")
    shell_pid_command = (
        "pgrep -u alice -f '(^|/)([.]xfce4-appfinder-wrapped|xfce4-appfinder)( |$)'"
    )
    panel_pid_command = (
        "pgrep -u alice -f '(^|/)([.]xfce4-panel-wrapped|xfce4-panel)( |$)'"
    )
    panel_pid = machine.succeed(panel_pid_command).strip()
    assert panel_pid.isdigit(), panel_pid
shell_pid = machine.succeed(shell_pid_command).strip()
assert shell_pid, shell_pid


def select_profile(target):
    machine.succeed(f"ln -sfn {target} /nix/var/nix/profiles/system")


def activate(target):
    machine.succeed(f"ln -sfn {target} /run/current-system")


try:
    if desktop_under_test == "gnome":
        with subtest("Reproduce stale GNOME cache without the notification"):
            machine.succeed("systemctl stop peasy-applications-refresh.path")
            wait_apps([])
            select_profile(base + "/with-app")
            # GNOME debounces AppInfoMonitor by 5 seconds. Force it to reload
            # before the executable appears in PATH, as during a slow activation.
            time.sleep(8)
            assert shell_apps() == []
            activate(base + "/with-app")
            time.sleep(8)
            assert shell_apps() == [], "Negative control must reproduce the bug"

    with subtest("Post-activation notification refreshes the running launcher"):
        activate(base + "/original")
        select_profile(base + "/original")
        machine.succeed("systemctl start peasy-applications-refresh.path")
        select_profile(base + "/with-app")
        time.sleep(8)
        assert shell_apps() == []
        target_timestamp = machine.succeed("stat -c %y " + base + "/with-app")
        activate(base + "/with-app")
        wait_apps(expected)
        assert machine.succeed("stat -c %y " + base + "/with-app") == target_timestamp
        machine.succeed(
            "systemctl show peasy-applications-refresh.service -p Result | grep '=success'"
        )

    with subtest("Uninstall and rollback update search without duplicate IDs"):
        select_profile(base + "/original")
        time.sleep(8)
        activate(base + "/original")
        wait_apps([])
        select_profile(base + "/with-app")
        time.sleep(8)
        activate(base + "/with-app")
        wait_apps(expected)
        select_profile(base + "/original")
        activate(base + "/original")
        wait_apps([])
        assert machine.succeed(shell_pid_command).strip() == shell_pid
        if desktop_under_test == "xfce":
            assert machine.succeed(panel_pid_command).strip() == panel_pid
        machine.succeed(
            "test ! -e /home/alice/.local/share/applications/peasy-search-fixture.desktop"
        )
        machine.succeed(
            "test ! -e /home/alice/.local/share/applications/peasy-peasy-search-fixture.desktop"
        )
finally:
    activate(base + "/original")
    cleanup = r"""
import json
from pathlib import Path
profile = Path('/nix/var/nix/profiles/system')
previous = json.loads(Path('/run/peasy-launcher-test/previous-profile.json').read_text())
profile.unlink(missing_ok=True)
if previous is not None:
    profile.symlink_to(previous)
"""
    machine.succeed("python3 -c " + shlex.quote(cleanup))
    machine.succeed("systemctl start peasy-applications-refresh.path")
