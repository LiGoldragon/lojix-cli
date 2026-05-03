# Handoff: Pure Inputs And Direct Home

## Current State

The deploy-input shape has been corrected after the aborted archive
publisher direction.

`lojix-cli` now materializes generated deploy inputs locally and passes
them to Nix as `path:` flake refs with `narHash` query parameters. There
is no network archive publisher and no generated Home Manager wrapper in
`lojix-cli`.

Generated inputs currently owned by lojix:

| Input | Used By | Shape |
|---|---|---|
| `horizon` | CriomOS and CriomOS-home | projected horizon JSON flake |
| `system` | CriomOS, CriomOS-pkgs, and CriomOS-home | target system string flake |
| `deployment` | CriomOS system deploys only | `includeHome` shape flake |

`HomeOnly` evaluates `CriomOS-home` directly:

```text
<home-flake>#homeConfigurations.<user>.activationPackage
```

The requested `home` flake in the Nota input is now the evaluated root
flake for `HomeOnly`; it is no longer embedded inside a generated
wrapper.

## Repo Changes

### lojix-cli

Commit:

```text
d99105261480 Use pure local generated inputs
```

Main changes:

- removed `src/publish.rs`;
- removed `ArchivePublisher` / tarball upload behavior;
- removed `HomeWrapperDir` and generated `home-wrapper`;
- changed `FlakeInputRef` to local `path:` plus `narHash`;
- changed `HomeOnly` target attr to
  `homeConfigurations.<user>.activationPackage`;
- updated README, architecture, and report 0004.

Validation:

```text
nix develop -c cargo clippy --all-targets --all-features -- -D warnings
nix develop -c cargo test --no-fail-fast
nix develop -c cargo build
```

All passed.

### CriomOS-home

Commits:

```text
9f14bdd24607 Expose direct home configurations
f22f9f8100bc Bump lojix direct home support
```

Main changes:

- added `horizon`, `system`, and `pkgs` flake inputs;
- added stubs for missing `horizon` and `system`;
- exposed `homeConfigurations` from projected `horizon.users`;
- direct home configurations instantiate Home Manager with
  `inputs.pkgs.pkgs`;
- updated `lojix-cli` input lock after the CLI correction.

Validation:

```text
nix eval --raw \
  path:/home/li/git/CriomOS-home#homeConfigurations.li.activationPackage.drvPath \
  --override-input horizon path:$H?narHash=$HH \
  --override-input system path:$S?narHash=$SH
```

This evaluated successfully using local generated `horizon` and `system`
inputs.

`nix flake check --no-build` reaches the known Blueprint
`checks.x86_64-linux.pkgs-formatter-__ignoreNulls` issue.

### CriomOS

Commit:

```text
152779ef45da Forward deploy inputs to home
```

Main changes:

- forwards `horizon`, `system`, and `pkgs` to `criomos-home` with
  `follows`;
- updated the `criomos-home` lock to the direct-home-capable commit;
- removed the erroneous generated-input archive HTTP service.

Validation:

`nix flake check --no-build` still fails before reaching this change on
the existing Blueprint `modules/nixos/disks/default.nix` discovery
issue.

## Important Correction

The prior tarball/archive implementation was the wrong boundary.
Publication mechanics do not belong in the CLI as currently shaped.

The current accepted first pass is:

```text
local generated source tree + NAR hash -> path:<dir>?narHash=<hash>
```

This preserves pure flake input identity while leaving long-term storage
open.

## Remaining Design

`system` can become a tiny real repo because its value space is finite.
A clean shape is one branch per contained system:

```text
x86_64-linux
aarch64-linux
```

Each branch exposes:

```nix
{
  outputs = _: {
    system = "<branch-name>";
  };
}
```

`horizon` remains the main unsolved storage question because it is
projected per cluster/node/proposal and has real generated content.

`deployment` remains only a CriomOS system-deploy input for the
`FullOs`/`OsOnly` split. It is not used by `HomeOnly`.

## Follow-Up Risks

- `CriomOS-home` now exposes `homeConfigurations` from `horizon.users`;
  the default stub horizon returns an empty user set so flake checks can
  inspect the output without a real projection.
- Remote builders currently receive local `path:` refs. That is fine for
  local development and local activation, but remote build usability
  still depends on a shared/published generated-input storage answer.
- `HomeOnly` no longer has an extra wrapper layer, so any direct-home
  behavior must belong in `CriomOS-home` itself.
