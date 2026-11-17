#!/usr/bin/env bash
# times the test suite's parts and what its slowest tests spawn, in place of `cargo test`
set -u
echo "PROBE cores: $(getconf _NPROCESSORS_ONLN) NIX_BUILD_CORES=${NIX_BUILD_CORES:-} nix: $(nix --version)"
cargo test --release --locked --no-run --message-format json-render-diagnostics > "$TMPDIR/build.json"
bin=$(jq -r 'select(.reason == "compiler-artifact" and .target.kind == ["lib"] and .profile.test) | .executable' "$TMPDIR/build.json")
span() { awk -v s="${1/,/.}" -v e="${2/,/.}" 'BEGIN { printf "%.1f", e - s }'; }
timed() {
  local label=$1; shift
  local s=$EPOCHREALTIME
  "$bin" -q "$@" > "$TMPDIR/out" 2>&1
  local c=$? e=$EPOCHREALTIME
  echo "PROBE $label: $(span "$s" "$e") s, exit $c, $(grep -o 'test result.*' "$TMPDIR/out" | head -1)"
}
kill=bootstrap::test::run::kill
timed "whole suite" 
timed "all but the kill tests" --skip "$kill::"
timed "the three kill tests together" "$kill::"
for t in a_run_stopped_anywhere_converges_when_run_again a_refused_switch_stopped_anywhere_converges_when_run_again a_run_dropped_while_it_waits_on_anything_converges_when_run_again; do
  timed "alone $t" --exact "$kill::$t"
done

shim=$TMPDIR/shim; mkdir -p "$shim"
for prog in git nix; do
  real=$(command -v "$prog")
  printf '#!%s\ns=$EPOCHREALTIME\n"%s" "$@"\nc=$?\nprintf "%%s\\t%%s\\t%s\\t%%s\\n" "$s" "$EPOCHREALTIME" "$*" >> "$SHIM_LOG"\nexit $c\n' "$BASH" "$real" "$prog" > "$shim/$prog"
  chmod +x "$shim/$prog"
done
export SHIM_LOG=$TMPDIR/shim.log
PATH=$shim:$PATH timed "alone, every git and nix logged, a_run_stopped_anywhere" --exact "$kill::a_run_stopped_anywhere_converges_when_run_again"
awk -F'\t' '{ gsub(",", ".", $1); gsub(",", ".", $2); n_w = split($4, w, " "); sub_ = ""; for (i = 1; i <= n_w; i++) { if (w[i] == "-C" || w[i] == "-c" || w[i] == "--extra-experimental-features") { i++; continue } if (w[i] !~ /^-/) { sub_ = w[i]; break } } k = $3 " " sub_; n[k]++; d[k] += $2 - $1; total += $2 - $1; count++ }
  END { printf "PROBE spawned %d processes, %.1f process-seconds\n", count, total; for (k in n) printf "PROBE   %-22s %5d runs %7.1f s %6.1f ms each\n", k, n[k], d[k], d[k] * 1000 / n[k] }' "$SHIM_LOG" | sort -k5 -nr

git init -q "$TMPDIR/repo" && cp src/bootstrap/test/run/contract.nix "$TMPDIR/repo/" && git -C "$TMPDIR/repo" add . && git -C "$TMPDIR/repo" -c user.name=t -c user.email=t@t commit -qm x
N=30 bash .github/probe/bench.sh "$TMPDIR/repo/contract.nix"
