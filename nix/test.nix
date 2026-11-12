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
    machine.wait_for_unit("coeusd.service")
    machine.wait_for_open_port(45729)

    output = machine.succeed("coeus --verbosity debug loopback 2>&1")
    print(output)
    assert "connected to" in output, "the CLI completed the encrypted hello"
    machine.succeed("coeus localhost")
    machine.fail("coeus --config /etc/coeus-test/wrong-key.toml localhost")

    pid = machine.succeed("systemctl show --property=MainPID --value coeusd.service").strip()
    machine.succeed("systemctl kill --signal=KILL coeusd.service")
    machine.wait_until_succeeds(
        f'pid=$(systemctl show --property=MainPID --value coeusd.service); [ "$pid" != 0 ] && [ "$pid" != {pid} ]'
    )
    machine.wait_for_open_port(45729)
    machine.succeed("coeus localhost")
  '';
}
