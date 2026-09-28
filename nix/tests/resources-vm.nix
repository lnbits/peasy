{
  pkgs,
  package,
  module,
}:
pkgs.testers.runNixOSTest {
  name = "peasy-resources";
  nodes.machine = { ... }: {
    imports = [ module ];
    services.peasy = {
      enable = true;
      desktop.enable = false;
      inherit package;
    };
    users.users.peasytest = {
      isNormalUser = true;
      uid = 1000;
      extraGroups = [ "wheel" ];
    };
    environment.systemPackages = [ pkgs.python3 ];
    virtualisation.memorySize = 2048;
    environment.etc."peasy-test-retained".source =
      pkgs.runCommand "peasy-test-retained" { }
        "mkdir $out";
    environment.etc."peasy-test-deleted".source = pkgs.runCommand "peasy-test-deleted" { } "mkdir $out";
    systemd.services.resource-demo = {
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = "touch /run/peasy-resource-demo";
    };
    environment.etc."nixos/configuration.nix".text = ''
      { ... }: {
        boot.loader.grub.devices = [ "nodev" ];
        fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
        system.stateVersion = "26.05";
      }
    '';
  };
  testScript = ''
    import json
    machine.start()
    machine.wait_for_unit("peasy-system.service")
    machine.wait_for_file("/run/peasy/peasy.sock")
    def request(value):
        code = "import socket,json; s=socket.socket(socket.AF_UNIX); s.connect('/run/peasy/peasy.sock'); s.sendall((" + repr(json.dumps(value)) + "+'\\n').encode()); print(s.makefile().readline())"
        import shlex
        return json.loads(machine.succeed("python3 -c " + shlex.quote(code)))
    status = request({"request":"inspect_resources", "query":{"domain":"services","target":"resource-demo.service"}})
    assert status["response"] == "resources", status
    before = machine.succeed("readlink /run/current-system")
    plan = {"operation":"service","unit":"resource-demo.service","action":"start"}
    review = request({"request":"propose_resources","change":plan})
    assert review["response"] == "proposal", review
    applied = request({"request":"apply","proposal":review["proposal"]["id"]})
    assert applied["response"] == "applied", applied
    machine.succeed("test -f /run/peasy-resource-demo")
    assert machine.succeed("readlink /run/current-system") == before
    assert request({"request":"apply","proposal":review["proposal"]["id"]})["response"] == "error"
    forbidden = {"operation":"service","unit":"peasy-system.service","action":"stop"}
    assert request({"request":"propose_resources","change":forbidden})["response"] == "error"
    plan["action"] = "restart"
    stale = request({"request":"propose_resources","change":plan})
    machine.succeed("systemctl stop resource-demo.service")
    rejected = request({"request":"apply","proposal":stale["proposal"]["id"]})
    assert rejected["response"] == "error", rejected
    assert "changed" in rejected["message"], rejected
    invalid_mount = {"operation":"persistent_mount","name":"usb","uuid":"missing-uuid","filesystem":"ext4","present":True}
    response = request({"request":"propose_resources","change":invalid_mount})
    assert response["response"] == "error", response
    assert "no longer present" in response["message"], response
    machine.succeed("systemctl show peasy-system -p CapabilityBoundingSet --value | grep '^$'")
    assert request({"request":"inspect_resources","query":{"domain":"diagnostics","target":None}})["response"] == "resources"
    # The VM has no system-profile link. Bind its profile to the running
    # system, then add synthetic rollback references inside this VM only.
    machine.succeed("ln -s $(readlink -f /etc/peasy-test-retained) /nix/var/nix/profiles/system-997-link")
    machine.succeed("ln -s $(readlink -f /etc/peasy-test-deleted) /nix/var/nix/profiles/system-998-link")
    machine.succeed("ln -s $(readlink -f /run/current-system) /nix/var/nix/profiles/system-999-link")
    machine.succeed("ln -s system-999-link /nix/var/nix/profiles/system")
    inspection = request({"request":"inspect_resources","query":{"domain":"nix_maintenance","target":None}})
    assert inspection["response"] == "resources", inspection
    deletion = {"operation":"nix_delete_generations","generations":[999]}
    assert request({"request":"propose_resources","change":deletion})["response"] == "error"
    deletion["generations"] = [997,998]
    assert request({"request":"propose_resources","change":deletion})["response"] == "error"
    deletion["generations"] = [998]
    reviewed = request({"request":"propose_resources","change":deletion})
    assert reviewed["response"] == "proposal", reviewed
    applied = request({"request":"apply","proposal":reviewed["proposal"]["id"]})
    assert applied["response"] == "applied", applied
    machine.fail("test -L /nix/var/nix/profiles/system-998-link")
    machine.succeed("test -L /nix/var/nix/profiles/system-997-link")
    assert machine.succeed("readlink /run/current-system") == before
  '';
}
