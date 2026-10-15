# Sourced by the user's Home Manager activation, never by root on their behalf.
# source_config, config_home and coreutils are supplied as shell-escaped values.
coeus_install_client_config() (
  set -eu
  : "${source_config:?Missing Coeus client source path}"
  : "${config_home:?Missing XDG config home}"
  : "${coreutils:?Missing coreutils path}"
  if [ "${DRY_RUN+x}" = x ] || [ -n "${DRY_RUN_CMD:-}" ]; then
    echo 'Would privately install the Coeus client configuration'
    return 0
  fi
  actual=$("$coreutils/realpath" -e -- "$source_config")
  case "$actual" in
    /nix/store|/nix/store/*) echo 'Coeus client config must not resolve into the Nix store' >&2; return 1 ;;
  esac
  test -f "$actual" && test -r "$actual" || {
    echo 'Coeus client config must be a readable regular runtime file' >&2
    return 1
  }
  destination="$config_home/coeus"
  # Never write through a pre-existing Coeus directory/file symlink. Activation
  # runs with the user's authority; no root writes into user-controlled paths.
  test ! -L "$destination" && test ! -L "$destination/config.toml" || {
    echo 'Refusing a symlink at the Coeus client config destination' >&2
    return 1
  }
  umask 077
  "$coreutils/mkdir" -p -- "$destination"
  "$coreutils/chmod" 00700 -- "$destination"
  temporary=$("$coreutils/mktemp" "$destination/.config.toml.XXXXXXXX")
  trap '"$coreutils/rm" -f -- "$temporary"' EXIT
  trap 'exit 1' HUP INT TERM
  "$coreutils/install" -m 0600 -- "$actual" "$temporary"
  "$coreutils/mv" -fT -- "$temporary" "$destination/config.toml"
)
coeus_install_client_config
unset -f coeus_install_client_config
