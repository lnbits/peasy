{ pkgs, package }:
let
  tools = import ./resource-tools.nix { inherit package; };
in
pkgs.testers.runNixOSTest {
  name = "peasy-resources-session";
  nodes.machine = { ... }: {
    users.users.peasytest = {
      isNormalUser = true;
      uid = 1000;
      linger = true;
    };
    security.rtkit.enable = true;
    services.pipewire = {
      enable = true;
      extraConfig.pipewire."90-test-sink" = {
        "context.objects" = [
          {
            factory = "adapter";
            args = {
              "factory.name" = "support.null-audio-sink";
              "node.name" = "peasy-test-sink";
              "node.description" = "Disposable test sink";
              "media.class" = "Audio/Sink";
              "audio.position" = [
                "FL"
                "FR"
              ];
            };
          }
        ];
      };
    };
    services.power-profiles-daemon.enable = true;
    services.upower.enable = true;
    services.printing.enable = true;
    services.avahi = {
      enable = true;
      nssmdns4 = true;
      publish = {
        enable = true;
        userServices = true;
      };
    };
    systemd.services.test-printer = {
      wantedBy = [ "multi-user.target" ];
      after = [ "avahi-daemon.service" ];
      serviceConfig = {
        ExecStart = "${pkgs.cups}/bin/ippeveprinter -p 8631 -f application/pdf -d /var/lib/peasy-test-printer -k PeasyFixture";
        StateDirectory = "peasy-test-printer";
      };
    };
    environment.systemPackages = [
      tools
      pkgs.pipewire
      pkgs.wireplumber
      pkgs.power-profiles-daemon
      pkgs.upower
      pkgs.cups
      pkgs.python3
    ];
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    import json, shlex
    machine.start()
    machine.wait_for_unit("user@1000.service")
    machine.succeed("systemctl start power-profiles-daemon.service cups.service")
    machine.wait_for_unit("power-profiles-daemon.service")
    machine.wait_for_unit("cups.service")
    machine.wait_for_unit("test-printer.service")
    def user(cmd):
        return "su - peasytest -c " + shlex.quote("XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus " + cmd)
    machine.succeed(user("systemctl --user start pipewire wireplumber"))
    machine.wait_until_succeeds(user("wpctl status | grep 'Disposable test sink'"))
    def probe(value, session=True, fail=False):
        cmd = "echo " + shlex.quote(json.dumps(value)) + " | PEASY_CUPS_TESTPAGE=${pkgs.cups}/share/cups/ipptool/testfile.pdf resource_session"
        cmd = user("sh -c " + shlex.quote(cmd)) if session else cmd
        return machine.fail(cmd + " 2>&1") if fail else machine.succeed(cmd)
    data = json.loads(probe({"test":"inspect","query":{"domain":"audio","target":None}}))
    sink = next(d for d in data["devices"] if d["name"] == "peasy-test-sink")
    for action, volume in [("volume",37),("mute",None),("unmute",None),("default",None)]:
        probe({"test":"apply","change":{"operation":"audio","id":sink["id"],"action":action,"volume":volume}})
        current = machine.succeed(user(f"wpctl get-volume {sink['id']}"))
        assert "0.37" in current, current
        assert ("MUTED" in current) == (action == "mute"), current
    data = json.loads(probe({"test":"inspect","query":{"domain":"power","target":None}}, session=False))
    assert "balanced" in data["profiles"] and "power-saver" in data["profiles"], data
    for profile in ["power-saver","balanced"]:
        probe({"test":"apply","change":{"operation":"power_profile","profile":profile.replace("-", "_")}}, session=False)
        assert machine.succeed("powerprofilesctl get").strip() == profile
    machine.wait_until_succeeds("ippfind -T 3 --print | grep :8631/")
    data = json.loads(probe({"test":"inspect","query":{"domain":"printing","target":None}}, session=False))
    uri = next(u for u in data["discovered_ipp_uris"] if ":8631/" in u)
    machine.log(machine.succeed("ipptool -tv " + shlex.quote(uri) + " ${pkgs.cups}/share/cups/ipptool/get-printer-attributes.test"))
    add = {"operation":"printer","name":"peasy-test","action":"add","uri":uri}
    probe({"test":"apply","change":add}, session=False)
    assert "cannot replace" in probe({"test":"apply","change":add}, session=False, fail=True)
    probe({"test":"apply","change":{"operation":"printer","name":"peasy-test","action":"default","uri":None}})
    assert "peasy-test" in machine.succeed(user("lpstat -d"))
    probe({"test":"apply","change":{"operation":"printer","name":"peasy-test","action":"test","uri":None}})
    machine.wait_until_succeeds("find /var/lib/peasy-test-printer -name '*.pdf' | grep .")
  '';
}
