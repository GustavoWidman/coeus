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
| `coeus bootstrap <host> --to <user@address>` | bring a machine that runs nixos, or a mac, into the fleet; without a host, list the bootstraps in progress |
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

the first switch that enables it comes from `coeus bootstrap` (below), or from somewhere else, like `nixos-rebuild` or `darwin-rebuild`. from then on coeus deploys the machine, coeus itself included.

on nixos, `coeusd` runs as root under systemd and opens its port on `tailscale0` with `openFirewall`. on macos it runs as a root launchd daemon. macos asks for one thing by hand: nix-darwin can only create or delete users with full disk access, and macos only lets a person grant it. run `coeus doctor --fix` on the mac and drag `coeus-launcher` into the list it opens. routine deployments work without it.

## bringing a machine in

```sh
coeus bootstrap nova --to nixos@192.168.1.40
```

run from your checkout of the configuration repository on an admin machine (one whose identity opens every secret), it takes a machine that already runs nixos, or a mac, all the way into the fleet:

1. **reach**: ssh to the machine, show the host key it presents so you can compare it on its console, pin it, learn what the machine is, and put a per-machine session key in root's `authorized_keys`, so a run keeps root after the first switch removes the stock user
2. **nix**: on a mac without it, install determinate nix, and move aside the `/etc` shell files nix-darwin would refuse to replace
3. **enroll**: ask the repository's questions, write the machine's files and make its own secrets in a private clone (`~/.local/state/coeus/bootstrap/<host>/repo`, so your checkout is never touched), and commit them on `coeus/bootstrap/<host>`
4. **rekey**: move the branch onto the base branch's head and re-encrypt every secret whose recipients changed, reading the admin's identity once through `sudo`
5. **push**: push the branch and open a draft pull request
6. **switch**: ship the clone, and the inputs only the admin can reach, into the machine's store and switch it with the coeus its own configuration pins, as an ordinary job guarded like any deployment: a switch that fails or leaves the machine unreachable rolls itself back. when nixos refuses to switch live (a pre-switch check, such as a new dbus implementation), the run boots the system instead and reboots into it, as nixos asks. on a mac, nix-darwin's first activation writes where only full disk access may, which only a person at the mac can grant: the run opens system settings on its screen and waits for the grant
7. **answer**: wait for its `coeusd` on the tailnet
8. **reboot**: offer one when it runs a kernel it did not boot
9. **review**: write what happened into the pull request and mark it ready
10. **cleanup**: take the session key out again

merging the pull request is what tells every other machine about the new one. stop it whenever you like: ctrl-c, a dropped connection, a dead laptop. a switch keeps going on the machine, and running `coeus bootstrap nova` again picks up from whatever it finds: it keeps no record of its progress, only how to reach the machine, your answers and what the machine was before its first switch. a second run of the same machine while one is going is refused, naming the first one's pid.

| option | |
| ------ | - |
| `--to <destination>` | how ssh reaches the stock machine; an alias of your ssh config works. saved for later runs |
| `--repo <dir>` | your checkout, the current directory by default. saved |
| `--base <branch>` | the branch of origin to enroll onto and open the pull request against, origin's default branch by default. saved |
| `--answer <id>=<value>` | answer a question ahead of time; without a terminal, every question needs one |
| `--identity <file>` | the identity that opens every secret, the contract's by default |
| `--host-key SHA256:…` | trust the key with this fingerprint without asking |
| `--retry` | order the switch of the same commit again after it failed; otherwise a failed switch waits for a fix on the branch |
| `--forget` | take the session key out of the machine if it can, and drop everything the admin keeps of this bootstrap; the branch and pull request stay |

`coeus bootstrap` with no host lists the bootstraps in progress and how each last ended. it exits 0 once the machine is in, 1 when a step failed or waits for you, 2 when the machine could not be reached and 130 when you detached. coeus adopts machines only: it never partitions or wipes a disk, and never pushes to the base branch.

the coeus the machine's configuration pins must know `coeusd adopt`, which admits the first switch while no `coeusd` runs yet.

## the bootstrap contract

coeus knows no repository layout. what a new machine adds, what to ask about it and who opens each secret come from a plain nix file at `.coeus/bootstrap.nix`, evaluated with builtins only:

```sh
nix eval --json --file .coeus/bootstrap.nix --apply '<function>'
```

so the file evaluates with no flake, no fetched input and no `--impure`. this is schema version 1.

```nix
{
  version = 1;
  identity = "/etc/ssh/ssh_host_ed25519_key"; # what an admin opens every secret with
  questions = input: [ question ];
  enroll = input: enrollment;
  recipients = path: [ "ssh-ed25519 AAAA… comment" ] or null;
}
```

**the input** every function gets is `{ name, hostKey, facts, answers }`:

- `name`: the machine's name in the fleet
- `hostKey`: the pinned ed25519 host key, `"ssh-ed25519 AAAA…"`, with no comment
- `answers`: question id to answer, every value a string (a choice's value, `"true"` or `"false"`, or the text). questions not answered yet are absent
- `facts`, the machine as its stock system showed it:

| fact | |
| ---- | - |
| `platform` | `"nixos"` or `"darwin"` |
| `system` | `"x86_64-linux"`, `"aarch64-linux"`, `"aarch64-darwin"` or `"x86_64-darwin"` |
| `osVersion` | the nixos release (`"26.05"`) or the macos version (`"26.6.2"`) |
| `stateVersion` | the running system's `system.stateVersion`; `null` on a mac |
| `hostname` | what the machine calls itself now |
| `efi` | a bool |
| `bootLoader` | `"systemd-boot"`, `"grub"`, `"limine"` or `null` |
| `bootDevice` | grub's device, or `null` |
| `hardwareConfig` | `nixos-generate-config --show-hardware-config`; `null` on a mac |
| `timeZone` | an iana name, or `null` |
| `loginUser` | the user ssh logged in as |
| `mainUser` | on a mac the console user; on nixos the login user unless that is root, else `null` |
| `cpus`, `memoryMib` | integers |
| `nixVersion` | `nix --version`'s version, or `null` |

**`questions input`** is a list of `{ id; prompt; kind; choices ? [ { value; label; help ? null; } ]; default ? null; pattern ? null; }`, `kind` being `"choice"`, `"confirm"` or `"text"`. coeus asks the first question whose id is not in `answers`, then calls `questions` again with the answer added, so a question may depend on an earlier one. a confirm's default is `true`, `false`, `"true"` or `"false"`; a text's `pattern` is a regular expression its answer must match whole. answers are saved as they come and never asked twice.

**`enroll input`** says what the machine adds:

```nix
{
  files = { "hosts/nova/flake.nix" = "…"; };     # written when absent, never edited
  secrets = { "hosts/nova/password.age" = { kind = "password"; prompt = "…"; }; };
  firstSwitch.installBootloader = true;          # NIXOS_INSTALL_BOOTLOADER=1 on the first switch
  messages = {
    enroll = "feat(nova): join the fleet";       # the enroll commit's subject
    rekey = "chore(secrets): rekey for nova";    # the rekey commit's subject
    title = "feat(nova): join the fleet";        # the pull request's title
    body = "…\n";                                # the pull request's body above coeus's own section
  };
  notes = [ "what only a person can do" ];       # said when the run ends
}
```

paths are relative to the repository, normalized, and never under `.git`; a file that is on the base branch already means the machine is enrolled there, and the run stops. a secret is made only when its file is absent, by one of four generators, in memory, and encrypted straight to its recipients:

| `kind` | makes |
| ------ | ----- |
| `password` | a password typed twice (`prompt`), stored as a sha512-crypt hash, which `hashedPasswordFile` takes |
| `ssh-ed25519` | an openssh ed25519 private key (`comment`) |
| `random` | `bytes` random bytes (32 by default), as lowercase hex |
| `text` | a value typed once (`prompt`), such as a token |

a password or a text needs a terminal.

**`recipients path`** returns the ssh keys that must open a `.age` file, comments allowed, or `null` for one the repository does not manage. coeus asks for every tracked secret in one evaluation, rekeys only those whose age header names other keys than these, and asks for a new machine's secrets after writing its files, so the rules may read the machine's own `host.pub`. agenix's `secrets.nix` can derive its rules from the same function, so the two never disagree.

**errors.** a contract that refuses (`throw "write hosts/nova/boot.nix by hand"`, a missing answer, a name it does not take) is shown in its own words, and the run stops at the step that asked. output with an attribute the schema does not name is refused, so a typo fails loudly. a repository without the file, or with another `version`, is refused saying whether coeus or the contract needs updating.

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

every pull request checks the flake on every system the fleet runs: `nix flake check` on x86_64-linux, the vm test included, and on aarch64-darwin, and every check but the vm test on aarch64-linux. its commits must follow conventional commits. the version lives in `Cargo.toml`, with `Cargo.lock` agreeing, and a pull request must raise it unless labeled `no-release`. a push to `main` that brings a version with no release yet builds coeus for x86_64-linux, aarch64-linux and aarch64-darwin, pushes it to `https://cache.r3dlust.com` (key `cache.r3dlust.com-1:Wdkf9mes2ued0P7C7CL3Jax1b4KPuJwQpM3h4G/Z+yI=`) pinned as `coeus-<system>`, then tags it `v<version>` and releases it. a push that releases nothing builds nothing. keep the input's own `nixpkgs` (no `follows`) and a machine gets that exact build from the cache.
