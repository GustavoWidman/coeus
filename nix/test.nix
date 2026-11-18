{ self }:
{
  name = "coeus";

  nodes.machine = {
    imports = [ self.nixosModules.default ];
    programs.git.enable = true;

    environment.etc = {
      "coeus-test/key".text = "4c1f0e3b8a2d5c7e9f6a1b3d5e7f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f";
      "coeus-test/read-key".text = "9e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e1b";
      "coeus-test/read-key.toml".text = ''
        [settings]
        key_file = "/etc/coeus-test/read-key"

        [settings.hosts.loopback]
        address = "127.0.0.1"
      '';
      "coeus-test/wrong-key.toml".text = ''
        [settings]
        key = "0000000000000000000000000000000000000000000000000000000000000000"

        [settings.hosts.loopback]
        address = "127.0.0.1"
      '';
    };

    services.coeus = {
      enable = true;
      keyFile = "/etc/coeus-test/key";
      readKeyFile = "/etc/coeus-test/read-key";
      flake = "git+file:///var/fleet";
      follow.ref = "deployed";
      hosts.loopback = {
        address = "127.0.0.1";
        tags = [ "test" ];
      };
    };
  };

  testScript = ''
    import json

    machine.wait_for_unit("coeusd.service")
    machine.wait_for_open_port(45729)

    # the CLI completes the encrypted hello and reads the machine's view
    status = json.loads(machine.succeed("coeus status loopback --json"))
    assert status[0]["view"]["host"]["hostname"] == "machine", status
    machine.succeed("coeus status @test")
    machine.succeed("coeus doctor loopback")

    # a lookup publishes where the followed ref points, readable by everyone
    machine.succeed(
        "git init --quiet --initial-branch deployed /var/fleet",
        "echo '{ outputs = _: { }; }' > /var/fleet/flake.nix",
        "git -C /var/fleet add flake.nix",
        "git -C /var/fleet -c user.name=coeus -c user.email=coeus@localhost commit --quiet -m first",
    )
    machine.succeed("coeus catch-up loopback")
    published = json.loads(machine.succeed("cat /run/coeus/follow.json"))
    assert published["ref"] == "deployed", published
    assert published["rev"] == machine.succeed("git -C /var/fleet rev-parse HEAD").strip(), published
    assert published["committed"] == int(machine.succeed("git -C /var/fleet log -1 --format=%ct")), published
    assert machine.succeed("stat -c %a /run/coeus /run/coeus/follow.json").split() == ["755", "644"]
    machine.succeed("su nobody -s /bin/sh -c 'cat /run/coeus/follow.json'")

    # the read-only key watches the machine and is refused anything that
    # makes coeusd work, a deploy of the commit just looked up included
    reader = "coeus --config /etc/coeus-test/read-key.toml"
    watched = json.loads(machine.succeed(f"{reader} status loopback --json"))
    assert watched[0]["view"]["host"]["hostname"] == "machine", watched
    machine.succeed(f"{reader} history loopback")
    rev = published["rev"]
    refused = machine.fail(f"{reader} deploy loopback --rev {rev} --build --detach 2>&1")
    assert "this key may only read" in refused, refused
    refused = machine.fail(f"{reader} catch-up loopback 2>&1")
    assert "this key may only read" in refused, refused

    # a restart, as in the switch that upgrades coeus, keeps it
    machine.succeed("systemctl restart coeusd.service")
    machine.succeed("test -f /run/coeus/follow.json")
    machine.wait_for_open_port(45729)

    # another fleet key never gets past the handshake
    refused = machine.fail("coeus --config /etc/coeus-test/wrong-key.toml status loopback 2>&1")
    assert "fleet key" in refused, refused

    # systemd brings back a coeusd that dies, and it answers again
    pid = machine.succeed("systemctl show --property=MainPID --value coeusd.service").strip()
    machine.succeed("systemctl kill --signal=KILL coeusd.service")
    machine.wait_until_succeeds(
        f'pid=$(systemctl show --property=MainPID --value coeusd.service); [ "$pid" != 0 ] && [ "$pid" != {pid} ]'
    )
    machine.wait_for_open_port(45729)
    machine.succeed("coeus status loopback")
  '';
}
