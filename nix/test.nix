{ self }:
{
  name = "coeus";

  nodes.machine = {
    imports = [ self.nixosModules.default ];

    environment.etc = {
      "coeus-test/key".text = "4c1f0e3b8a2d5c7e9f6a1b3d5e7f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f";
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
      flake = "git+https://example.invalid/nix";
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
