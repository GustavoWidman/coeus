# a script that copies this machine's *.ts.net certificate from tailscaled into
# `dir`, as `cert.pem` (with its chain) and `key.pem`, for `owner` alone.
# tailscaled orders it from let's encrypt the first time and renews it near its
# expiry; until then `tailscale cert` hands back the one it holds, so running
# this often costs nothing. it always exits 0: a machine without a certificate
# yet is not a failed unit, and coeus web keeps trying to load one.
{
  lib,
  pkgs,
  tailscale,
  owner,
  dir,
}:
pkgs.writeShellScript "coeus-web-cert" ''
  PATH=${
    lib.makeBinPath [
      pkgs.coreutils
      pkgs.diffutils
      pkgs.jq
    ]
  }:$PATH

  name=$(${tailscale} status --json --peers=false 2>/dev/null | jq -r '.Self.DNSName // "" | rtrimstr(".")' 2>/dev/null)
  if [ -z "$name" ]; then
    echo "tailscale names no tailnet machine here yet; trying again later"
    exit 0
  fi

  new=$(mktemp -d)
  trap 'rm -rf "$new"' EXIT
  if ! ${tailscale} cert --cert-file "$new/cert.pem" --key-file "$new/key.pem" "$name"; then
    echo "tailscale issued no certificate for $name; trying again later"
    exit 0
  fi

  install -d -m 0700 -o ${owner} -g ${owner} ${dir}
  # renamed into place, so coeus web never reads half a file
  for file in key.pem cert.pem; do
    cmp -s "$new/$file" "${dir}/$file" && continue
    install -m 0600 -o ${owner} -g ${owner} "$new/$file" "${dir}/.$file.new"
    mv -f "${dir}/.$file.new" "${dir}/$file"
    echo "wrote $file for $name"
  done
''
