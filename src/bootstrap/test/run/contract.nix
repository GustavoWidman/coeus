# a small fleet's contract: every host has hosts/<name>/host.pub, admins
# also have hosts/<name>/admin; a host's secrets open on it and the admins,
# secrets/ opens everywhere
let
  root = ./..;
  hosts = builtins.attrNames (builtins.readDir (root + "/hosts"));
  key =
    host:
    let
      file = root + "/hosts/${host}/host.pub";
    in
    if builtins.pathExists file then
      builtins.replaceStrings [ "\n" ] [ "" ] (builtins.readFile file)
    else
      throw "hosts/${host}/host.pub is missing";
  admins = map key (builtins.filter (host: builtins.pathExists (root + "/hosts/${host}/admin")) hosts);
  unique = builtins.foldl' (kept: key: if builtins.elem key kept then kept else kept ++ [ key ]) [ ];
in
{
  version = 1;
  identity = "/etc/ssh/ssh_host_ed25519_key";

  questions =
    input:
    [
      {
        id = "type";
        prompt = "what kind of machine is ${input.name}?";
        kind = "choice";
        choices = [
          {
            value = "server";
            label = "server";
            help = "always on, no screen";
          }
          {
            value = "desktop";
            label = "desktop";
          }
        ];
      }
    ]
    ++ (
      if (input.answers.type or null) == "server" then
        [
          {
            id = "public";
            prompt = "does ${input.name} serve the internet?";
            kind = "confirm";
            default = "false";
          }
        ]
      else
        [ ]
    );

  enroll = input: {
    files = {
      "hosts/${input.name}/host.pub" = input.hostKey + "\n";
      "hosts/${input.name}/default.nix" =
        "{ type = \"${input.answers.type}\"; system = \"${input.facts.system}\"; }\n";
    };
    secrets = {
      "hosts/${input.name}/token.age" = {
        kind = "random";
        bytes = 16;
      };
      "hosts/${input.name}/ssh.age" = {
        kind = "ssh-ed25519";
        comment = "root@${input.name}";
      };
    };
    firstSwitch.installBootloader = input.facts.platform == "nixos";
    messages = {
      enroll = "feat(${input.name}): join the fleet";
      rekey = "chore(secrets): rekey for ${input.name}";
      title = "feat(${input.name}): join the fleet";
      body = "adds ${input.name}, a ${input.answers.type}\n";
    };
    notes = [ "add ${input.name} to the ssh aliases by hand" ];
  };

  recipients =
    path:
    let
      host = builtins.match "hosts/([^/]+)/.*" path;
    in
    if host != null then
      unique ([ (key (builtins.head host)) ] ++ admins)
    else if builtins.match "secrets/.*" path != null then
      unique (admins ++ map key hosts)
    else
      null;
}
