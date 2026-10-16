{
  pkgs,
  package,
  module,
}:
let
  managed = pkgs.runCommand "peasy-network-vm-managed.nix" { } ''
    ${package}/libexec/peasy-system --render-test-network > $out
  '';
in
pkgs.testers.runNixOSTest {
  name = "peasy-networking";
  nodes.machine = { lib, ... }: {
    imports = [
      module
      "${managed}"
    ];
    boot.kernelModules = [ "mac80211_hwsim" ];
    networking.usePredictableInterfaceNames = false;
    # QEMU disables wireless by default; this VM deliberately has synthetic radios.
    networking.wireless.enable = lib.mkOverride 9 true;
    services.peasy = {
      enable = true;
      inherit package;
      desktop.enable = false;
      tray.enable = false;
    };
    environment.etc."nixos/configuration.nix".text = "{ ... }: {}";
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    import shlex

    machine.start()
    machine.wait_for_unit("NetworkManager.service")
    machine.wait_for_unit("peasy-network-profiles.service")
    machine.succeed("nmcli -g WIFI-PROPERTIES.AP device show wlan0 | grep -qx yes")
    machine.wait_until_succeeds("nmcli -g GENERAL.STATE device show wlan0 | grep -q '^30'", timeout=60)
    path = "/etc/NetworkManager/system-connections/peasy-wireless.nmconnection"
    machine.succeed(f"test $(stat -c %a {path}) = 600")
    machine.succeed(f"cp {path} /tmp/original.nmconnection")
    # Synthetic radios and a fixture-only password: no host networking or real secrets.
    machine.succeed("printf '%s\\n' 'fixture-password' | nmcli --wait 30 --ask connection up id peasy-wireless")
    machine.succeed("nmcli -g GENERAL.STATE device show wlan0 | grep -q '^100'")
    machine.succeed("ip -4 address show wlan0 | grep -q '10.42.'")
    # Exercise the exact session-profile security properties used by simple
    # Wi-Fi connections against a synthetic access point on the second radio.
    ap_ssid = machine.succeed("nmcli -g 802-11-wireless.ssid connection show id peasy-wireless").strip()
    ap_bssid = machine.succeed("nmcli -g GENERAL.HWADDR device show wlan0").strip()
    machine.wait_until_succeeds("nmcli -t -f SSID device wifi list ifname wlan1 --rescan yes | grep -F " + shlex.quote(ap_ssid), timeout=60)
    machine.succeed(
        "nmcli connection add save no connection.id peasy-wifi-session "
        "connection.type 802-11-wireless connection.interface-name wlan1 "
        "connection.autoconnect false 802-11-wireless.mode infrastructure "
        "802-11-wireless.ssid " + shlex.quote(ap_ssid) + " "
        "802-11-wireless.bssid " + shlex.quote(ap_bssid) + " "
        "802-11-wireless-security.key-mgmt wpa-psk "
        "802-11-wireless-security.proto rsn 802-11-wireless-security.psk-flags 2 "
        "ipv4.method auto ipv6.method auto"
    )
    machine.succeed("printf '%s\\n' 'fixture-password' | nmcli --wait 45 --ask connection up id peasy-wifi-session")
    machine.succeed("nmcli -g GENERAL.STATE device show wlan1 | grep -q '^100'")
    machine.succeed("nmcli -g 802-11-wireless-security.key-mgmt connection show id peasy-wifi-session | grep -qx wpa-psk")
    machine.succeed("! grep -R -F 'fixture-password' /run/NetworkManager/system-connections /etc/NetworkManager/system-connections")
    machine.succeed("nmcli connection delete id peasy-wifi-session")
    machine.succeed("nmcli connection down id peasy-wireless")
    # Open networks omit wifi-security completely (key-mgmt=none is WEP).
    machine.succeed("nmcli connection add save no connection.id peasy-open-ap connection.type 802-11-wireless connection.interface-name wlan0 connection.autoconnect false 802-11-wireless.ssid PeasyOpenFixture 802-11-wireless.mode ap ipv4.method shared ipv6.method disabled")
    machine.succeed("nmcli --wait 30 connection up id peasy-open-ap")
    ap_bssid = machine.succeed("nmcli -g GENERAL.HWADDR device show wlan0").strip()
    machine.wait_until_succeeds("nmcli -t -f SSID device wifi list ifname wlan1 --rescan yes | grep -qx PeasyOpenFixture", timeout=60)
    machine.succeed(
        "nmcli connection add save no connection.id peasy-open-session "
        "connection.type 802-11-wireless connection.interface-name wlan1 "
        "connection.autoconnect false 802-11-wireless.mode infrastructure "
        "802-11-wireless.ssid PeasyOpenFixture 802-11-wireless.bssid " + shlex.quote(ap_bssid) + " "
        "ipv4.method auto ipv6.method auto"
    )
    machine.succeed("nmcli --wait 45 connection up id peasy-open-session")
    machine.succeed("nmcli -g GENERAL.STATE device show wlan1 | grep -q '^100'")
    machine.succeed("nmcli connection delete id peasy-open-session")
    machine.succeed("nmcli connection delete id peasy-open-ap")
    # NixOS withdraws /etc files on removal. Reload must withdraw the inactive profile.
    machine.succeed(f"rm {path}")
    machine.succeed("systemctl restart peasy-network-profiles")
    machine.fail("nmcli connection show id peasy-wireless")
    machine.succeed(f"install -m600 /tmp/original.nmconnection {path}")
    machine.succeed("systemctl restart peasy-network-profiles")
    machine.succeed("nmcli connection show id peasy-wireless")
    # Verify the canonical property syntax and save=no semantics used by session plans.
    machine.succeed("nmcli connection add save no connection.id peasy-temporary connection.type 802-3-ethernet connection.interface-name eth0 connection.autoconnect false ipv4.method disabled ipv6.method disabled")
    machine.succeed("nmcli connection show id peasy-temporary")
    machine.fail("test -e /etc/NetworkManager/system-connections/peasy-temporary.nmconnection")
    machine.succeed("test $(stat -c %a /run/NetworkManager/system-connections/peasy-temporary.nmconnection) = 600")
    machine.succeed("nmcli connection delete id peasy-temporary")
    machine.fail("test -e /run/NetworkManager/system-connections/peasy-temporary.nmconnection")
  '';
}
