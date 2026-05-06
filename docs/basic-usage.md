---
source: (our own — derived from /home/li/git/lojix-cli/{ARCHITECTURE,README}.md and field deploys 2026-04-27..28)
fetched: 2026-04-28
scope: how to use lojix-cli to build / deploy / inspect CriomOS systems; not the lojix-cli internals (those live in ARCHITECTURE.md)
---

# lojix-cli basic usage

## Use `lojix-cli`, not `lojix-cli-v2`

`~/git/lojix-cli-v2` is a dev fork where the next-generation
nota-native CLI and home-deploy work is being staged. **It is not
usable yet.** All deploy / build / eval flows go through the v1
`lojix-cli` binary documented here. When v2 ships, this note moves.

## The one concept that matters

`lojix-cli` projects a **cluster proposal nota** (e.g. `goldragon/datom.nota`)
through `horizon-rs` into a content-addressed `horizon` flake, then invokes
`nixos-rebuild` against `CriomOS` with `--override-input horizon ...`. The
content-addressing means *the same proposal → the same narHash → cache
hits across machines*. The agent's job is to pick an action and pass the
right cluster + node + source.

## The five actions

`lojix-cli deploy --action <X>` (or `lojix-cli {build,eval}`) — `X` is one of:

| action | what it does | needs root | invokes | when |
|---|---|---|---|---|
| **`eval`** | print the toplevel's `drvPath` | no | `nix eval --raw` | check eval-cache reuse / debug projection |
| **`build`** | build the toplevel, no profile change | no | `nix build --no-link --print-out-paths` | pre-warm a remote cache; verify compilation |
| **`test`** | activate live, do **not** touch boot loader (rebooting reverts) | yes | `nixos-rebuild test` | safest "try it and roll back via reboot" path *— but blocked on the new criomos by the dbus → dbus-broker switch inhibitor; use `boot` instead* |
| **`boot`** | write the bootloader entry, do **not** activate live | yes | `nixos-rebuild boot` | the canonical deploy: reboot to land on it; systemd-boot menu lets you fall back to the previous gen if the new one breaks |
| **`switch`** | both: activate live + write bootloader | yes | `nixos-rebuild switch` | only when you're confident `test` would succeed *and* you're happy with persistence in one go |

Default with `lojix-cli deploy` (no `--action`) is `switch`. **Prefer `boot`
explicitly** for any first-touch deploy on a new criomos generation.

## Canonical invocation

```bash
lojix-cli deploy --cluster goldragon --node ouranos \
  --source /home/li/git/goldragon/datom.nota \
  --action boot \
  --criomos github:LiGoldragon/CriomOS/<rev>
```

- `--cluster` / `--node` — keys into the proposal; the projector emits
  the `(cluster, node)`-shaped horizon.
- `--source` — path to the cluster proposal nota.
- `--criomos` — flake ref to consume. **Always pin a rev** (see below).
- `--action` — overrides the default for the subcommand.

## Always pin `--criomos github:LiGoldragon/CriomOS/<rev>`

Without a rev, nix's eval cache returns *whatever was previously
evaluated* for `github:LiGoldragon/CriomOS` even after you've pushed
new commits. Pinning the rev forces a fresh fetch + eval. Get the rev
with `git -C ~/git/CriomOS rev-parse main` or from the latest commit
in the `gh repo view` output.

If you change CriomOS source, push, then forget to bump the rev —
you'll deploy stale code with no warning. The rev is the only
trustworthy signal.

## Privilege: ssh root@localhost is built in

`eval` and `build` run unprivileged in the user shell.

`test`, `boot`, and `switch` automatically re-invoke the `nixos-rebuild`
tail through `ssh -o BatchMode=yes root@localhost <quoted command>`.
**Do not run `lojix-cli` with `sudo`.** It already escalates the parts
that need root.

For this to work the user must have an SSH key allowed to log into
`root@localhost`. CriomOS sets this up for the primary user.

The user-side projection still writes to `~/.cache/forge/...`; the
ssh-root step's nix-daemon resolves the same `/nix/store` (shared)
and reads the user's cache files via the `--override-input
horizon|system path:...` paths. So local builds done as the user
benefit ssh-root activations downstream — no cache duplication.

## Recovery — systemd-boot menu fallback

Every `boot` / `switch` deploy keeps the previous generation in the
systemd-boot menu. If the new gen fails to come up cleanly:

1. Reboot.
2. At the systemd-boot menu, select the previous `nixos-generation-NN.conf`.
3. Diagnose from there (logs in `/var/log/`, `journalctl -b -1`).
4. Re-deploy a fix.

This is why `boot` is the safer first-touch action: the bootloader
entry is the only mutation; live system stays on the prior gen until
reboot, and the menu lets you opt out.

## What's not yet wired

- **Multi-node deploys.** `--node zeus` from `ouranos` would want
  `ssh root@zeus` not `ssh root@localhost` for the privileged tail.
  Today the ssh-host is hardcoded `localhost`, so cross-node deploy
  is broken. Run `lojix-cli` *on the target node* until horizon-derived
  addressing lands.
- **Home-only deploy.** `lojix-cli` deploys system + HM together. There
  is no `--home-only` mode for "just rebuild HM, leave system alone."
  Tracked in `bd CriomOS-4yt`.
- **Streaming progress.** `nix build` output streams to your terminal
  via `stderr=inherit`; lojix-cli does not yet add structured
  per-target progress. Tracked in `bd CriomOS-t50` / `forge-auy`.

## Common patterns

### Deploy ouranos with a freshly-pushed CriomOS

```bash
rev=$(jj log -r main --no-graph --template 'commit_id' -n1 2>/dev/null \
      || git -C ~/git/CriomOS rev-parse main)
lojix-cli deploy --cluster goldragon --node ouranos \
  --source ~/git/goldragon/datom.nota \
  --action boot --criomos github:LiGoldragon/CriomOS/$rev
# reboot to land
```

### Verify a build will succeed without deploying

```bash
lojix-cli build --cluster goldragon --node ouranos \
  --source ~/git/goldragon/datom.nota \
  --criomos github:LiGoldragon/CriomOS/$rev
# prints the toplevel store path on success
```

### See what the projector produces (eval-only)

```bash
lojix-cli eval --cluster goldragon --node ouranos \
  --source ~/git/goldragon/datom.nota \
  --criomos github:LiGoldragon/CriomOS/$rev
# prints the .drvPath of the toplevel
```

## Pitfalls

- **`--action switch` on first-touch.** Live activation may fail on a
  new criomos if it changes the dbus implementation (dbus → dbus-broker
  trips the switch inhibitor). Use `boot` and reboot.
- **Unpinned `--criomos`.** Stale eval cache. Pin the rev.
- **Run as user, not root.** Don't `sudo lojix-cli`; the projection
  caches under `/root/.cache` instead of `~/.cache` and you lose
  user-side reuse on the next iteration. lojix-cli escalates internally.
- **Forgetting to push CriomOS / CriomOS-home / hexis bumps.** Pinned
  rev fetches from origin; unpushed commits are invisible. The
  `jj git push --bookmark main` step is non-optional after every
  bump.
- **Prom binary cache offline.** `warning: unable to download
  http://nix.prometheus.goldragon.criome/...` is benign — falls back
  to `cache.nixos.org`. Slower if many derivations weren't built
  upstream; otherwise no impact.

## See also

- `lojix-cli/ARCHITECTURE.md` — internal actor topology, projection
  semantics, the planned migration to the forge daemon.
- `lojix-cli/AGENTS.md` — bootstrap rules for working in the lojix-cli
  repo itself.
- `nix/basic-usage.md` — when in doubt about a nix command shape.
