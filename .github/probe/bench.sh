#!/usr/bin/env bash
# ms per run, over N sequential runs of each command
set -u
N=${N:-30}
c=$1
t() {
  local label=$1; shift
  local s=$EPOCHREALTIME
  for ((i = 0; i < N; i++)); do "$@" >/dev/null 2>&1; done
  local e=$EPOCHREALTIME
  awk -v s="${s/,/.}" -v e="${e/,/.}" -v n="$N" -v l="$label" 'BEGIN { printf "PROBE spawn %-40s %7.1f ms\n", l, (e - s) * 1000 / n }'
}
t "env true (process spawn)" env true
t "git --version" git --version
t "git -C repo rev-parse HEAD" git -C "$(dirname "$c")" rev-parse HEAD
t "nix --version" nix --version
t "nix eval --expr 1" nix --extra-experimental-features nix-command eval --expr 1
t "nix eval --expr 1, dummy store" nix --extra-experimental-features nix-command eval --store dummy:// --expr 1
t "nix eval contract head" nix --extra-experimental-features nix-command eval --json --file "$c" --apply 'c: { inherit (c) version identity; }'
t "nix hash path of the contract" nix --extra-experimental-features nix-command hash path "$c"
