{
  pkgs,
  package,
  module,
}:
let
  manifest = ../../peapod/networking/pea.json;
  value = builtins.fromJSON (builtins.readFile manifest);
  pin = {
    inherit (value)
      id
      version
      host_api
      permissions
      ;
    revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    hash = builtins.hashFile "sha256" manifest;
  };
  pinFile = pkgs.writeText "test-pea-pin.json" (builtins.toJSON pin);
  catalogue = pkgs.writeText "test-pea-catalogue.json" (
    builtins.toJSON {
      format = 1;
      peas = [
        {
          package = builtins.removeAttrs pin [ "revision" ];
          inherit (value) capabilities;
        }
      ];
    }
  );
  certificate =
    pkgs.runCommand "peasy-test-only-github-certificate" { nativeBuildInputs = [ pkgs.openssl ]; }
      ''
        mkdir -p $out
        openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
          -subj /CN=Peasy-Test-CA -addext 'basicConstraints=critical,CA:TRUE' \
          -keyout $out/ca-key.pem -out $out/ca.pem
        openssl req -new -newkey rsa:2048 -nodes -subj /CN=api.github.com \
          -keyout $out/key.pem -out server.csr
        cat > extensions <<'EOF'
        basicConstraints=critical,CA:FALSE
        keyUsage=critical,digitalSignature,keyEncipherment
        extendedKeyUsage=serverAuth
        subjectAltName=DNS:api.github.com,DNS:raw.githubusercontent.com
        EOF
        openssl x509 -req -in server.csr -CA $out/ca.pem -CAkey $out/ca-key.pem \
          -CAcreateserial -days 3650 -extfile extensions -out $out/cert.pem
      '';
  server = pkgs.writeText "test-official-pea-server.py" ''
    import http.server, ssl, json
    from pathlib import Path
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            mode = Path('/run/fixture-mode').read_text().strip()
            if self.path == '/repos/lnbits/peasy/git/ref/heads/main':
                body = json.dumps({'object': {'type': 'commit', 'sha': '${pin.revision}'}}).encode()
            elif self.path == '/lnbits/peasy/${pin.revision}/peapod/catalogue.json':
                body = Path('${catalogue}').read_bytes() if mode != 'unlisted' else b'{"format":1,"peas":[]}'
            elif self.path == '/lnbits/peasy/${pin.revision}/peapod/networking/pea.json':
                body = Path('${manifest}').read_bytes()
                if mode == 'tampered': body += b' '
                if mode == 'oversized': body = b'x' * 65537
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    httpd = http.server.HTTPServer(('127.0.0.1', 443), Handler)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain('${certificate}/cert.pem', '${certificate}/key.pem')
    httpd.socket = tls.wrap_socket(httpd.socket, server_side=True)
    httpd.serve_forever()
  '';
  client = pkgs.writeText "test-pea-ipc.py" ''
    import json, socket, sys
    from pathlib import Path
    pin = json.loads(Path('${pinFile}').read_text())
    if sys.argv[1] == 'unpublished': pin['revision'] = 'b' * 40
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(120)
        connection.connect('/run/peasy/peasy.sock')
        connection.sendall((json.dumps({'request': 'propose_pea', 'pin': pin, 'enable': True}) + '\n').encode())
        result = json.loads(connection.makefile().readline())
    print(json.dumps(result))
    assert (result['response'] == 'proposal') == (sys.argv[1] == 'valid'), result
  '';
in
pkgs.testers.runNixOSTest {
  name = "peasy-pea-fetch";
  nodes.machine = { lib, ... }: {
    imports = [ module ];
    services.peasy = {
      enable = true;
      inherit package;
      desktop.enable = false;
      tray.enable = false;
    };
    environment.etc."nixos/configuration.nix".text = "{ ... }: {}";
    networking.hosts."127.0.0.1" = [
      "api.github.com"
      "raw.githubusercontent.com"
    ];
    # Only the fixture trusts this test certificate; production uses the normal CA bundle.
    systemd.services.peasy-pea-fetch.environment.SSL_CERT_FILE = lib.mkForce "${certificate}/ca.pem";
    systemd.services.pea-fixture = {
      wantedBy = [ "multi-user.target" ];
      preStart = "echo valid > /run/fixture-mode";
      serviceConfig.ExecStart = "${pkgs.python3}/bin/python ${server}";
    };
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("peasy-system.service")
    machine.wait_for_unit("pea-fixture.service")
    machine.wait_until_succeeds("test -S /run/peasy/peasy.sock")
    machine.wait_for_open_port(443)
    machine.succeed("${pkgs.python3}/bin/python ${client} valid")
    machine.succeed("cmp /run/peasy/pea-fetch/pea.json ${manifest}")
    machine.succeed("test $(stat -c %u /run/peasy-pea-fetch/pea.json) != 0")
    machine.succeed("test $(stat -c %a /run/peasy-pea-fetch/pea.json) = 640")
    machine.succeed("test $(stat -c %a /run/peasy-pea-fetch) = 750")
    machine.succeed("systemctl show peasy-system -p RestrictAddressFamilies --value | grep -v AF_INET")
    machine.succeed("${pkgs.python3}/bin/python ${client} unpublished")
    for mode in ["unlisted", "tampered", "oversized"]:
        machine.succeed("systemctl reset-failed peasy-pea-fetch")
        machine.succeed(f"echo {mode} > /run/fixture-mode")
        machine.succeed("${pkgs.python3}/bin/python ${client} rejected")
    # A failure is recoverable and never silently enables a package.
    machine.succeed("systemctl reset-failed peasy-pea-fetch")
    machine.succeed("echo valid > /run/fixture-mode")
    machine.succeed("${pkgs.python3}/bin/python ${client} valid")
    machine.fail("test -e /etc/peasy/peas/networking.json")
  '';
}
