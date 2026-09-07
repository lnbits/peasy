"""Narrow the pinned upstream chooser without replacing its desktop recipes."""
from pathlib import Path
import sys
import yaml

path = Path(sys.argv[1])
config = yaml.safe_load(path.read_text())
choices = []
for desktop in ("gnome", "xfce"):
    matches = [item for item in config["items"] if item["id"] == desktop]
    if len(matches) != 1 or matches[0]["packages"] != [desktop]:
        raise ValueError(f"Upstream installer choice changed: {desktop}")
    choices.append(matches[0])
choices[1]["name"] = "XFCE (lightweight)"
choices[1]["description"] = choices[1]["description"].replace(
    "</html>",
    "<br/><br/>Peasy is included. Installing XFCE from this image requires an Internet connection.</html>",
)
config["default"] = "gnome"
config["items"] = choices
path.write_text(yaml.safe_dump(config, sort_keys=False))
