{
  pkgs,
  package,
  module,
}:
let
  managed = pkgs.runCommand "peasy-postgresql-managed.nix" { } ''
    ${package}/libexec/peasy-system --render-test-postgresql > $out
  '';
in
pkgs.testers.runNixOSTest {
  name = "peasy-postgresql";
  nodes.machine =
    {
      config,
      lib,
      pkgs,
      ...
    }@args:
    {
      imports = [ module ];
      options.peasyTestSetup = lib.mkOption {
        type = lib.types.bool;
        default = true;
      };
      config = lib.mkMerge [
        (lib.mkIf config.peasyTestSetup (import managed args))
        (lib.mkIf (!config.peasyTestSetup) {
          environment.etc."peasy/state.json".text = builtins.toJSON {
            packages = [ ];
            theme = {
              accent_color = null;
              color_scheme = null;
            };
          };
        })
        {
          services.peasy = {
            enable = true;
            desktop.enable = false;
            inherit package;
          };
          environment.systemPackages = [ pkgs.python3 ];
          environment.etc."nixos/configuration.nix".text = ''
            { ... }: {
              imports = [ /etc/nixos/.peasy/peasy-managed.nix ];
              boot.loader.grub.devices = [ "nodev" ];
              fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
              users.users.peasytest = { isNormalUser = true; uid = 1000; };
              system.stateVersion = "26.05";
            }
          '';
          environment.etc."peasy-postgresql-review.py".text = ''
            import json, socket, sys
            body = {"request": "propose_setup", "package": "postgresql_17", "setup": {
              "packages": [], "enable": [], "groups": [],
              "postgresql": {"package": "postgresql_17", "caller_database": True}
            }}
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(600)
                connection.connect('/run/peasy/peasy.sock')
                connection.sendall((json.dumps(body) + '\n').encode())
                response = json.loads(connection.makefile().readline())
            if sys.argv[1] == 'installed':
                assert response['response'] == 'error' and 'already managed' in response['message'], response
            elif sys.argv[1] == 'conflict':
                assert response['response'] == 'error' and 'Retained PostgreSQL 16' in response['message'], response
            else:
                assert response['response'] == 'proposal', response
          '';
          users.users.peasytest = {
            isNormalUser = true;
            uid = 1000;
            extraGroups = [ "wheel" ];
          };
          system.stateVersion = "26.05";
          specialisation.removed.configuration.peasyTestSetup = false;
          virtualisation.memorySize = 4096;
        }
      ];
    };
  testScript = ''
    machine.start(allow_reboot=True)
    machine.wait_for_unit("postgresql.service")
    machine.wait_for_unit("peasy-system.service")
    # The real daemon has no DAC bypass. The fixed inventory helper must work
    # with the server's private directory on both first use and reinstallation.
    machine.succeed("chmod 0700 /var/lib/postgresql")
    machine.succeed("su - peasytest -c 'python /etc/peasy-postgresql-review.py installed'", timeout=900)
    machine.succeed("mkdir /var/lib/postgresql/16")
    machine.succeed("su - peasytest -c 'python /etc/peasy-postgresql-review.py conflict'", timeout=900)
    machine.succeed("rmdir /var/lib/postgresql/16")
    machine.succeed("pg_isready -h /run/postgresql")
    machine.succeed("su - peasytest -c \"psql -d peasytest -c 'CREATE TABLE retained (value integer); INSERT INTO retained VALUES (42);'\"")
    machine.succeed("su - peasytest -c \"psql -At -d peasytest -c 'SELECT value FROM retained'\" | grep -qx 42")
    machine.succeed("su - postgres -c \"psql -At -c \\\"SELECT rolsuper FROM pg_roles WHERE rolname = 'peasytest'\\\"\" | grep -qx f")
    machine.succeed("su - postgres -c \"psql -At -c 'SHOW listen_addresses'\" | grep -qx localhost")
    machine.fail("su - peasytest -c \"psql -d peasytest -c 'CREATE ROLE forbidden SUPERUSER'\"")
    machine.reboot()
    machine.wait_for_unit("postgresql.service")
    machine.succeed("su - peasytest -c \"psql -At -d peasytest -c 'SELECT value FROM retained'\" | grep -qx 42")
    installed = machine.succeed("readlink -f /run/current-system").strip()
    machine.succeed(f"{installed}/specialisation/removed/bin/switch-to-configuration test")
    machine.fail("systemctl is-active --quiet postgresql.service")
    machine.succeed("test -s /var/lib/postgresql/17/PG_VERSION")
    machine.succeed("systemctl restart peasy-system")
    machine.wait_for_unit("peasy-system.service")
    machine.succeed("su - peasytest -c 'python /etc/peasy-postgresql-review.py removed'", timeout=900)
    machine.succeed(f"{installed}/bin/switch-to-configuration test")
    machine.wait_for_unit("postgresql.service")
    machine.succeed("su - peasytest -c \"psql -At -d peasytest -c 'SELECT value FROM retained'\" | grep -qx 42")
  '';
}
