"""Check built artwork and keep ISO branding separate from installation logic."""
from pathlib import Path
import struct
import sys
import yaml

artwork, extensions, upstream, grub_upstream = map(Path, sys.argv[1:])
branding = extensions / "share/calamares/branding/peasy"


def load(path):
    return yaml.safe_load(path.read_text())


settings = load(extensions / "etc/calamares/settings.conf")
# Upstream substitutes its own package prefix during installation.
expected_settings = yaml.safe_load(
    (upstream / "config/settings.conf").read_text().replace("@out@", str(extensions))
)
expected_settings["branding"] = "peasy"
assert settings == expected_settings, "branding changed installer workflow/settings"

original = load(upstream / "branding/nixos/branding.desc")
desc = load(branding / "branding.desc")
assert desc["componentName"] == "peasy"
assert desc["strings"] == original["strings"], "keep NixOS wording, links and bootloader name"
assert desc["style"]["SidebarBackgroundCurrent"] == "#5cd698"
assert desc["style"]["SidebarBackground"] == "#246347"
assert desc["style"]["SidebarTextCurrent"] == "#101715"
assert desc["images"]["productIcon"] == original["images"]["productIcon"]
assert desc["images"]["productLogo"] == original["images"]["productLogo"]
assert desc["images"]["productWelcome"] == "peasy-welcome.png"
for filename in desc["images"].values():
    assert (branding / filename).is_file(), filename
assert desc["slideshow"] == original["slideshow"]
for source in (upstream / "branding/nixos").rglob("*"):
    if source.is_file() and source.name != "branding.desc":
        assert (branding / source.relative_to(upstream / "branding/nixos")).read_bytes() == source.read_bytes()

for name, dimensions in [("grub/background.png", (1, 1)), ("grub/select_c.png", (638, 36)), ("installer-welcome.png", (320, 250))]:
    data = (artwork / name).read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    assert struct.unpack(">II", data[16:24]) == dimensions
assert (branding / "peasy-welcome.png").read_bytes() == (artwork / "installer-welcome.png").read_bytes()
for source in grub_upstream.rglob("*"):
    if source.is_file() and source.name not in {"theme.txt", "background.png", "select_c.png"}:
        assert (artwork / "grub" / source.relative_to(grub_upstream)).read_bytes() == source.read_bytes()
theme = (artwork / "grub/theme.txt").read_text()
expected_theme = (grub_upstream / "theme.txt").read_text().replace("#5579C4", "#246347").replace("#7EBAE4", "#5cd698")
assert theme.startswith(expected_theme), "keep upstream GRUB layout and behaviour"
assert 'text = "Includes Peasy"' in theme
assert 'id = "__timeout__"' in theme
print("ISO branding assets and unchanged installer workflow checks passed")
