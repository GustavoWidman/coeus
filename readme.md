# coeus

![rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)
![nix](https://img.shields.io/badge/nix-flake-5277C3?logo=nixos)
![platforms](https://img.shields.io/badge/runs_on-nixos%20%7C%20nix--darwin-lightgrey)

a deployment daemon for a nix fleet. every machine runs `coeusd`, and the `coeus` cli orders nixos and nix-darwin deployments to one machine, a handful, or all of them, then follows them live.

because ssh-ing into every machine to run a deploy script one at a time stops being fun around the third machine.

## the idea

you order, the machine deploys. `coeus deploy` hands each machine a pinned commit of your configuration repository (or a snapshot of your working tree) and returns as soon as the machine says it took the order. from there each `coeusd` fetches, evaluates, builds and switches on its own, so closing the laptop doesn't stop anything.

you look whenever you want. `coeus watch` shows every machine's progress live, whoever ordered the deployment: you, another terminal, or a github actions run. `coeus attach` follows one machine's build through nix-output-monitor, at your terminal's size, live or replayed from the start.

nobody has to babysit it. a push to `main` can roll out to the whole fleet, canary first. a machine that slept through it catches up when it wakes. a switch that leaves a machine unreachable rolls itself back. adding a host to the fleet is just enabling the module.

## commands

| command | what it does |
| ------- | ------------ |
| `coeus deploy [targets]` | order a deployment and follow it; `--ref`, `--rev`, `--local [dir]`, `--boot`, `--test`, `--build`, `--canary`, `--batch`, `-d` to detach |
| `coeus watch [targets]` | everything happening on the fleet, live |
| `coeus top [targets]` | a full-screen dashboard: attach, cancel, roll back, reboot or deploy from it |
| `coeus status [targets]` | what every machine runs, its generation, last job and catch-up state; `-w` keeps it live |
| `coeus attach <host> [job]` | a job's build through nom, then its activation; `--raw` for the bare log |
| `coeus logs <host> [job]` | a job's log as plain text; `-f` follows it |
| `coeus history [targets]` | past jobs: who ordered what, how it ended, how long it took |
| `coeus cancel <targets>` | stop queued or running jobs before they activate |
| `coeus rollback <host> [--to n]` | switch back to an earlier generation, as a job like any other |
| `coeus generations <host>` | the machine's system generations |
| `coeus reboot <host>` | reboot, once nothing is activating |
| `coeus doctor [targets]` | problems and how to fix them; `--fix` on a mac walks through the one permission it needs |
| `coeus keygen` | a new fleet key |
| `coeus completions <shell>` | completions for bash, zsh, fish and nushell |

targets are machine names, `@all`, `@<tag>` from the inventory, and `!name` or `!@tag` to leave some out. names outside the inventory are reached as tailnet hostnames. `status`, `history`, `generations` and `doctor` take `--json`, and `deploy` exits 0 when every job succeeded, 1 when one failed, and 2 when a machine could not be reached.

## setting a machine up

enable the module. the flake exports `nixosModules.default` and `darwinModules.default`:

```nix
services.coeus = {
  enable = true;
  keyFile = config.age.secrets.coeus-key.path; # `coeus keygen`, shared by the fleet
  flake = "git+https://github.com/you/nix";
  follow.ref = "deployed"; # optional: catch up with this ref on its own
  hosts = {
    # the cli's inventory, for @all and tags
    lab.tags = [ "nixos" "canary" ];
    altar.tags = [ "darwin" ];
  };
};
```

the first switch that enables it has to come from somewhere else, like `nixos-rebuild` or `darwin-rebuild`. from then on coeus deploys the machine, coeus itself included.

on nixos, `coeusd` runs as root under systemd and opens its port on `tailscale0` with `openFirewall`. on macos it runs as a root launchd daemon. macos asks for one thing by hand: nix-darwin can only create or delete users with full disk access, and macos only lets a person grant it. run `coeus doctor --fix` on the mac and drag `coeus-launcher` into the list it opens. routine deployments work without it.

## rollouts from github actions

the workflow joins the tailnet as `tag:ci` through a tailscale oauth client, then runs:

```sh
coeus deploy @all --rev "$GITHUB_SHA" --ref main --canary @canary --skip-unreachable
```

canaries go first, and the rest only follow if they succeed. a failed machine fails the run. when everything that answered succeeded, the workflow moves a `deployed` branch to the commit, and machines following `deployed` catch up when they come back online. give `tag:ci` access to port 45729 and nothing else.

## machines that are mostly off

`deployed` is the last commit the fleet took without a failure, so a machine that was off or asleep through any number of rollouts switches straight to it, without anyone ordering it:

- 30 seconds after `coeusd` starts, and right after the machine wakes up, it looks `deployed` up. while it can't reach the repository it looks again after 30 seconds, doubling the wait up to the follow interval.
- a catch-up that fails before building, which is where a network that comes and goes shows up, is tried again after 1 minute and then 2, three tries in all. one that fails while building or activating waits for `deployed` to move.
- shutting the machine down while it fetches or builds only pauses the catch-up: the job runs again after the next boot, keeping whatever it already built.
- a machine switched by hand after `deployed` last moved keeps what it runs until the fleet moves on.

## how a deployment runs

each job is a directory under `/var/lib/coeus/jobs` holding its order, an append-only log and, once finished, a summary. `coeusd` hands the job to a runner it starts through the service manager, a transient systemd unit or a transient launchd job, so the runner survives `coeusd` restarting, crashing or replacing itself with a newer version in the middle of a switch. the runner:

1. fetches the pinned commit with nix's own fetchers,
2. evaluates the machine's system and stops early when the machine already runs it,
3. builds it, logging nix's internal json so the progress bars and nom can read it,
4. diffs the closure against the running system,
5. switches, with `switch-to-configuration` on nixos or nix-darwin's `activate`,
6. checks the machine is still reachable, and switches back if it isn't.

a newer order replaces one that is still waiting, `--now` replaces one that hasn't started activating, and a `coeusd` that starts up finishes the bookkeeping of whatever it finds on disk.

## the protocol

`coeus` and `coeusd` talk over tcp encrypted with noise (`NNpsk0_25519_ChaChaPoly_BLAKE2s`) under the fleet's pre-shared key. packets are postcard-encoded and numbered by `#[packet_enum]`; every connection starts with a hello that names the machine and checks both ends speak the same protocol version.

## development

```sh
nix develop -c cargo test   # unit and end-to-end tests over the real protocol
nix flake check             # the package and clippy, plus a nixos vm running coeusd on linux
```
