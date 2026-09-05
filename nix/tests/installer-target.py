"""Compare pinned upstream and Peasy-integrated Calamares configuration.

Every external process and privileged write is mocked. Capture the configuration
that upstream actually hands to nixos-install; an ISO cannot be called persistent
merely because the live system includes Peasy.
"""
import importlib.util
import io
import json
import sys
import types
from pathlib import Path
from unittest.mock import patch


def capture(source, desktop, helper=None, fail_helper=False):
    values = {
        "rootMountPoint": "/test-target",
        "firmwareType": "efi",
        "partitions": [],
        "packagechooser_packagechooser": desktop,
        "username": "peasytest",
        "fullname": "Peasy Test",
        "hostname": "peasytest",
    }
    written = {}
    commands = []
    install_commands = []

    def privileged_write(argv, unused=None, contents=None):
        assert argv == ["cp", "/dev/stdin", "/test-target/etc/nixos/configuration.nix"], argv
        written["configuration"] = contents

    def check_output(argv, **unused):
        commands.append(argv)
        if helper is not None and argv == ["pkexec", helper, "/test-target"]:
            if fail_helper:
                import subprocess
                raise subprocess.CalledProcessError(1, argv, output=b"refused unsafe target")
            assert "configuration" not in written
            return b""
        assert argv in [
            ["pkexec", "nixos-generate-config", "--root", "/test-target"],
            ["pkexec", "chmod", "755", "/test-target"],
        ], argv
        return b""

    def start_install(argv, **unused):
        assert "nixos-install" in argv, argv
        assert argv[argv.index("--root") + 1] == "/test-target"
        install_commands.append(argv)
        return types.SimpleNamespace(stdout=io.BytesIO(b""), wait=lambda: 0)

    def open_fixture(path, *args, **kwargs):
        assert path == "/test-target/etc/nixos/hardware-configuration.nix", path
        return io.StringIO('{ ... }: { fileSystems."/" = { device = "none"; fsType = "tmpfs"; }; }')

    calamares = types.ModuleType("libcalamares")
    calamares.globalstorage = types.SimpleNamespace(value=values.get)
    calamares.job = types.SimpleNamespace(setprogress=lambda unused: None)
    calamares.utils = types.SimpleNamespace(
        gettext_path=lambda: "/nonexistent", gettext_languages=lambda: ["en"],
        warning=lambda *args: None, error=lambda *args: None,
        host_env_process_output=privileged_write,
    )
    spec = importlib.util.spec_from_file_location("upstream_nixos", source)
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {"libcalamares": calamares}):
        spec.loader.exec_module(module)
    with (
        patch.object(module.configparser.ConfigParser, "read", return_value=[]),
        patch.object(module.subprocess, "check_output", side_effect=check_output),
        patch.object(module.subprocess, "getoutput", return_value="26.05.test"),
        patch.object(module.subprocess, "Popen", side_effect=start_install),
        patch("builtins.open", side_effect=open_fixture),
    ):
        result = module.run()
        if fail_helper:
            assert result[0] == "Peasy target installation failed", result
            assert not install_commands and not written
            return None
        assert result is None, result
        assert len(install_commands) == 1, "upstream did not invoke mocked nixos-install"
    cfg = written["configuration"]
    assert "./hardware-configuration.nix" in cfg
    assert 'users.users."peasytest"' in cfg
    assert f"services.desktopManager.{desktop}.enable = true" in cfg
    assert ("./peasy.nix" in cfg) == (helper is not None)
    if helper is not None:
        assert commands.count(["pkexec", helper, "/test-target"]) == 1
    assert commands[0][1] == "nixos-generate-config"
    return cfg


if __name__ == "__main__":
    for desktop in ["gnome", "plasma6"]:
        original = capture(sys.argv[1], desktop)
        cfg = capture(sys.argv[2], desktop, sys.argv[3])
        # The generated system differs only by the reviewed Peasy module import.
        assert cfg == original.replace(
            "      ./hardware-configuration.nix\n",
            "      ./hardware-configuration.nix\n      ./peasy.nix\n",
        )
        capture(sys.argv[2], desktop, sys.argv[3], fail_helper=True)
        Path(f"{desktop}-configuration.nix").write_text(cfg)
    print(json.dumps({"installedTargetHasPeasy": True,
                      "generatorVerified": True, "installedBootVerified": False}))
