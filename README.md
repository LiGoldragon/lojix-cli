# lojix-cli

`lojix-cli` is the CriomOS deploy orchestrator. It reads one Nota
request, projects a cluster proposal through `horizon-lib`, materializes
the flake override inputs CriomOS expects, runs `nix`, and optionally
activates the result.

The CLI has no flags and no subcommands. The request is the interface.
All operator intent lives in the Nota record.

## Current Status

This repo is the rewrite workspace for the deploy CLI. It is allowed to
break the old CLI surface so the Nota-native model, the system/home
deploy split, and Home Manager activation can land in the right shape
before cutover.

The current implementation supports:

- `FullOs`: system generation with Home Manager included.
- `OsOnly`: system generation with Home Manager excluded for that
  CriomOS evaluation.
- `HomeOnly`: one user's Home Manager activation package, built by
  evaluating `CriomOS-home` directly.
- Inline Nota requests, request files, and a no-argument default config.
- Remote system and home builders selected from projected horizon nodes.
- Local or remote Home Manager profile setting and activation.
- SSH-break-resistant `BootOnce` system activation.

## Quick Start

Run an inline Nota request:

```sh
lojix-cli '(FullOs goldragon ouranos [./datom.nota] [github:LiGoldragon/CriomOS/main] BootOnce None None)'
```

Run a request file:

```sh
lojix-cli ./request.nota
```

Run the configured default request:

```sh
lojix-cli
```

Run from this repo without installing:

```sh
nix run .# -- '(HomeOnly goldragon ouranos li [./datom.nota] [github:LiGoldragon/CriomOS-home/main] Profile None None)'
```

The `--` in the last command belongs to `nix run`; it is not a
`lojix-cli` flag.

## Request Input

`lojix-cli` accepts exactly one request source:

| Invocation shape | Meaning |
|---|---|
| `lojix-cli '(<record> ...)'` | Decode the command-line text as an inline Nota record. |
| `lojix-cli ./request.nota` | Read and decode that file. Extra path arguments are rejected. |
| `lojix-cli` | Load the first existing default config file. |

Default config search order:

1. `LOJIX_CONFIG`
2. `XDG_CONFIG_HOME/lojix/config.nota`
3. `HOME/.config/lojix/config.nota`

No-argument mode does not invent a request. It only decodes the default
Nota file, so "local redeploy by default" is configured by putting the
desired request in that file.

Inline requests are joined back together when the first shell argument
starts with `(`. This lets normal shell tokenization work for simple
records, but quoting the whole record is still the clearest habit.

## Nota Schema

The top-level record head is the deploy kind. Fields are unnamed and
positional.

```nota
(FullOs cluster node source criomos action builder? substituters?)
(OsOnly cluster node source criomos action builder? substituters?)
(HomeOnly cluster node user source home mode builder? substituters?)
```

Fields:

| Field | Meaning |
|---|---|
| `cluster` | Horizon cluster name. |
| `node` | Target node name within the projected cluster. |
| `user` | Target Unix/Home Manager user for `HomeOnly`. |
| `source` | Path to the cluster proposal Nota file. |
| `criomos` | CriomOS flake reference to evaluate for `FullOs` and `OsOnly`. Use branch refs such as `github:LiGoldragon/CriomOS/main` for operator-facing requests. |
| `home` | CriomOS-home flake reference to evaluate for `HomeOnly`. Use branch refs such as `github:LiGoldragon/CriomOS-home/main`. |
| `action` | System action for `FullOs` and `OsOnly`. |
| `mode` | Home action for `HomeOnly`. |
| `builder?` | Optional builder node. Use `None`, omit it, or provide a node name. |
| `substituters?` | Optional list of horizon node names whose Nix cache endpoints should be injected into the build as `extra-substituters`. To specify substituters while leaving the builder unset, write `None (Some [prometheus])`. |

System actions:

| Action | Effect |
|---|---|
| `Eval` | Evaluate the Nix derivation path only. No closure build or activation. |
| `Build` | Build the selected closure. No copy or activation. |
| `Boot` | Build, copy to target, set system profile, install boot entry, reconcile EFI default. |
| `Switch` | Build, copy to target, set system profile, live switch, reconcile EFI default. |
| `Test` | Build, copy to target, run a non-persistent test switch. |
| `BootOnce` | Build, copy to target, install a one-shot boot entry while preserving the current persistent default. |

Home modes:

| Mode | Effect |
|---|---|
| `Build` | Build the Home Manager activation package only. |
| `Profile` | Set the user's Home Manager profile to the built generation. |
| `Activate` | Set the profile, then run the generation's activation script. |

Examples:

```nota
(FullOs goldragon ouranos [./datom.nota] [github:LiGoldragon/CriomOS/main] BootOnce None None)
(OsOnly goldragon ouranos [./datom.nota] [github:LiGoldragon/CriomOS/main] Build (Some prom) None)
(HomeOnly goldragon ouranos li [./datom.nota] [github:LiGoldragon/CriomOS-home/main] Profile None None)
(FullOs goldragon zeus [./datom.nota] [github:LiGoldragon/CriomOS/main] Boot (Some zeus) (Some [prometheus]))
(FullOs goldragon zeus [./datom.nota] [github:LiGoldragon/CriomOS/main] Boot None (Some [prometheus]))
```

All deploy kinds validate a requested builder against the projected
horizon before invoking Nix.

## Deploy Kinds

### `FullOs`

`FullOs` builds CriomOS with Home Manager enabled. The result is a
normal system generation whose system activation owns both OS-level
state and the Home Manager units embedded in the generation.

`FullOs` and `OsOnly` build the same public CriomOS output:

```text
nixosConfigurations.target.config.system.build.toplevel
```

The difference is the generated `deployment` override input.

### `OsOnly`

`OsOnly` builds the same CriomOS system output with Home Manager
disabled for that evaluation. This is not implemented by merely
skipping a post-build home step; CriomOS receives an override input
whose `deployment.includeHome` value is `false`, so the Home Manager
module and generated users are absent from the evaluated system.

This preserves the CriomOS invariant that the public system surface is
only:

```text
nixosConfigurations.target
```

### `HomeOnly`

`HomeOnly` builds one user's Home Manager activation package without
evaluating the CriomOS flake. `lojix-cli` evaluates the requested
`CriomOS-home` flake directly and passes the same generated `horizon`
and `system` inputs that CriomOS receives.

The build target is the requested home flake's Home Manager output:

```text
homeConfigurations.<user>.activationPackage
```

Before Nix runs, `lojix-cli` checks that the requested user exists in
the projected horizon. For `Profile` and `Activate`, it copies the
realized closure to the target when needed, then runs the profile and
activation commands as the requested Unix user on the target. If the
dispatcher is already the requested user on the requested node, the
commands run locally.

## How A Deploy Runs

The runtime flow is:

```text
Nota request
  -> typed request model
  -> cluster proposal read from source Nota
  -> horizon projection for cluster + node
  -> builder and home-user validation
  -> materialized override inputs
  -> nix eval/build
  -> optional closure copy
  -> optional system or home activation
```

The actor pipeline is:

```text
DeployCoordinator
  ├── ProposalReader       reads the source Nota proposal
  ├── HorizonProjector     projects with horizon-lib in-process
  ├── HorizonArtifact      writes generated flake inputs
  ├── NixBuilder           runs nix locally or through ssh
  ├── ClosureCopier        copies closures to activation targets
  └── Activator            performs system or home activation
```

Each actor message carries one domain object plus its reply channel.
Process execution is represented by `ProcessInvocation`, which owns the
program, arguments, stdout/stderr mode, process group, and kill-on-drop
behavior.

## Generated Inputs

`lojix-cli` materializes small flake inputs under the user's cache,
computes their NAR hashes, and passes local `path:` flake refs with a
`narHash` suffix to Nix.

| Input | Contents | Used as |
|---|---|---|
| `horizon` | Projected horizon JSON and flake wrapper. | `--override-input horizon ...` |
| `system` | The target Nix system string. | `--override-input system ...` |
| `deployment` | `deployment.includeHome = true` or `false`. | System deploys only: `--override-input deployment ...` |

The deployment shape is:

| Request kind | `includeHome` |
|---|---:|
| `FullOs` | `true` |
| `OsOnly` | `false` |
| `HomeOnly` | Not used |

`HomeOnly` evaluates `CriomOS-home` directly. That flake receives the
same generated `horizon` and `system` inputs as CriomOS.

When a remote builder is selected, `lojix-cli` first stages the generated
inputs onto that builder with `rsync`, under a content-keyed directory in
the builder's temporary storage. The Nix command that runs on the builder
then receives `path:` refs to those staged remote directories. This keeps
remote build evaluation independent of dispatcher-local cache paths.

## Builder Semantics

The optional builder field names a horizon node, not an arbitrary SSH
host. The name is resolved after horizon projection.

`lojix-cli` passes `--refresh` to Nix for both eval and build
operations. That keeps branch flake refs fresh without replacing them
with resolved commit hashes in requests or documentation.

Validation rules:

- `builder == node` is allowed and means "build on the target".
- A different builder must be present in projected `ex_nodes`.
- When the builder is a different node from the target, that builder
  must have `isRemoteNixBuilder = true`.
- When `builder == node`, the build runs on the target over SSH and does
  not require the target to expose the remote Nix builder service.

If `builder == node`, the closure copy phase is skipped because the build
already happened on the activation target.

The optional substituters list is an operator-selected subset of cluster
Nix cache nodes. Each name must resolve to a node with `nixUrl` and
`nixPubKeyLine`; when the node has a Yggdrasil address, `lojix-cli` uses
that address for the injected cache URL so bootstraps do not depend on
the target's current DNS state. The values are passed to Nix as
`extra-substituters` and `extra-trusted-public-keys`, preserving the
target's configured defaults such as `cache.nixos.org`.

Current third-party builder limitation: when `builder` names a node
different from the deployment target, `lojix-cli` runs the closure copy
from the dispatcher as `nix copy --from <builder> --to <target>`. Nix
streams the NAR data through the dispatcher process; it is not a direct
builder-to-target push. For large closures, run `lojix-cli` on the
builder itself or build on the target until the copy phase moves onto the
builder. See [report 0008](reports/0008-third-party-builder-copy-topology.md).

SSH always uses key-based batch mode. The target address is derived
from the projected node's Criome domain name as root SSH, not from a
CLI target flag. Home activation switches to the requested user for the
profile and activation commands.

## Activation Details

### System Activation

System activation applies to `FullOs` and `OsOnly`.

For `Boot`, `Switch`, and `Test`, `lojix-cli` copies the built closure
to the target unless it was already built on the target. It then runs
the target-side activation command over SSH.

`Boot` and `Switch` set the system profile before
`switch-to-configuration`, then reconcile EFI state by setting the EFI
default to the new generation and clearing any pending one-shot entry.

`Test` runs `switch-to-configuration test` without setting the system
profile and without EFI reconciliation.

`BootOnce` dispatches a target-side transient systemd unit with
`systemd-run --wait --collect --service-type=oneshot`. If the SSH
connection drops, the unit continues on the target. The unit installs
the new boot entry, restores the persistent default to the currently
running generation, and arms the new generation as the one-shot entry.

### Home Activation

Home activation applies to `HomeOnly`.

`Profile` sets the Home Manager profile in the user's state directory
to the built generation. It does not run the generation's activation
script.

`Activate` runs `Profile` first, then executes the generation's
activation script. This can mutate the live user session, so use it
when live home activation is intended.

For remote targets, `Profile` and `Activate` first copy the closure to
the target unless it was built there. The commands then run over SSH as
the requested user, not as root.

## Output

On success, stdout is the typed Nix result:

| Request result | Stdout |
|---|---|
| `Eval` | The evaluated derivation path. |
| Build or activation result | The realized output path reported by Nix. |

Nix progress and activation logs stream on stderr. Process failures are
reported through typed crate errors such as Nix failure, SSH failure,
local hostname failure, invalid builder, or unknown home user.

## Development

Enter the development shell:

```sh
nix develop
```

Run checks:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --no-fail-fast
nix flake check --no-write-lock-file
```

Build or run through Nix:

```sh
nix build .#
nix run .# -- '(FullOs goldragon ouranos [./datom.nota] [github:LiGoldragon/CriomOS/main] Eval None None)'
```

Repository-specific process and style rules live in `AGENTS.md`.
The repo role and invariants live in `ARCHITECTURE.md`. The main
design records for the current shape are:

- `reports/0001-three-deploy-kind-split.md`
- `reports/0002-code-standards-remediation.md`
- `reports/0003-direct-home-deploy.md`
- `reports/0004-pure-local-generated-inputs.md`
- `reports/0005-handoff-pure-inputs-direct-home.md`
- `reports/0006-archive-hosting-review.md`
