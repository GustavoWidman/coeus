{ self }:
{
  name = "coeus";

  nodes.machine =
    { pkgs, ... }:
    let
      fakePush = pkgs.writers.writePython3Bin "fake-push" {
        libraries = [ pkgs.python3Packages.cryptography ];
        flakeIgnore = [ "E501" ];
      } (builtins.readFile ./fake-push.py);
      # the browser of RFC 8291's appendix A, whose private key the fake push service holds
      keys = {
        p256dh = "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
        auth = "BTBZMqHH6r4Tts7J_aSIgg";
      };
    in
    {
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
        "coeus-test/subscriptions.json".text = builtins.toJSON [
          {
            name = "phone";
            endpoint = "http://127.0.0.1:8080/push/phone";
            inherit keys;
          }
          {
            name = "old-laptop";
            endpoint = "http://127.0.0.1:8080/push/gone";
            inherit keys;
          }
        ];
      };

      systemd.services.fake-push = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          ExecStart = "${fakePush}/bin/fake-push /var/lib/fake-push/seen.jsonl";
          StateDirectory = "fake-push";
        };
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
        push = {
          origin = "https://coeus.example.com";
          vapidKeyFile = "/var/lib/coeus-test/vapid";
          subscriptionsFile = "/etc/coeus-test/subscriptions.json";
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

    # with no VAPID key yet the push settings are unusable, and say why
    unusable = machine.fail("coeus push test 2>&1")
    assert "failed to read /var/lib/coeus-test/vapid" in unusable, unusable

    # a key from `coeus push keygen`, and a test push from the rendered config
    # reaches every device; a gone one is reported, never an exit status
    machine.succeed("mkdir -p /var/lib/coeus-test")
    machine.succeed("coeus push keygen > /var/lib/coeus-test/vapid 2> /tmp/vapid.pub")
    public = machine.succeed("cat /tmp/vapid.pub").rsplit(": ", 1)[1].strip()
    machine.wait_for_open_port(8080)
    report = machine.succeed("coeus push test")
    assert "phone: delivered" in report, report
    assert "old-laptop: gone (410)" in report, report

    def pushed(tag):
        lines = machine.succeed("cat /var/lib/fake-push/seen.jsonl").splitlines()
        seen = [json.loads(line) for line in lines]
        return [each for each in seen if each["message"]["notification"]["tag"] == tag]

    seen = pushed("coeus/test")
    assert sorted(each["path"] for each in seen) == ["/push/gone", "/push/phone"], seen
    for each in seen:
        assert each["key"] == public, each
        assert each["claims"]["sub"] == "https://coeus.example.com", each
        assert each["claims"]["aud"] == "http://127.0.0.1:8080", each
        assert each["ttl"] == "3600", each
        notification = each["message"]["notification"]
        assert notification["title"] == "coeus test \u00b7 from machine", notification
        assert notification["navigate"] == "https://coeus.example.com/", notification

    # the commit the machine caught up with is no system of it, so every try
    # fails to evaluate it. the first two say nothing, as the next might work;
    # the third ends the tries, and coeusd says so once from its rendered config
    machine.wait_until_succeeds("grep -q 'stopped catching up' /var/lib/fake-push/seen.jsonl")
    history = machine.succeed("coeus history loopback")
    seen = pushed("catch-up/machine")
    assert sorted(each["path"] for each in seen) == ["/push/gone", "/push/phone"], (seen, history)
    for each in seen:
        assert each["urgency"] == "high", each
        assert each["ttl"] == "86400", each
        notification = each["message"]["notification"]
        assert notification["title"] == f"machine stopped catching up to {rev[:7]}", notification
        assert " 3 times: " in notification["body"], notification
        assert notification["body"].endswith("tries again when deployed moves"), notification
        assert notification["navigate"].startswith("https://coeus.example.com/machines/machine/jobs/"), notification
  '';
}
