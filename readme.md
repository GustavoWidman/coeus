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
| `coeus catch-up <targets>` | have machines look up the ref they follow now, and catch up if it moved; `--skip-unreachable` |
| `coeus rollback <host> [--to n]` | switch back to an earlier generation, as a job like any other |
| `coeus generations <host>` | the machine's system generations |
| `coeus reboot <host>` | reboot, once nothing is activating |
| `coeus bootstrap <host> --to <user@address>` | bring a machine that runs nixos, or a mac, into the fleet; without a host, list the bootstraps in progress |
| `coeus doctor [targets]` | problems and how to fix them; `--fix` on a mac walks through the one permission it needs |
| `coeus web` | serve the fleet's dashboard, live, to browsers signed in on the tailnet; `--fixture instrument` serves a simulated fleet instead |
| `coeus keygen` | a new fleet key |
| `coeus completions <shell>` | completions for bash, zsh, fish and nushell |

targets are machine names, `@all`, `@<tag>` from the inventory, and `!name` or `!@tag` to leave some out. names outside the inventory are reached as tailnet hostnames. a machine the local tailscale sees offline counts as unreachable at once, with the time tailscale last saw it, instead of after the connect timeout. `status`, `history`, `generations` and `doctor` take `--json`, and `deploy` exits 0 when every job succeeded, 1 when one failed, and 2 when a machine could not be reached. `catch-up` exits 2 when a machine could not be reached or could not look its ref up.

## setting a machine up

enable the module. the flake exports `nixosModules.default` and `darwinModules.default`:

```nix
services.coeus = {
  enable = true;
  keyFile = config.age.secrets.coeus-key.path; # `coeus keygen`, shared by the fleet
  readKeyFile = config.age.secrets.coeus-read-key.path; # optional: a key that may only watch
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
git push --force origin "$GITHUB_SHA:refs/heads/deployed"
coeus catch-up @all --skip-unreachable
```

canaries go first, and the rest only follow if they succeed. a failed machine fails the run. when everything that answered succeeded, the workflow moves a `deployed` branch to the commit, and machines following `deployed` catch up when they come back online. `coeus catch-up` has every machine that answers look `deployed` up at once: without it, a machine that took the commit by order shows it holding `deployed` until its next scheduled look. give `tag:ci` access to port 45729 and nothing else.

## machines that are mostly off

`deployed` is the last commit the fleet took without a failure, so a machine that was off or asleep through any number of rollouts switches straight to it, without anyone ordering it:

- 30 seconds after `coeusd` starts, right after the machine wakes up, and whenever `coeus catch-up` asks, it looks `deployed` up. while it can't reach the repository it looks again after 30 seconds, doubling the wait up to the follow interval.
- a catch-up that fails before building, which is where a network that comes and goes shows up, is tried again after 1 minute and then 2, three tries in all. one that fails while building or activating waits for `deployed` to move.
- shutting the machine down while it fetches or builds only pauses the catch-up: the job runs again after the next boot, keeping whatever it already built.
- a machine switched by hand after `deployed` last moved keeps what it runs until the fleet moves on.

every successful lookup, scheduled or asked for, rewrites `/run/coeus/follow.json` (`/var/run/coeus/follow.json` on macos, `follow.stateFile` in the module, `null` to turn it off), readable by every user and replaced whole, so a program on the machine can act on what the whole fleet has rolled out:

```json
{ "ref": "deployed", "rev": "<40 hex digits>", "committed": 1794942959, "checked": 1794943200 }
```

`committed` is the commit's time, exactly the flake's `lastModified`, and `checked` when the lookup succeeded, both in unix seconds. a failed lookup leaves the file as it was, so `checked` only ages while the repository is out of reach, and a reader should stop trusting a file it last checked long ago. a write that fails is logged and changes nothing else. a dns reconciler like norn, for example, deletes a record shared by the fleet only once `committed` is at least the `lastModified` of the configuration that dropped it.

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

a daemon can accept a second key, `read_key_file` (`readKeyFile` in the module), for things that should see the fleet and never touch it. the first noise message only decrypts under the key the client used, so coeusd tries the fleet key, then the read-only one, and the one that opens it decides what the connection may ask. a read-only connection may watch, read history and generations, and attach to a log as plain frames. everything else gets `this key may only read` and the connection stays open: deploy, cancel and reboot obviously, but also resolving a ref, which makes the daemon fetch git, catching up, which can start a job, uploads, and a drawn attach, which spawns nix-output-monitor on the machine. `Role::allows` names every packet with no wildcard, so a new one does not compile until someone decides. coeusd refuses to start when both keys are the same.

## the dashboard

`coeus web` watches every machine of its inventory with the read-only key and keeps one copy of the fleet, built by a single task from each machine's views. every machine, running build and deployment in it carries the number of the change that last touched it. it reads `/etc/coeus/web.toml` (`web.example.toml` shows it), serves the built app from `--web-dir` (`COEUS_WEB_DIR`) and everything else under `/api`, on a unix socket only its own user can open, for cloudflared. anyone gets the app and `/api/hello` (where to sign in, the build, the schema); everything else under `/api` is an empty 401 without a session.

signing in is a separate site, served over tls with the machine's own `*.ts.net` certificate on its tailnet addresses alone, so nothing that comes through cloudflare can reach it, whatever it carries. the app, loaded from the public origin, makes a p-256 key pair the browser won't export and posts its public half to `/auth/session`, signed. wireguard vouches for the caller's tailnet address, `tailscale whois` names the login and node behind it (any local user may ask; no operator needed), and a node gets a session when the tailnet policy grants it the app capability (`r3dlust.com/cap/coeus` unless `signin.capability` says otherwise) toward the serving host, or, for a user's node, when its login is in `signin.logins`. a tagged node has no login, so only a grant lets it in. `/auth/ping` answers 204, which is how the app knows it is on the tailnet. both routes let in the public origin alone, private network access preflights included.

a session is a token every serving host checks on its own: an hmac, under the fleet's session key, over the login, the node, the browser's public key and when it ends, a week after signing in. the key file holds one or more keys, 64 hex digits a line; the first signs and all of them check, so rotating means putting a new key first and dropping the old one a week later, or right away to end every session. every request also carries `Coeus-Proof: <ms>.<signature>`, the private key's signature over the method, path and query, that moment and the token's hash. a copied token is useless without the browser that holds the key, and a captured proof replays only within two minutes, on the same request, which needs no nonces shared between hosts. a stream ends when its session does, with an `expired` event. signing in again on the tailnet makes a new one, so off the tailnet a session runs out a week after the last sign-in.

`/api/stream` is server-sent events: a snapshot, then each thing again, whole, whenever it changes, with `<epoch>.<number>` as the id. come back with the last id as `Last-Event-ID` and you get only what changed since, unless the process restarted, or you were gone so long it forgot a removal, in which case you get a snapshot. a browser is nothing but that id, so one that stops reading costs no memory: when it reads again it gets the latest of each thing once, not everything it missed. a build's numbers go out at most every 250 ms, the rest within 50 ms. `/api/machines/<host>/jobs/<id>/log` streams a job's log as typed lines, its ids the machine's own log offsets, so a reconnect resumes on any host; `/api/machines/<host>/jobs/<id>/diff` is the job's closure diff.

a serving host connects to the cloudflare tunnel in `[settings.web.tunnel]` only while it can serve. every 2 s it checks that tailscale runs, holds its long poll to the coordination server and warns about neither that server nor its relays, and that its own coeusd shows its view to the read-only key on 127.0.0.1. three passes in a row bring the tunnel up and five fails in a row take it down, so a coeusd restarting into a new version, which listens again within tens of milliseconds, never bounces it. no other machine is asked, so the last host standing keeps serving. cloudflared runs as coeus web's child, `cloudflared tunnel --config cloudflared.json --protocol quic run` with a 10 s grace period, the config written beside the origin socket: the tunnel, its credentials, and the socket as its only ingress (`--url unix:` would carry raw tcp over websockets instead). leaving the tunnel sends it SIGTERM, then SIGKILL 15 s later; one that dies while the host is ready comes back after 1, 2, 5, then 10 s. it stays in coeus web's process group, so systemd's control group and launchd's process group (with `AbandonProcessGroup` false) take it down with coeus web however that dies.

on a serving host the module runs it, beside `readKeyFile` and the `hosts` it shows, in order:

```nix
services.coeus.web = {
  enable = true;
  publicOrigin = "https://coeus.example.com";
  # 64 hex digits a line (`coeus keygen`), the same file on every serving host
  sessionKeyFile = config.age.secrets.coeus-session-key.path;
  # every serving host's, so the app finds whichever answers
  signin.urls = [ "https://lab.tail0000.ts.net:45730" "https://altar.tail0000.ts.net:45730" ];
  tunnel = {
    id = "6f1c2a9e-3b7d-4e58-9a0c-d2e4f6a8b1c3"; # `cloudflared tunnel create`
    credentialsFile = config.age.secrets.coeus-tunnel.path;
  };
};
```

on nixos it is `coeus-web.service`, running as the `coeus-web` system user in a tight sandbox, which cloudflared and the tailscale cli it runs share. it gets the read key, the session key and the tunnel's credentials as systemd credentials, so they can stay root's, and keeps the origin socket and cloudflared's config in `/run/coeus-web`. the credentials are linked from `/run/credstore`, where systemd leaves out one whose file is missing instead of failing the start, and coeus web then names the file in the journal and tries again a minute later, so a secret that isn't there yet never fails the switch. `coeus-web-cert` asks tailscaled for the machine's `*.ts.net` certificate when coeus web starts and every hour after, which needs https certificates enabled for the tailnet, and coeus web picks a renewed one up without restarting. `signin.certFile` and `signin.keyFile` point it at one you already have instead. the sign-in port opens on the tailscale interface alone. on macos it is a launchd daemon running as `_coeusweb` (uid 471, `services.coeus.web.uid`), which must be able to read the files it is given, agenix's `owner`. there it keeps everything in `/var/lib/coeus-web`, since `/var/run` empties at boot and only root could make its directory again. either way it logs at info, to the journal or `/var/log/coeus-web.log`, so the host becoming ready, and the tunnel coming and going, show there.

`coeus web --fixture instrument` runs the same site over a simulated fleet on `127.0.0.1:7979`, with sign-in on that same address taking anyone as the fixture's viewer under a session key made up per run (`--open` skips signing in): the design prototype's six machines in the middle of a rollout, a failed build, a laptop that wakes to catch up, and then it all starts over (about 108 s). it never dials a coeusd. one seeded generator draws every number, so a moment looks the same however the clock got there. `POST /fixture/seek?t=<seconds>` moves the clock there and holds it, `&speed=1` runs it from there, and the answer, which comes once the site shows that moment, says what time it is in the simulated fleet, for a test to pin the browser's clock to.

## development

```sh
nix develop -c cargo test   # unit and end-to-end tests over the real protocol
nix flake check             # the package and clippy, plus nixos vms running coeusd and coeus web on linux
```

the dashboard lives in `web/`, a svelte 5 app that vite builds into static files. `bun run dev` serves it with hot reload, talking to `cargo run -- web --fixture instrument`, and `nix build .#coeus-web` builds what ships. its checks hold what `index.html` loads to `web/perf-budget.json` (gzipped js and css) and the built css to animating only `transform`, `opacity` and `clip-path`. after changing its dependencies, run `nix develop -c lock-web` so nix fetches the same ones.

`web/tests/matrix/states.ts` lists every state the dashboard can be in, each as a moment on the simulated fleet's clock, a page to open, and what the network answers or a person does before the screenshot. states the simulation never reaches, like a host that refuses the read key, rewrite the stream on its way into the page. `nix develop -c matrix` builds coeus and the app and runs every state in light and dark, on a 390x844 phone and a 1440x900 desktop, with full and reduced motion. each worker starts its own `coeus web --fixture instrument` and its own browsers. chromium opens every cell and webkit the phone ones, and each cell gets a screenshot, 2 s of the page at rest and, where the state moves, 10 s of live updates. chromium reports long tasks, dropped frames, layouts, style recalcs, paints and the area they cover, main-thread and raster time, composited layers, dom nodes, the heap and the page's gpu memory. every animation the page starts has to move only `transform`, `opacity` or `clip-path`, the same rule the css check holds. the run writes `web/test-results/matrix/` and fails on anything over `web/matrix-budget.json`. `--only screen=fleet` (or `state=`, `theme=`, `viewport=`, `motion=`, `engine=`) narrows it, `--trace` keeps the traces for perfetto, `--repeat 5` runs each live window five times and keeps the median, since a laptop drops a frame now and then all by itself, and `--reference <dir>` runs the same states in the design prototype too and writes how far each screenshot is from its prototype twin, with a picture of the difference.

`nix flake check` runs the matrix on x86_64-linux in the build sandbox. there is no gpu there, so frame timing and gpu memory would measure the cpu, and it checks only the counts, plus every screenshot against its hash in `web/tests/matrix/baselines/linux-x64.json`. a cell whose screen can't be reached yet, say a button that doesn't exist, records why, and that is its baseline until the button shows up. after changing how a screen looks, `nix develop -c matrix-bless` runs the matrix there through a linux builder, prints where its screenshots landed, and rewrites the hashes. look at the screenshots first. the hashes will agree with anything.

the web dashboard's types live in `src/web/view.rs`. `web/src/lib/wire/types.ts` and `contract/web.json`, an example of every message, are generated from them, and the tests fail until both are rewritten with `COEUS_BLESS=1 nix develop -c cargo test web::`.

when every serving host is down, cloudflare answers the dashboard's address with its own error 1033 page, and the free plan can't swap it for yours. `worker/` is a cloudflare worker for the dashboard's route that can. it passes every request through untouched, and when the tunnel answers 530 or the origin can't be reached, a page load gets coeus's own offline page (a 503 that checks again every 15 s and reloads once a host is back) and anything else gets `503 {"offline":true}`. the page splices in the app's `web/src/lib/ui/tokens.css`, so it can't drift from the dashboard's colors. `lib.offlineWorker { hosts = [ ... ]; }` builds it into one es module with `builtins.toFile`, which gives it the same store path on every system, as norn's `cloudflare.fleet.workers.<name>.module` requires of every peer. `nix build .#offline-worker` builds it for the fleet's servers, and `nix flake check` runs `worker/worker.test.ts` against that build.

every pull request checks the flake on every system the fleet runs: `nix flake check` on x86_64-linux, the vm tests included, and on aarch64-darwin, where a nix-darwin system serving the dashboard evaluates, and every check but the vm tests on aarch64-linux. its commits must follow conventional commits. the version lives in `Cargo.toml`, with `Cargo.lock` agreeing, and a pull request must raise it unless labeled `no-release`. a push to `main` that brings a version with no release yet builds coeus for x86_64-linux, aarch64-linux and aarch64-darwin, pushes it to `https://cache.r3dlust.com` (key `cache.r3dlust.com-1:Wdkf9mes2ued0P7C7CL3Jax1b4KPuJwQpM3h4G/Z+yI=`) pinned as `coeus-<system>`, then tags it `v<version>` and releases it. a push that releases nothing builds nothing. keep the input's own `nixpkgs` (no `follows`) and a machine gets that exact build from the cache.
