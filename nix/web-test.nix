# a machine serving the dashboard on a tailnet it never joins: tailscaled runs
# but is not logged in, so coeus web serves the site on its origin socket and
# keeps cloudflared down
{ self }:
{
  name = "coeus-web";

  nodes.machine = {
    imports = [ self.nixosModules.default ];

    environment.etc = {
      "coeus-test/key".text = "4c1f0e3b8a2d5c7e9f6a1b3d5e7f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f";
      "coeus-test/read-key".text = "9e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e1b";
      "coeus-test/session-key".text = "1d3f5a7c9e1b2d4f6a8c0e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c5e7b9d2f";
      "coeus-test/tunnel.json".text = builtins.toJSON {
        AccountTag = "0123456789abcdef0123456789abcdef";
        TunnelID = "6f1c2a9e-3b7d-4e58-9a0c-d2e4f6a8b1c3";
        TunnelSecret = "c2VjcmV0";
      };
    };

    services.tailscale.enable = true;

    services.coeus = {
      enable = true;
      keyFile = "/etc/coeus-test/key";
      readKeyFile = "/etc/coeus-test/read-key";
      flake = "git+file:///var/fleet";
      hosts.loopback.address = "127.0.0.1";

      web = {
        enable = true;
        publicOrigin = "https://coeus.example.com";
        sessionKeyFile = "/etc/coeus-test/session-key";
        signin.urls = [ "https://machine.tail0000.ts.net:8443" ];
        tunnel = {
          id = "6f1c2a9e-3b7d-4e58-9a0c-d2e4f6a8b1c3";
          credentialsFile = "/etc/coeus-test/tunnel.json";
        };
      };
    };
  };

  testScript = ''
    import json
    from datetime import timedelta

    socket = "/run/coeus-web/origin.sock"
    site = f"curl --silent --unix-socket {socket} http://coeus"

    def answer(path):
        status, size = machine.succeed(
            f"{site}{path} --output /dev/null --write-out '%{{http_code}} %{{size_download}}'"
        ).split()
        return int(status), int(size)

    def serves():
        assert "<html" in machine.succeed(f"{site}/ --fail"), "the app's shell"
        hello = json.loads(machine.succeed(f"{site}/api/hello --fail"))
        assert hello["signin"] == ["https://machine.tail0000.ts.net:8443"], hello
        assert answer("/api/stream") == (401, 0), "nothing but the app without a session"

    def tunnel_down():
        machine.fail("pgrep --uid coeus-web cloudflared")

    machine.wait_for_unit("coeusd.service")
    machine.wait_for_unit("coeus-web.service")
    machine.wait_until_succeeds(f"test -S {socket}")
    assert machine.succeed(f"stat -c '%U %a' {socket}").split() == ["coeus-web", "600"]
    serves()

    # the probe reaches tailscaled from inside the sandbox. tailscaled is not
    # logged in, so the host never becomes ready and cloudflared never starts
    machine.wait_until_succeeds(
        "journalctl --unit coeus-web.service --grep 'not ready to serve: tailscale is'"
    )
    machine.succeed("test -f /run/coeus-web/cloudflared.json")
    machine.sleep(timedelta(seconds=10))
    tunnel_down()
    machine.fail("journalctl --unit coeus-web.service --grep 'ready to serve$'")

    # no certificate to fetch, which is no failure
    machine.wait_until_succeeds("journalctl --unit coeus-web-cert.service --grep 'trying again later'")
    machine.fail("systemctl is-failed coeus-web-cert.service")

    # coeusd restarting, as in the switch that upgrades it, leaves coeus web serving
    pid = machine.succeed("systemctl show --property=MainPID --value coeus-web.service").strip()
    machine.succeed("systemctl restart coeusd.service")
    machine.wait_for_open_port(45729)
    machine.sleep(timedelta(seconds=5))
    assert machine.succeed("systemctl show --property=MainPID --value coeus-web.service").strip() == pid
    serves()
    tunnel_down()
  '';
}
