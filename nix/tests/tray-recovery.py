"""GNOME/XFCE regression: require visible recovery after late/returning hosts."""

from pathlib import Path


def visible_peasy_icons():
    if desktop_under_test == "xfce":
        from PIL import Image

        # XFCE does not expose the SNI button's title through accessibility.
        # Inspect the actual mint icon in the top/bottom panel regions instead.
        machine.screenshot("peasy-xfce-tray-probe")
        with Image.open(Path(machine.out_dir) / "peasy-xfce-tray-probe.png") as image:
            image = image.convert("RGB")
            points = {
                (x, y)
                for y in list(range(64)) + list(range(image.height - 64, image.height))
                for x in range(image.width)
                if all(
                    abs(a - b) <= 3
                    for a, b in zip(image.getpixel((x, y)), (92, 214, 152))
                )
            }
        icons = 0
        while points:
            component = {points.pop()}
            pending = list(component)
            while pending:
                x, y = pending.pop()
                for neighbour in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                    if neighbour in points:
                        points.remove(neighbour)
                        component.add(neighbour)
                        pending.append(neighbour)
            width = max(x for x, _ in component) - min(x for x, _ in component) + 1
            height = max(y for _, y in component) - min(y for _, y in component) + 1
            if len(component) >= 20 and 4 <= width <= 48 and 4 <= height <= 48:
                icons += 1
        return icons
    return shell_eval(
        "Object.values(Main.panel.statusArea)"
        ".filter(i => i?._indicator?.id === 'io.github.peasy.Peasy')"
        ".filter(i => i.visible && i.mapped).length"
    )


def wait_for_visible_tray():
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if visible_peasy_icons() == 1:
            assert machine.succeed("pgrep -u alice -x peasy-tray").strip() == pid
            return
        time.sleep(0.5)
    raise AssertionError("Peasy did not recover a visible tray icon")


def stop_host():
    if desktop_under_test == "xfce":
        machine.succeed(user("xfce4-panel --quit"))
    else:
        machine.succeed(
            user("gnome-extensions disable appindicatorsupport@rgcjonas.gmail.com")
        )


def start_host():
    if desktop_under_test == "xfce":
        machine.succeed(user("systemd-run --user --collect xfce4-panel"))
    else:
        machine.succeed(
            user("gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com")
        )


watcher_query = (
    "gdbus call --session --dest org.freedesktop.DBus "
    "--object-path /org/freedesktop/DBus "
    "--method org.freedesktop.DBus.NameHasOwner org.kde.StatusNotifierWatcher"
)
# Exercise the missing discovery helper through the actual session PATH.
if desktop_under_test == "gnome":
    machine.succeed(user("gjs --version"))
wait_for_visible_tray()

# Start a fresh Peasy process while the host is absent, deterministically.
stop_host()
machine.wait_until_succeeds(user(watcher_query + " | grep false"))
machine.succeed(user("kill " + pid))
machine.wait_until_fails("pgrep -u alice -x peasy-tray")
# Preserve the installed launch command, including the absolute UI path.
# A bare peasy-ui changes argv[0] and escapes the later store-path process check.
tray_command = next(
    line.removeprefix("Exec=")
    for line in autostart.splitlines()
    if line.startswith("Exec=")
)
machine.succeed(
    user("systemd-run --user --collect --unit=peasy-tray-recovery-test " + tray_command)
)
machine.wait_until_succeeds("pgrep -u alice -x peasy-tray")
pid = machine.succeed("pgrep -u alice -x peasy-tray").strip()
assert pid.isdigit(), "Exactly one Peasy process must run"
machine.wait_until_succeeds(
    user(
        "journalctl --user -u peasy-tray-recovery-test --no-pager | grep retrying.tray"
    )
)
# A host may own the name before its registration method is ready. It must
# recover without requiring a second NameOwnerChanged signal from that host.
machine.succeed(
    user(
        "systemd-run --user --collect --unit=peasy-tray-test-watcher "
        "python3 /etc/peasy-test-watcher.py"
    )
)
machine.wait_until_succeeds(
    user(
        "journalctl --user -u peasy-tray-test-watcher --no-pager | grep registration.rejected"
    )
)
machine.wait_until_succeeds(
    user(
        "journalctl --user -u peasy-tray-test-watcher --no-pager | grep registration.accepted"
    )
)
assert machine.succeed("pgrep -u alice -x peasy-tray").strip() == pid
machine.succeed(user("systemctl --user stop peasy-tray-test-watcher"))
start_host()
wait_for_visible_tray()

# A returning host must restore exactly one icon without restarting Peasy.
for _ in range(2):
    stop_host()
    machine.wait_until_succeeds(user(watcher_query + " | grep false"))
    start_host()
    wait_for_visible_tray()

# The supervisor can replace its D-Bus name; use the current one below.
items = machine.succeed(
    user(
        "gdbus call --session --dest org.kde.StatusNotifierWatcher "
        "--object-path /StatusNotifierWatcher --method org.freedesktop.DBus.Properties.Get "
        "org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems"
    )
)
names = re.findall(r"org.kde.StatusNotifierItem-" + pid + r"-\d+", items)
assert len(names) == 1, items
name = names[0]
machine.screenshot("peasy-" + desktop_under_test + "-tray-recovered")
