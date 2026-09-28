{
  pkgs,
  package,
  module,
}:
let
  tools = import ./resource-tools.nix { inherit package; };
  uuid = "f72e637b-9148-4f67-a156-12ca94afc001";
  firewall = trusted: ports: {
    operation = "firewall";
    tcp = ports;
    udp = [ ];
    trusted_interfaces = trusted;
  };
  initial = [
    {
      operation = "user_create";
      name = "guest";
    }
    {
      operation = "user_groups";
      groups = [ "lp" ];
    }
    {
      operation = "persistent_mount";
      inherit uuid;
      name = "usb";
      filesystem = "ext4";
      present = true;
    }
    {
      operation = "power_settings";
      lid = "ignore";
      idle_minutes = null;
    }
    {
      operation = "power_settings";
      lid = null;
      idle_minutes = 30;
    }
    (firewall [ ] [ 3333 ])
  ];
  managed =
    changes:
    pkgs.runCommand "peasy-resource-lifecycle.nix" { } ''
      echo '${
        builtins.toJSON {
          test = "render";
          inherit changes;
        }
      }' | ${tools}/bin/resource_session > $out
    '';
  stages = {
    empty = managed [ ];
    configured = managed initial;
    trusted = managed (initial ++ [ (firewall [ "eth1" ] [ 3333 ]) ]);
    disabled = managed (
      initial
      ++ [
        (firewall [ ] [ ])
        {
          operation = "user_disabled";
          name = "guest";
          disabled = true;
        }
      ]
    );
    restored = managed (initial ++ [ (firewall [ ] [ ]) ]);
  };
  key = pkgs.runCommand "peasy-test-ssh-key" { nativeBuildInputs = [ pkgs.openssh ]; } ''
    mkdir $out
    ssh-keygen -q -t ed25519 -N "" -f $out/key
  '';
in
pkgs.testers.runNixOSTest {
  name = "peasy-resources-lifecycle";
  nodes.client = { ... }: {
    environment.systemPackages = [
      pkgs.netcat-openbsd
      pkgs.openssh
    ];
  };
  nodes.machine =
    {
      config,
      lib,
      pkgs,
      ...
    }@args:
    {
      imports = [ module ];
      options.peasyTestStage = lib.mkOption {
        type = lib.types.enum (builtins.attrNames stages);
        default = "empty";
      };
      config = lib.mkMerge (
        (lib.mapAttrsToList (
          stage: path:
          lib.mkIf (config.peasyTestStage == stage) (
            let
              rendered = import path args;
            in
            lib.mkMerge (
              [ (builtins.removeAttrs rendered [ "imports" ]) ]
              ++ map (
                part:
                let
                  resource = part args;
                in
                lib.mkMerge [
                  resource
                  {
                    # qemu-vm overrides host fileSystems. Carry the actual generated
                    # mount definition into its VM-specific filesystem set as well.
                    virtualisation.fileSystems = resource.fileSystems or { };
                  }
                ]
              ) (rendered.imports or [ ])
            )
          )
        ) stages)
        ++ [
          {
            services.peasy = {
              enable = true;
              desktop.enable = false;
              inherit package;
            };
            services.udisks2.enable = true;
            services.openssh.enable = true;
            users.users.peasytest = {
              isNormalUser = true;
              uid = 1000;
              extraGroups = [ "wheel" ];
            };
            users.users.administrator = {
              isNormalUser = true;
              uid = 1050;
            };
            users.users.guest = lib.mkIf (config.peasyTestStage != "empty") {
              openssh.authorizedKeys.keyFiles = [ "${key}/key.pub" ];
            };
            users.groups.lp = { };
            security.polkit.extraConfig = ''
              polkit.addRule(function(action, subject) {
                if (subject.user == "peasytest" && action.id.indexOf("org.freedesktop.udisks2.") == 0) return polkit.Result.YES;
              });
            '';
            networking.firewall.allowedTCPPorts = [ 2222 ];
            environment.systemPackages = [
              tools
              pkgs.python3
              pkgs.udisks
              pkgs.util-linux
              pkgs.e2fsprogs
              pkgs.lvm2
              pkgs.acl
              pkgs.netcat-openbsd
            ];
            boot.kernelModules = [ "dm_mod" ];
            systemd.services.listeners = {
              wantedBy = [ "multi-user.target" ];
              serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 /etc/peasy-listeners.py";
            };
            environment.etc."peasy-listeners.py".text = ''
              import socket, threading, time
              def serve(port):
                  s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                  s.bind(('0.0.0.0', port)); s.listen()
                  while True:
                      c, _ = s.accept(); c.close()
              for port in [2222,3333,4444]: threading.Thread(target=serve, args=(port,), daemon=True).start()
              time.sleep(100000)
            '';
            environment.etc."nixos/configuration.nix".text = ''
              { ... }: {
                imports = [ /etc/nixos/.peasy/peasy-managed.nix ];
                boot.loader.grub.devices = [ "nodev" ];
                fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
                users.users.peasytest = { isNormalUser = true; uid = 1000; extraGroups = [ "wheel" ]; };
                users.users.administrator = { isNormalUser = true; uid = 1050; };
                users.groups.lp = {};
                networking.firewall.allowedTCPPorts = [2222];
                system.stateVersion = "26.05";
              }
            '';
            specialisation = lib.mapAttrs (stage: _: { configuration.peasyTestStage = lib.mkForce stage; }) (
              builtins.removeAttrs stages [ "empty" ]
            );
            virtualisation.memorySize = 3072;
          }
        ]
      );
    };
  testScript = ''
    import json, shlex
    start_all()
    machine.wait_for_unit("peasy-system.service")
    machine.succeed("systemctl start udisks2.service")
    machine.wait_for_unit("udisks2.service")
    original = machine.succeed("readlink -f /run/current-system").strip()
    def probe(value, user="peasytest", fail=False):
        cmd = "echo " + shlex.quote(json.dumps(value)) + " | resource_session"
        cmd = "su - " + user + " -c " + shlex.quote(cmd)
        return machine.fail(cmd + " 2>&1") if fail else machine.succeed(cmd)
    def disk(action, fail=False, device="/dev/sda"):
        return probe({"test":"apply","change":{"operation":"disk","device":device,"action":action,"filesystem":"ext4" if action == "format" else None}}, fail=fail)
    def request(value, user="peasytest"):
        code = "import socket,json; s=socket.socket(socket.AF_UNIX); s.connect('/run/peasy/peasy.sock'); s.sendall((" + repr(json.dumps(value)) + "+'\\n').encode()); print(s.makefile().readline())"
        return json.loads(machine.succeed("su - " + user + " -c " + shlex.quote("python3 -c " + shlex.quote(code))))
    def switch(stage):
        # Each generation was built from the real Rust renderer by Nix. Exercise
        # real NixOS activation and daemon/source reconciliation, with no fake switch.
        machine.succeed(f"{original}/specialisation/{stage}/bin/switch-to-configuration test", timeout=180)
        machine.succeed("systemctl restart peasy-system")
        machine.wait_for_unit("peasy-system.service")
    def port(number, open):
        cmd = f"nc -z -w 2 machine {number}"
        client.wait_until_succeeds(cmd) if open else client.fail(cmd)
    # Attach a new raw USB disk owned solely by this VM test.
    image = machine.state_dir / "disposable-usb.img"
    with open(image, "wb") as f: f.truncate(128 * 1024 * 1024)
    machine.send_monitor_command(f"drive_add 0 id=stick,if=none,file={image},format=raw")
    machine.send_monitor_command("device_add usb-storage,id=stick,drive=stick,serial=peasy-disposable")
    machine.wait_until_succeeds("udisksctl info -b /dev/sda")
    inventory = json.loads(probe({"test":"inspect","query":{"domain":"storage","target":None}}))
    selected = next(d for d in inventory["devices"] if d["device"] == "/dev/sda")
    assert selected["removable"] and not selected["protected"], selected
    disk("format")
    machine.succeed("blkid /dev/sda | grep 'TYPE=\"ext4\"'")
    disk("mount")
    mountpoint = machine.succeed("findmnt -rn -S /dev/sda -o TARGET").strip()
    machine.succeed("echo preserved > " + shlex.quote(mountpoint + "/marker"))
    assert "unmount" in disk("format", fail=True)
    assert machine.succeed("cat " + shlex.quote(mountpoint + "/marker")).strip() == "preserved"
    disk("unmount")
    machine.fail("findmnt -rn -S /dev/sda")
    # Protect a USB system mount, as well as the VM's actual internal root disk.
    machine.succeed("mkdir /system-data; mount /dev/sda /system-data")
    assert "non-system" in disk("format", fail=True)
    machine.succeed("umount /system-data")
    disk("format", fail=True, device="/dev/vda")
    # A real device-mapper child protects both the layer and its backing disk.
    machine.succeed("echo '0 262144 linear /dev/sda 0' | dmsetup create peasy-layer; udevadm settle")
    disk("format", fail=True)
    disk("format", fail=True, device="/dev/dm-0")
    machine.succeed("dmsetup remove peasy-layer; udevadm settle")
    # Bind a deterministic UUID to the prebuilt persistent-mount generation.
    machine.succeed("tune2fs -U ${uuid} /dev/sda; udevadm trigger; udevadm settle")
    mount = {"operation":"persistent_mount","uuid":"${uuid}","name":"usb","filesystem":"ext4","present":True}
    assert request({"request":"propose_resources","change":mount})["response"] == "proposal"
    assert request({"request":"propose_resources","change":{"operation":"user_create","name":"guest"}})["response"] == "proposal"
    port(2222, True); port(3333, False); port(4444, False)
    switch("configured")
    machine.wait_until_succeeds("findmnt /mnt/peasy-usb")
    assert machine.succeed("cat /mnt/peasy-usb/marker").strip() == "preserved"
    machine.succeed("findmnt -n -o OPTIONS /mnt/peasy-usb | grep nodev | grep nosuid")
    assert machine.succeed("id -u guest").strip() == "1001"
    assert machine.succeed("getent shadow guest | cut -d: -f2").strip() == "!"
    groups = machine.succeed("id -nG peasytest").split()
    assert "wheel" in groups and "lp" in groups, groups
    machine.succeed("grep 'HandleLidSwitch=ignore' /etc/systemd/logind.conf; grep 'IdleActionSec=30min' /etc/systemd/logind.conf")
    port(2222, True); port(3333, True); port(4444, False)
    # Guest is correctly excluded by the socket group. Grant only temporary
    # socket access to exercise the separate UID-bound self-disable check.
    machine.succeed("setfacl -m u:guest:rw /run/peasy/peasy.sock")
    for name, user, reason in [("guest","guest","requesting account"),("administrator","peasytest","does not own")]:
        response = request({"request":"propose_resources","change":{"operation":"user_disabled","name":name,"disabled":True}}, user)
        assert response["response"] == "error" and reason in response["message"], response
    machine.succeed("setfacl -b /run/peasy/peasy.sock")
    assert request({"request":"propose_resources","change":{"operation":"user_create","name":"administrator"}})["response"] == "error"
    machine.succeed("cp ${key}/key /tmp/test-key; chmod 600 /tmp/test-key")
    ssh = "ssh -i /tmp/test-key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes guest@localhost true"
    machine.succeed(ssh)
    switch("trusted")
    port(4444, True)
    switch("disabled")
    port(2222, True); port(3333, False); port(4444, False)
    assert machine.succeed("id -u guest").strip() == "1001"
    machine.succeed("getent passwd guest | grep /bin/nologin; grep '^DenyUsers guest$' /etc/ssh/sshd_config")
    machine.fail(ssh)
    machine.succeed("journalctl -u sshd --no-pager | grep 'guest.*DenyUsers'")
    switch("restored")
    assert machine.succeed("id -u guest").strip() == "1001"
    machine.fail("getent passwd guest | grep /bin/nologin")
    machine.succeed(ssh)
    assert "wheel" in machine.succeed("id -nG peasytest").split()
  '';
}
