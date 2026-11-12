# what the machine is, read by `sh -s -- <login user>` over ssh. POSIX sh:
# it prints `key=value` lines and never stops half-way. every probe is
# guarded, and a key it cannot learn is printed empty.

login=${1:-$(id -un)}
say() { printf '%s=%s\n' "$1" "$2"; }
first() { head -n 1 2>/dev/null | tr -d '\r'; }

kernel=$(uname -s)
say kernel "$kernel"
say machine "$(uname -m)"
say hostname "$(uname -n)"
say loginUser "$login"
say hostKey "$(cat /etc/ssh/ssh_host_ed25519_key.pub 2>/dev/null | first)"
say timeZone "$(readlink /etc/localtime 2>/dev/null | sed -n 's|.*/zoneinfo/||p')"

nix=
for candidate in nix /nix/var/nix/profiles/default/bin/nix /run/current-system/sw/bin/nix; do
  if command -v "$candidate" >/dev/null 2>&1; then nix=$candidate; break; fi
done
say nixVersion "$([ -n "$nix" ] && "$nix" --version 2>/dev/null | first)"
say nixReceipt "$([ -e /nix/receipt.json ] && echo yes)"

case "$kernel" in
Linux)
  say osId "$(. /etc/os-release 2>/dev/null && echo "$ID")"
  say osName "$(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")"
  release=$(. /etc/os-release 2>/dev/null && echo "$VERSION_ID")
  say osVersion "$release"
  # what the stock installer wrote is what the running system was built
  # with; a release upgraded since keeps its older stateVersion
  stateVersion=$(sed -n 's/^[^#]*system\.stateVersion *= *"\([0-9.]*\)".*/\1/p' /etc/nixos/configuration.nix 2>/dev/null | first)
  say stateVersion "${stateVersion:-$release}"
  say efi "$([ -d /sys/firmware/efi ] && echo yes)"
  loader=
  if grep -q '"org.nixos.systemd-boot"' /run/current-system/boot.json 2>/dev/null; then
    loader=systemd-boot
  else
    for dir in /boot /efi /boot/efi; do
      if [ -e "$dir/loader/loader.conf" ]; then loader=systemd-boot; break; fi
      if [ -e "$dir/limine/limine.conf" ] || [ -e "$dir/limine.conf" ]; then loader=limine; break; fi
    done
    if [ -z "$loader" ] && [ -d /boot/grub ]; then loader=grub; fi
  fi
  say bootLoader "$loader"
  device=
  if [ "$loader" = grub ]; then
    device=$(sed -n 's/^[^#]*boot\.loader\.grub\.device *= *"\([^"]*\)".*/\1/p' /etc/nixos/configuration.nix 2>/dev/null | first)
    if [ -z "$device" ] && [ ! -d /sys/firmware/efi ]; then
      parent=$(lsblk -no pkname "$(findmnt -no source /boot 2>/dev/null || findmnt -no source /)" 2>/dev/null | first)
      [ -n "$parent" ] && device=/dev/$parent
    fi
  fi
  say bootDevice "$device"
  say cpus "$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN)"
  say memoryMib "$(sed -n 's/^MemTotal: *\([0-9]*\) kB/\1/p' /proc/meminfo | awk '{ print int($1 / 1024) }')"
  ;;
Darwin)
  say osVersion "$(sw_vers -productVersion 2>/dev/null)"
  say consoleUser "$(stat -f %Su /dev/console 2>/dev/null)"
  say cpus "$(sysctl -n hw.ncpu 2>/dev/null)"
  say memoryMib "$(sysctl -n hw.memsize 2>/dev/null | awk '{ print int($1 / 1048576) }')"
  ;;
esac
