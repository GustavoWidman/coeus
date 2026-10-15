"""Strict argv policy for nh 4.4.2 activation. No caller environment is forwarded."""
import os
import re
import sys

PROFILE = "/nix/var/nix/profiles/system"
STORE_NAME = re.compile(r"[0-9abcdfghijklmnpqrsvwxyz]{32}-[^/]+")


def closure(path, platform, store="/nix/store"):
    if not os.path.isabs(path) or any(part in (".", "..") for part in path.split("/")):
        raise ValueError("expected absolute closure path without traversal")
    target = os.path.realpath(path, strict=True)
    if os.path.dirname(target) != store or not STORE_NAME.fullmatch(os.path.basename(target)):
        raise ValueError("expected a realised Nix store system closure")
    marker = "bin/switch-to-configuration" if platform == "linux" else "sw/bin/darwin-rebuild"
    program = os.path.join(target, marker)
    if (not os.path.isfile(program) or not os.access(program, os.X_OK)
            or not os.path.realpath(program, strict=True).startswith(store + "/")):
        raise ValueError("missing system activation program")
    return target


def validate(argv, platform, nix, store="/nix/store"):
    if len(argv) == 6 and argv[0] in ("nix", nix) and argv[1:5] == [
        "build", "--no-link", "--profile", PROFILE
    ]:
        return [nix, *argv[1:5], closure(argv[5], platform, store)]
    if platform == "linux" and len(argv) == 3 and argv[1:] == ["restart", "coeusd.service"] and argv[0].endswith("/sw/bin/systemctl"):
        target = closure(argv[0][:-len("/sw/bin/systemctl")], platform, store)
        if os.path.realpath("/run/current-system", strict=True) != target:
            raise ValueError("service restart requires the active exact closure")
        return [target + "/sw/bin/systemctl", "restart", "coeusd.service"]
    suffix = "/bin/switch-to-configuration" if platform == "linux" else "/sw/bin/darwin-rebuild"
    actions = ("test", "boot") if platform == "linux" else ("activate",)
    if len(argv) == 2 and argv[0].endswith(suffix) and argv[1] in actions:
        target = closure(argv[0][:-len(suffix)], platform, store)
        return [target + suffix, argv[1]]
    raise ValueError("unsupported nh activation command")


def envelope(argv):
    if not argv or argv[0] != "env" or len(argv) > 256:
        raise ValueError("expected bounded nh env envelope")
    index = 1
    while index < len(argv) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", argv[index], re.DOTALL):
        if len(argv[index]) > 65536:
            raise ValueError("oversized nh environment assignment")
        index += 1
    if index == len(argv):
        raise ValueError("missing nh command")
    return argv[index:]


def main():
    mode, platform, nix, sudo, helper, path = sys.argv[1:7]
    try:
        if mode == "adapter":
            args = envelope(sys.argv[7:])
            return os.execv(sudo, [sudo, "-n", "--", helper, *args])
        if mode != "helper" or os.geteuid() != 0:
            raise ValueError("activation helper must run as root")
        args = validate(sys.argv[7:], platform, nix)
        env = {"HOME": "/var/root" if platform == "darwin" else "/root",
               "USER": "root", "LOGNAME": "root", "PATH": path,
               "NIX_REMOTE": "daemon"}
        # Set by sudo, not supplied through nh's untrusted env envelope.
        if "SUDO_USER" in os.environ:
            env["SUDO_USER"] = os.environ["SUDO_USER"]
        os.execve(args[0], args, env)
    except (ValueError, OSError) as error:
        print(f"coeus elevation: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
