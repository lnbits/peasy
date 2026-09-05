{
  pkgs,
  module,
  package,
  desktop,
}:
let
  gnome = desktop == "gnome";
in
pkgs.testers.runNixOSTest {
  name = "peasy-${desktop}-tray";
  meta.timeout = 900;
  globalTimeout = 900;
  nodes.machine = {
    imports = [
      module
      ../iso-appearance.nix
    ];
    services.displayManager.autoLogin = {
      enable = true;
      user = "alice";
    };
    services.displayManager.gdm.enable = gnome;
    services.displayManager.sddm.enable = !gnome;
    services.desktopManager.gnome.enable = gnome;
    services.desktopManager.plasma6.enable = !gnome;
    services.desktopManager.gnome.extraGSettingsOverrides = pkgs.lib.mkIf gnome ''
      [org.gnome.shell]
      welcome-dialog-last-shown-version='9999999999'
    '';
    services.xserver.enable = true;
    environment.systemPackages = [ pkgs.glib ] ++ pkgs.lib.optional (!gnome) pkgs.kdePackages.kconfig;
    environment.sessionVariables.GSK_RENDERER = "cairo";
    services.peasy = {
      enable = true;
      inherit package;
    };
    users.users.alice = {
      isNormalUser = true;
      password = "test";
      extraGroups = [ "wheel" ];
    };
    virtualisation.memorySize = 4096;
    virtualisation.resolution = {
      x = 1024;
      y = 768;
    };
  };
  testScript = ''
    import json
    import re

    def user(command):
        # Preserve the real graphical session environment, including Plasma's
        # display authorization; a fresh login shell deliberately loses it.
        return "su - alice -c 'XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus systemd-run --user --pipe --wait --collect --quiet -- " + command + "'"

    start_all()
    machine.wait_for_unit("graphical.target")
    machine.wait_until_succeeds("test -S /run/user/1000/bus")
    machine.wait_until_succeeds("pgrep -u alice -x peasy-tray")
    pid = machine.succeed("pgrep -u alice -x peasy-tray").strip()
    assert pid.isdigit(), "Exactly one shared tray process must run"
    items = machine.wait_until_succeeds(user(
        "gdbus call --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems | grep StatusNotifierItem-" + pid + "-"
    ))
    match = re.search(r"org.kde.StatusNotifierItem-" + pid + r"-\d+", items)
    assert match is not None, items
    name = match.group(0)
    identity = machine.succeed(user("gdbus call --session --dest " + name + " --object-path /StatusNotifierItem --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem Id"))
    assert "io.github.peasy.Peasy" in identity
    autostart = machine.succeed("cat /etc/xdg/autostart/peasy-tray.desktop")
    assert "OnlyShowIn" not in autostart and "NotShowIn" not in autostart
    machine.fail("test -e /etc/xdg/autostart/peasy-hyprland.desktop")
    profile = json.loads(machine.succeed("cat /etc/peasy/system-profile.json"))
    assert profile["configured_desktops"] == ["${if gnome then "gnome" else "kde_plasma"}"]
    assert profile["peasy_variant"] == "desktop"
    machine.succeed("test -f /etc/peasy/module-import-path")
    machine.succeed("test -f /run/current-system/sw/share/peasy/source/nix/module.nix")
    machine.succeed("test -f /run/current-system/sw/share/icons/hicolor/scalable/apps/io.github.peasy.Peasy.svg")
    ${
      if gnome then
        ''
          extension = machine.succeed(user("gdbus call --session --dest org.gnome.Shell.Extensions --object-path /org/gnome/Shell/Extensions --method org.gnome.Shell.Extensions.GetExtensionInfo ${pkgs.gnomeExtensions.appindicator.extensionUuid}"))
          assert "'enabled': <true>" in extension
          accent = machine.succeed(user("gsettings get org.gnome.desktop.interface accent-color"))
          assert "green" in accent
          background = machine.succeed(user("gsettings get org.gnome.desktop.background picture-uri"))
          assert "peasy_bg.png" in background
          assert "peasy_bg.png" in machine.succeed(user("gsettings get org.gnome.desktop.background picture-uri-dark"))
        ''
      else
        ''
          machine.fail("test -e /etc/xdg/autostart/peasy-panel.desktop")
          machine.wait_until_succeeds("test -f /home/alice/.local/state/peasy/iso-appearance-v1")
          accent = machine.succeed(user("kreadconfig6 --file kdeglobals --group General --key AccentColor"))
          assert "58,148,74" in accent, repr(accent)
          try:
              machine.wait_until_succeeds("grep -q peasy_bg.png /home/alice/.config/plasma-org.kde.plasma.desktop-appletsrc", timeout=60)
          except Exception:
              print(machine.succeed("cat /home/alice/.config/plasma-org.kde.plasma.desktop-appletsrc"))
              print(machine.succeed("journalctl --no-pager _SYSTEMD_USER_UNIT=peasy-iso-appearance.service"))
              raise
        ''
    }
    machine.screenshot("peasy-${desktop}-iso-defaults")
    machine.succeed("printf '%s\\n' '{\"accent_color\":\"purple\",\"color_scheme\":\"dark\"}' > /tmp/peasy-live-theme.json")
    machine.succeed(user("peasy --sync-theme --theme-state /tmp/peasy-live-theme.json"))
    ${
      if gnome then
        ''
          assert "purple" in machine.succeed(user("gsettings get org.gnome.desktop.interface accent-color"))
          assert "prefer-dark" in machine.succeed(user("gsettings get org.gnome.desktop.interface color-scheme"))
        ''
      else
        ''
          assert "BreezeDark" in machine.succeed(user("kreadconfig6 --file kdeglobals --group General --key ColorScheme"))
          accent = machine.succeed(user("kreadconfig6 --file kdeglobals --group General --key AccentColor"))
          assert "145,65,172" in accent, repr(accent)
          # Restarting the ISO defaults service must not overwrite a user's choice.
          machine.succeed(user("systemctl --user restart peasy-iso-appearance.service"))
          assert "145,65,172" in machine.succeed(user("kreadconfig6 --file kdeglobals --group General --key AccentColor"))
        ''
    }
    worker = machine.succeed(user("peasy --panel-worker"))
    assert '"event":"error"' in worker and "provider" in worker
    machine.fail("pgrep -u alice -f '^/nix/store/[^ ]+/bin/peasy-ui'")
    machine.succeed(user("gdbus call --session --dest " + name + " --object-path /StatusNotifierItem --method org.kde.StatusNotifierItem.Activate 0 0"))
    machine.wait_until_succeeds("pgrep -u alice -f '^/nix/store/[^ ]+/bin/peasy-ui'", timeout=30)
    machine.wait_until_succeeds(user("gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.peasy.Peasy | grep true"))
    machine.screenshot("peasy-${desktop}-provider-setup")
  '';
}
