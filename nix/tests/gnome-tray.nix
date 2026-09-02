{
  pkgs,
  module,
  package,
}:

pkgs.testers.runNixOSTest {
  name = "peasy-gnome-tray";
  meta.timeout = 300;
  nodes.machine = {
    imports = [ module ];
    services.displayManager.gdm.enable = true;
    services.displayManager.autoLogin = {
      enable = true;
      user = "alice";
    };
    services.desktopManager.gnome.enable = true;
    services.desktopManager.gnome.extraGSettingsOverrides = ''
      [org.gnome.shell]
      welcome-dialog-last-shown-version='9999999999'
    '';
    services.xserver.enable = true;
    environment.systemPackages = [ pkgs.glib ];
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
    start_all()
    machine.wait_for_unit("graphical.target")
    machine.wait_for_x()
    extension_info = machine.succeed(
      "su - alice -c 'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u alice)/bus gnome-extensions info peasy@peasy-nixos.github.io'"
    )
    print(extension_info)
    assert "ENABLED" in extension_info or "ACTIVE" in extension_info
    machine.wait_until_succeeds("test -f /run/user/1000/peasy-user/panel-ready")
    machine.succeed(
      "test -f /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed("test -f /etc/xdg/autostart/peasy-hyprland.desktop")
    machine.succeed("test -f /etc/peasy/system-profile.json")
    machine.succeed("test -f /etc/peasy/host-configuration-path")
    machine.succeed("test -f /etc/peasy/module-import-path")
    machine.succeed("grep -qx '/etc/nixos/configuration.nix' /etc/peasy/host-configuration-path")
    machine.succeed("grep -q '\"configured_desktops\":\[\"gnome\"\]' /etc/peasy/system-profile.json")
    machine.succeed("grep -q '\"peasy_variant\":\"desktop\"' /etc/peasy/system-profile.json")
    machine.succeed("grep -q '\"installed_system_packages\"' /etc/peasy/system-profile.json")
    machine.succeed(
      "grep -q 'PeasyLauncher' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed(
      "grep -q 'peasy-launcher-dot' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/stylesheet.css"
    )
    machine.succeed(
      "grep -q '#bfffd4' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/stylesheet.css"
    )
    machine.succeed(
      "grep -q '/run/current-system/sw/bin/peasy-ui' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed(
      "grep -q 'new Clutter.ClickGesture' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed(
      "grep -q \"connect('recognize'\" /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed(
      "grep -q \"connect('key-press-event'\" /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.fail(
      "grep -q \"connect('button-press-event'\" /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.fail(
      "grep -q \"menu.connect('open-state-changed'\" /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.fail(
      "grep -q 'Ask Peasy' /run/current-system/sw/share/gnome-shell/extensions/peasy@peasy-nixos.github.io/extension.js"
    )
    machine.succeed(
      "test -f /run/current-system/sw/share/icons/hicolor/scalable/apps/io.github.peasy.Peasy.svg"
    )
    machine.succeed("test -f /run/current-system/sw/share/peasy/source/nix/module.nix")
    machine.succeed(
      "grep -q '#bfffd4' /run/current-system/sw/share/icons/hicolor/scalable/apps/io.github.peasy.Peasy.svg"
    )
    # Apply validated appearance data in an already-running GNOME session.
    machine.succeed(
      "printf '%s\\n' '{\"accent_color\":\"purple\",\"color_scheme\":\"dark\"}' > /tmp/peasy-live-theme.json"
    )
    machine.succeed(
      "su - alice -c 'XDG_RUNTIME_DIR=/run/user/$(id -u alice) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u alice)/bus peasy --sync-theme --theme-state /tmp/peasy-live-theme.json'"
    )
    live_theme = machine.succeed(
      "su - alice -c 'XDG_RUNTIME_DIR=/run/user/$(id -u alice) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u alice)/bus gsettings get org.gnome.desktop.interface accent-color; gsettings get org.gnome.desktop.interface color-scheme'"
    )
    assert "purple" in live_theme
    assert "prefer-dark" in live_theme
    worker = machine.succeed(
      "su - alice -c 'peasy --panel-worker'"
    )
    assert '"event":"error"' in worker
    assert "provider" in worker
    # The VM's virtual pointer does not map absolute or relative coordinates
    # consistently under Mutter. Verify that the active extension contains the
    # native PanelMenu activation path, then launch the same UI executable in
    # the logged-in user session.
    machine.fail("pgrep -u alice -f 'peasy-ui'")
    machine.succeed(
      "su - alice -c 'XDG_RUNTIME_DIR=/run/user/$(id -u alice) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u alice)/bus systemd-run --user --unit=peasy-vm-ui --collect /run/current-system/sw/bin/peasy-ui'"
    )
    machine.wait_until_succeeds("pgrep -u alice -f 'peasy-ui'", timeout=30)
    machine.wait_until_succeeds(
      "su - alice -c 'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u alice)/bus gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.peasy.Peasy | grep true'"
    )
    machine.succeed("sleep 2")
    machine.screenshot("peasy-mint-launcher-and-provider-setup")
  '';
}
