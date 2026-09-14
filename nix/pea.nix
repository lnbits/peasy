{
  lib,
  runCommand,
  id,
}:
let
  source = ../peas + "/${id}/pea.json";
  manifest = builtins.fromJSON (builtins.readFile source);
in
assert manifest.id == id && manifest.host_api == 1;
runCommand "peasy-pea-${id}-${manifest.version}"
  {
    passthru = {
      hostApi = manifest.host_api;
      permissions = manifest.permissions;
      capabilities = manifest.capabilities;
    };
    meta.description = "Peasy ${id} domain instructions and guarded host schema";
  }
  ''
    install -Dm444 ${source} "$out/share/peasy/peas/${id}.json"
  ''
