# what the machine is, read by `sh -s` over ssh as the login user. POSIX sh,
# prints `key=value` lines and never stops half-way: every probe is guarded,
# and a key it cannot learn is printed empty.
#
# TODO, one key per line:
#   platform, system        uname -s / uname -m; /etc/os-release ID=nixos
#   osVersion               nixos-version, or sw_vers -productVersion
#   stateVersion            the running system's stateVersion, NixOS only
#   hostname                hostname
#   efi                     [ -d /sys/firmware/efi ]
#   bootLoader, bootDevice  bootctl is-installed, /boot/grub, limine
#   timeZone                readlink /etc/localtime
#   loginUser, consoleUser  id -un; stat -f %Su /dev/console on a Mac
#   cpus, memoryMib         nproc / sysctl -n hw.ncpu; /proc/meminfo / hw.memsize
#   nix                     nix --version; /nix/receipt.json without a working nix
#   hostKey                 cat /etc/ssh/ssh_host_ed25519_key.pub
