# Three Deploy-Kind Split

## Question

The new `lojix-cli` surface needs a first-class split between:

| kind | artifact | activation owner | home effect |
|---|---|---|---|
| `FullOs` | NixOS system toplevel with embedded Home Manager | root/system | durable through the system generation |
| `OsOnly` | NixOS system toplevel with Home Manager disabled for this eval | root/system | no home-manager activation from this generation |
| `HomeOnly` | one user's Home Manager activation package | target user | local live/profile overlay, not inherently reboot-durable |

This must be a typed Nota input shape, not CLI flags.

## Current Constraint

CriomOS currently has one public NixOS surface:

```text
nixosConfigurations.target
```

The build attr used by `lojix-cli` is:

```text
nixosConfigurations.target.config.system.build.toplevel
```

CriomOS imports Home Manager unconditionally and maps all projected
`horizon.users` into `home-manager.users`. Therefore the current system
toplevel is already `FullOs`.

Important consequence: `OsOnly` cannot be implemented honestly inside
`lojix-cli` by merely skipping a post-build home activation step. The
system generation itself would still contain Home Manager and boot-time
`hm-activate-<user>` units. True `OsOnly` needs CriomOS evaluation to
exclude the Home Manager module/users for that build.

## Recommended Nota Wire Shape

Use the deploy kind as the top-level record head. That keeps the
operator-facing shape direct and avoids reintroducing a hidden
subcommand/flag grammar.

```nota
(FullOs goldragon ouranos "/home/li/git/goldragon/datom.nota" "github:LiGoldragon/CriomOS/<rev>" BootOnce None)
(OsOnly goldragon ouranos "/home/li/git/goldragon/datom.nota" "github:LiGoldragon/CriomOS/<rev>" Boot None)
(HomeOnly goldragon ouranos li "/home/li/git/goldragon/datom.nota" "github:LiGoldragon/CriomOS/<rev>" Profile None)
```

Field order should be frozen by the Rust `NotaRecord` schema and
documented by golden encoder tests, not hand-maintained prose. The
trailing builder remains optional, but canonical output should emit
`None` because `nota-codec` always encodes absent `Option<T>` fields
explicitly.

Action spaces must stay separate:

| kind | action/mode type | valid values |
|---|---|---|
| `FullOs` | system action | `Eval`, `Build`, `Boot`, `Switch`, `Test`, `BootOnce` |
| `OsOnly` | system action | `Eval`, `Build`, `Boot`, `Switch`, `Test`, `BootOnce` |
| `HomeOnly` | home mode | `Build`, `Profile`, `Activate` |

Do not accept `Boot`, `Switch`, `Test`, or `BootOnce` in a home request.
Do not overload `BuildAction` across system and home.

## CriomOS Boundary

Keep the public output as `nixosConfigurations.target`. Do not add
`targetOsOnly`, `homeConfigurations`, or host-specific outputs.

Recommended boundary change: add a tiny deployment-shape flake input
that `lojix-cli` overrides beside `horizon` and `system`.

```text
inputs.deployment.deployment.includeHome = true | false
```

Then CriomOS uses that value to decide whether to include:

| module/input | `FullOs` | `OsOnly` | `HomeOnly` eval |
|---|---:|---:|---:|
| Home Manager NixOS module | yes | no | yes |
| `modules/nixos/userHomes.nix` | yes | no | yes |
| `inputs.criomos-home.homeModules.default` | yes | no | yes |

This preserves the single public `target` surface while making the
evaluation cache key differ by deploy kind. `lojix-cli` already
materializes override-input flakes for `horizon` and `system`; adding a
third materialized `deployment` input is consistent with that model.

## Build Target Selection

`FullOs` and `OsOnly` both build the same system attr:

```text
nixosConfigurations.target.config.system.build.toplevel
```

The difference is the `deployment` override input:

| kind | deployment input | attr |
|---|---|---|
| `FullOs` | `includeHome = true` | system toplevel |
| `OsOnly` | `includeHome = false` | system toplevel |
| `HomeOnly` | `includeHome = true` | user activation package |

`HomeOnly` builds:

```text
nixosConfigurations.target.config.home-manager.users.<user>.home.activationPackage
```

If `includeHome = false`, that attr must not exist. This is desirable:
it makes accidental home builds under an OS-only eval fail at the type
or request-resolution layer before Nix.

## Activation Semantics

System activation (`FullOs`, `OsOnly`) remains root-owned:

| action | effect |
|---|---|
| `Eval` | print drv path, no closure |
| `Build` | build closure, no copy/activation unless needed for remote builder handling |
| `Boot` | set system profile, install boot entry, reconcile EFI default |
| `Switch` | set profile, live switch, reconcile EFI default |
| `Test` | live test switch only |
| `BootOnce` | set one-shot boot entry and preserve current persistent default |

Home activation (`HomeOnly`) is user-owned:

| mode | effect |
|---|---|
| `Build` | build activation package only |
| `Profile` | set `~/.local/state/nix/profiles/home-manager` |
| `Activate` | set profile, then run the package's `activate` |

`HomeOnly Activate` is intentionally risky for graphical sessions.
Report 0037 showed a freeze after live Home Manager activation changed
niri/Xwayland-related state. The safe default for no-arg local redeploy
should be either `FullOs BootOnce` for durable OS+home work or
`HomeOnly Profile` for home iteration without live session mutation.

## SSH-Break-Proof Dispatch

The current `BootOnce` path already uses a transient target-side
`systemd-run --wait --collect --service-type=oneshot` unit. The split
should generalize that idea instead of keeping it as a special case.

Recommended system rule:

```text
Every effect-bearing system action runs one target-side transient unit.
```

That unit should include the whole mutation sequence:

| action | transient unit body must include |
|---|---|
| `Boot` | profile set, `switch-to-configuration boot`, EFI default reconcile, one-shot clear |
| `Switch` | profile set, `switch-to-configuration switch`, EFI default reconcile, one-shot clear |
| `Test` | `switch-to-configuration test` only |
| `BootOnce` | profile set, boot entry install, persistent-default preservation, one-shot set |

This avoids the current partial safety where `BootOnce` is SSH-break
resistant but `Boot`/`Switch` still have multi-step SSH tails for EFI
reconciliation.

Remote `HomeOnly` should not be mixed into the first cut. It needs a
separate design for user SSH identity, closure copy destination, and
whether activation should use `systemd-run --user`. Local `HomeOnly`
should require running on the target as the requested user, or fail
clearly if that cannot be established.

## Pipeline Changes

The actor pipeline can stay, but the objects crossing it need sharper
nouns:

```text
Nota request
  -> projected horizon
  -> deployment input materialization
  -> realization target selection
  -> build/eval
  -> domain-specific finish
```

Required splits:

| current noun | problem | replacement direction |
|---|---|---|
| `BuildAction` | mixes eval/build/system activation | separate system action from home mode |
| `NixBuild.action` | action implies attr path | carry a realization target plus eval/build behavior |
| `DeployRequest` | no deploy kind or user | carry `FullOs` / `OsOnly` / `HomeOnly` request |
| `SystemActivation` | only activation noun | add local `HomeActivation` sibling |
| `ClosureCopy` | assumes root SSH target | keep for system; add separate local/home closure handling later |

## Validation

Validate at request-resolution or horizon-boundary time:

| check | reason |
|---|---|
| `HomeOnly.user` exists in `horizon.users` | fail before Nix |
| `HomeOnly` does not use system actions | avoid impossible activation |
| system requests do not use home modes | avoid action ambiguity |
| `OsOnly` does not try to build home attrs | prove the deployment input is working |
| local `HomeOnly` runs as the requested Unix user | avoid mutating the wrong home |
| remote `HomeOnly` is rejected until designed | avoid fake root/user SSH semantics |

Builder validation remains as today for system requests. For `HomeOnly`,
remote builders should either be rejected in the first cut or followed
by an explicit local-store copy-from-builder step before profile
mutation.

## Implementation Order

1. Replace the request schema with the three top-level deploy-kind
   records and golden Nota encoder tests.
2. Add `deployment` override-input materialization in `lojix-cli`.
3. Patch CriomOS to gate Home Manager imports/users from that
   deployment input while preserving `nixosConfigurations.target`.
4. Generalize Nix build attr selection and exact argv tests.
5. Add local `HomeActivation` and home-user validation.
6. Convert all effect-bearing system actions to one target-side
   transient unit.
7. Only then decide whether remote `HomeOnly` belongs in this CLI or a
   later daemon/client shape.

## Decision

The three-kind split is real, not a boolean:

- `FullOs` is the existing durable system-plus-home generation.
- `OsOnly` needs a CriomOS evaluation switch; skipping activation in
  `lojix-cli` is insufficient.
- `HomeOnly` is a separate user-profile flow and must not share system
  activation machinery.

