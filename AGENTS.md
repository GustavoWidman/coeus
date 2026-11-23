# agents

notes for coding agents working in this repository. the readme explains how coeus works; this file says which checks to run when.

## the dashboard

the dashboard is `web/`, served by `coeus web`, which `src/web/` implements. `cargo run -- web --fixture instrument` serves it a simulated fleet from `src/web/fixture/`.

ci checks the dashboard's types, formatting, unit tests, bundle budget and css motion rule, and boots `coeus web` in a nixos vm that talks to it with curl. no check in ci opens a browser. two checks do, and they run only on demand, so run them yourself whenever a change can alter what a screen shows or what it costs: anything under `web/src/`, `web/static/` or `web/index.html`, the css, the stream and views in `src/web/`, or the fixture.

- `nix develop -c matrix-diff` photographs every state in `web/tests/matrix/states.ts` at `origin/main` and on this tree, and lists the screens that changed, with a picture of the difference in `web/test-results/matrix-diff/head/against/`. open each listed picture. every change on that list should be one you meant. the same run holds the tree to `web/matrix-budget.json`: layouts and style recalcs per stream message, paints at rest, dom nodes and their growth, layers, the heap, long tasks, animations that move anything but `transform`, `opacity` or `clip-path`, and animations that still run under reduced motion. a failure there is a regression. on a machine without a gpu, add `--no-timings`, since dropped frames and gpu memory would measure the cpu.
- `nix develop -c e2e` runs the end-to-end tests: live updates, navigation, signing in, offline, the log sheet and diffs, on a webkit phone and a chromium desktop.

a new screen or state needs an entry in `web/tests/matrix/states.ts`, so the matrix can reach it. narrow a run while iterating with `--only screen=fleet` (or `state=`, `theme=`, `viewport=`, `motion=`, `engine=`), then run the whole matrix once before you finish.

the matrix counts long tasks in wall time, so run it with nothing else busy on the machine. compare screenshots only from runs made on the same machine, since another cpu draws some pixels differently. `matrix-diff` does that by itself.

`web/src/lib/wire/types.ts` and `contract/web.json` are generated from `src/web/view.rs`. after changing those types, rewrite both with `COEUS_BLESS=1 nix develop -c cargo test web::`.
