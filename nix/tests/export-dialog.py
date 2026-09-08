"""Open/cancel the real export picker with no inherited GTK schema paths."""

import shlex

with subtest("Export folder picker opens and cancels without crashing"):
    # Restart only the disposable test UI, not the desktop or Peasy daemon.
    ui_pid = machine.succeed(
        "pgrep -u alice -f '^/nix/store/[^ ]+/bin/peasy-ui'"
    ).strip()
    assert ui_pid.isdigit(), ui_pid
    machine.succeed("kill -TERM " + ui_pid)
    # A tray-spawned child may remain as a zombie; it has exited but kill -0
    # still succeeds. Wait for its command line to disappear instead.
    machine.wait_until_fails(
        "pgrep -u alice -f '^/nix/store/[^ ]+/bin/peasy-ui'", timeout=30
    )
    machine.succeed("mkdir -p /tmp/peasy-empty-data")
    launch = (
        "XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "systemd-run --user --unit=peasy-export-test --collect --no-block "
        "env -u GSETTINGS_SCHEMA_DIR XDG_DATA_DIRS=/tmp/peasy-empty-data "
        "GTK_A11Y=atspi GDK_DEBUG=no-portals peasy-ui"
    )
    machine.succeed("su - alice -c " + shlex.quote(launch))
    # Use accessibility actions rather than screen coordinates or test-only UI
    # hooks. The same GTK button/callback is used by an ordinary user.
    probe = r"""
import time
import pyatspi
from gi.repository import GLib

def dispatch_events():
    # AT-SPI updates its cached tree from D-Bus events as dialogs open/close.
    context = GLib.MainContext.default()
    while context.pending():
        context.iteration(False)

def descendants(node):
    yield node
    for child in node:
        if child is not None:
            yield from descendants(child)

def find(name):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        dispatch_events()
        for app in pyatspi.Registry.getDesktop(0):
            if app is not None and "peasy" in app.name.lower():
                for node in descendants(app):
                    if node.name == name:
                        return node
        time.sleep(0.25)
    for app in pyatspi.Registry.getDesktop(0):
        if app is not None:
            print("APP", repr(app.name), flush=True)
            if "peasy" in app.name.lower():
                for node in descendants(app):
                    print(node.getRoleName(), repr(node.name), flush=True)
    raise AssertionError("Peasy accessibility element missing: " + name)

def wait_closed():
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        dispatch_events()
        if not any(node.name == "Export here"
                   for app in pyatspi.Registry.getDesktop(0)
                   if app is not None and "peasy" in app.name.lower()
                   for node in descendants(app)):
            return
        time.sleep(0.25)
    raise AssertionError("Export picker did not close")

def click(name):
    action = find(name).queryAction()
    assert action.nActions > 0, name
    assert action.doAction(0), name

click("Export system")
find("Export here")
click("Cancel")
wait_closed()
find("Export system")
# A second open ensures cancelling did not break the callback or settings UI.
click("Export system")
find("Export here")
click("Cancel")
wait_closed()
find("Export system")
print("Export picker opened and cancelled twice")
"""
    # Put the quoted probe in a root-owned VM-only file so user()'s shell
    # quoting cannot alter Python code. No export destination is ever accepted.
    import base64

    encoded = base64.b64encode(probe.encode()).decode()
    machine.succeed("echo " + encoded + " | base64 -d > /tmp/peasy-export-probe.py")
    try:
        machine.succeed(
            user(
                "env "
                + export_probe_environment
                + " python3 /tmp/peasy-export-probe.py"
            )
        )
    except Exception:
        machine.screenshot("peasy-export-failed")
        raise
    machine.succeed(user("systemctl --user is-active peasy-export-test.service"))
    machine.succeed("test ! -e /home/alice/peasy-system-config")
    machine.screenshot("peasy-export-cancelled")
