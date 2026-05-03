# Direct Home Deploy

## Question

`HomeOnly` is meant to bypass the CriomOS flake. The current
implementation does not: it builds a Home Manager activation package
through:

```text
<criomos>#nixosConfigurations.target.config.home-manager.users.<user>.home.activationPackage
```

That still evaluates the full CriomOS NixOS configuration and consumes
`CriomOS-home` only through CriomOS's `criomos-home` input. This misses
the point of fast home iteration against the home repo itself.

## Current Facts

`CriomOS-home` exposes:

```text
homeModules.default
```

It does not currently expose standalone `homeConfigurations`.

Its top aggregate is designed to work in both contexts:

```text
modules/home/default.nix
```

The module expects these arguments:

```text
horizon
user
inputs
```

The flake wrapper around `homeModules.default` already imports upstream
home modules from `stylix`, `niri-flake`, and `noctalia`, and forces
`_module.args.inputs` to CriomOS-home's own inputs. That part is
correct for direct consumption.

The existing CriomOS NixOS path adds the per-user pieces outside
CriomOS-home:

```nix
home-manager.extraSpecialArgs = {
  inherit horizon constants;
};
home-manager.sharedModules = [ inputs.criomos-home.homeModules.default ];
home-manager.users = mapAttrs mkUserConfig horizon.users;
```

The direct path must recreate only the Home Manager evaluation, not
the NixOS system.

One current `CriomOS-home` module also expects:

```text
constants
```

Today only `constants.fileSystem.screenshots` is used by home code.
That value currently originates in CriomOS's `modules/nixos/constants.nix`,
but direct home deployment must not import or evaluate the CriomOS
flake. If a value is needed by both CriomOS and CriomOS-home, it belongs
in `CriomOS-lib`, whose flake is already described as shared helpers
and data consumed by both repos. Importing CriomOS just to get constants
would reintroduce the dependency this deploy kind exists to avoid.

## Important Wrinkle: Pkgs

CriomOS-home modules assume the `pkgs` they receive has CriomOS-pkgs
overlays and `allowUnfree = true`. The VSCodium module specifically
uses:

```text
pkgs.open-vsx
pkgs.vscode-utils.buildVscodeMarketplaceExtension
pkgs.vscode-extensions.*
```

`pkgs.open-vsx` comes from the `CriomOS-pkgs` overlay, not plain
nixpkgs. Therefore a direct home wrapper should not instantiate bare
nixpkgs unless CriomOS-home first stops relying on that overlay.

The right direct home build inputs are:

```text
criomos-home
home-manager
pkgs
horizon
system
criomos-lib
```

where `pkgs` is the same `CriomOS-pkgs` flake style used by CriomOS,
with its `system` input following the projected target system, and
`criomos-lib` provides shared non-host-specific data used by both
CriomOS and CriomOS-home.

`CriomOS-pkgs` owns the nixpkgs revision for direct home deploys. The
generated wrapper's root `nixpkgs` input should follow `pkgs/nixpkgs`,
and `CriomOS-home` should follow that root. The wrapper must not make
`CriomOS-pkgs` follow `CriomOS-home`'s nixpkgs; that inverts ownership
and lets the home repo accidentally choose the package universe.

The package universe source is the `LiGoldragon/nixpkgs` fork on `main`. CriomOS,
CriomOS-home, CriomOS-pkgs, and lojix-cli all point at that fork so
the ecosystem controls when nixpkgs advances. Upstream NixOS/nixpkgs
is consumed by updating the fork, not by individual repos following
upstream directly.

The direct wrapper must also preserve the same effective Home Manager
package overlay set as the NixOS-integrated path. Standalone Home
Manager imports its `nixpkgs` module and merges `nixpkgs.overlays`
from modules such as Stylix. In the NixOS-integrated path,
`home-manager.useGlobalPkgs` plus CriomOS's read-only pkgs keeps Home
Manager on the already-instantiated CriomOS-pkgs package set. If the
direct wrapper lets standalone Home Manager extend overlays, packages
that happen to depend on themed libraries can drift; observed example:
Stylix added a `gtksourceview` post-fixup, which changed Inkscape's
derivation even though nixpkgs and CriomOS-pkgs were pinned correctly.
The wrapper therefore forces:

```nix
nixpkgs.overlays = lib.mkForce pkgs.overlays;
```

This keeps the CriomOS-pkgs overlays (`open-vsx`, local check
overrides, etc.) while preventing Home Manager-only overlay mutation.

## Home Manager Interface

Home Manager's flake library exposes:

```nix
home-manager.lib.homeManagerConfiguration {
  pkgs = ...;
  modules = [ ... ];
  extraSpecialArgs = { ... };
}
```

The returned value has:

```text
activationPackage
```

So a direct wrapper flake can expose one package:

```text
packages.<system>.activationPackage
```

or a Home Manager-compatible output:

```text
homeConfigurations."<user>@<node>".activationPackage
```

For lojix's build machinery, a simple package attr is enough and keeps
the target attr independent of user-facing Home Manager CLI naming.

## Remote Home Deploy

`HomeOnly` must support remote use. Bypassing CriomOS means bypassing
the CriomOS flake evaluation, not restricting activation to the
dispatcher host.

The remote sequence is:

```text
project horizon for target node
materialize horizon/system/home-wrapper inputs
stage wrapper inputs onto builder when builder is set
build activation package locally or on builder
copy resulting closure to target when activation is requested
run profile/activate as the requested Unix user on the target
```

The builder field has the same meaning for `HomeOnly` as for system
deploys:

- omitted builder: build on dispatcher;
- builder equal to target node: build on the target;
- builder equal to another builder node: build there, then copy to
  target.

`HomeOnly Build` does not activate and therefore does not need a target
copy. `HomeOnly Profile` and `HomeOnly Activate` both require the
closure to exist on the target before the profile command runs.

Remote home activation must use the requested Unix user, not root. The
profile command is the same command currently used locally:

```text
nix-env -p ~/.local/state/nix/profiles/home-manager --set <activationPackage>
```

For `Activate`, lojix runs the generation's `activate` script after
setting the profile. The remote command must run through SSH as the
target user. That keeps Home Manager state in the target user's home
directory and avoids root-owned profile or state files.

## Wrapper Flake Shape

`lojix-cli` should materialize a generated direct-home flake for each
`HomeOnly` request. The generated flake should be small and explicit:

```nix
{
  inputs = {
    criomos-home.url = "github:LiGoldragon/CriomOS-home/main";
    home-manager.follows = "criomos-home/home-manager";
    nixpkgs.follows = "pkgs/nixpkgs";
    criomos-home.inputs.nixpkgs.follows = "nixpkgs";

    criomos-lib.url = "github:LiGoldragon/CriomOS-lib/main";
    system.url = "path:./system";
    pkgs.url = "github:LiGoldragon/CriomOS-pkgs";
    pkgs.inputs.system.follows = "system";

    horizon.url = "path:./horizon";
  };

  outputs = inputs:
  let
    system = inputs.system.system;
    pkgs = inputs.pkgs.pkgs;
    horizon = inputs.horizon.horizon;
    userName = "...";
    user = horizon.users.${userName};

    home = inputs.home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      extraSpecialArgs = {
        inherit horizon user;
        constants = inputs.criomos-lib.lib.constants;
      };
      modules = [
        inputs.criomos-home.homeModules.default
        {
          home.stateVersion = "26.05";
        }
      ];
    };
  in {
    packages.${system}.activationPackage = home.activationPackage;
    homeConfigurations.${userName} = home;
  };
}
```

The exact attr can be chosen during implementation. The important
part is that the attr is under the generated wrapper flake, not under
CriomOS.

## Lojix Request Shape

The current `HomeOnly` field named `criomos` is wrong for direct home
deploys. The request should carry a home flake reference:

```nota
(HomeOnly cluster node user source home mode builder?)
```

where `home` is usually:

```text
github:LiGoldragon/CriomOS-home/main
```

The field remains positional in Nota, but the Rust struct should rename
it from `criomos` to `home` so the type reads correctly.

`FullOs` and `OsOnly` keep the CriomOS flake field.

## Materialized Inputs

Current system/home-through-CriomOS materialization creates:

```text
horizon
system
deployment
```

Direct home materialization should create:

```text
horizon
system
home-wrapper
CriomOS-lib
```

The wrapper flake either embeds the selected `home` flake ref in its
`inputs.criomos-home.url`, or points to a local path/ref if the request
uses one.

The wrapper must be self-contained for remote builds. Its `system` and
`horizon` inputs are relative paths (`path:./system` and
`path:./horizon`), so wrapper materialization writes those subflakes
inside the wrapper directory instead of relying on sibling cache
directories. This makes rsyncing only `home-wrapper` to a remote
builder sufficient and avoids source-copy failures during lock-file
resolution.

The `deployment` input is not needed for direct home builds because
CriomOS is not being evaluated.

The wrapper also needs the shared constants from `CriomOS-lib` unless
`CriomOS-home` moves `constants.fileSystem.screenshots` into its own
module defaults first. This must come from the shared lib input, not
from CriomOS.

## Build Target

Current `HomeOnly` target attr:

```text
<criomos>#nixosConfigurations.target.config.home-manager.users.<user>.home.activationPackage
```

Direct target attr:

```text
<home-wrapper>#packages.<system>.activationPackage
```

or:

```text
<home-wrapper>#homeConfigurations.<user>.activationPackage
```

The package attr is simpler because the wrapper already knows the
selected user and target system.

## Build Observations

After pinning `LiGoldragon/nixpkgs/main` to the working nixpkgs
revision and pinning `CriomOS-pkgs`'s `nix-vscode-extensions` input to
the working overlay revision, direct-home still planned an Inkscape
build. The cause was not nixpkgs: it was standalone Home Manager
applying Stylix overlays to the package set. Forcing
`nixpkgs.overlays` to the base `pkgs.overlays` made direct-home's
Inkscape and `gtksourceview` derivations match the system-integrated
CriomOS evaluation.

The remaining planned builds after that fix were legitimate current
`CriomOS-home/main` changes, not direct-home drift: current home input
pins evaluate newer `codex`, `claude-code`, `noctalia`, and
`quickshell` artefacts than the active local Home Manager profile.
Those were not present in the local store or on the tested remote
builder at the time of the attempt, so deploying latest home-only
would require building or substituting those artefacts first.

## Validation

Keep the existing validations:

- projected horizon must contain the requested user;
- `Profile` and `Activate` must run as the requested Unix user;
- `Profile` and `Activate` must run on the requested node;

Add direct-home-specific validation:

- the home flake ref must be a branch/ref style operator-facing flake
  ref, not a pasted commit hash in docs/examples/configs;
- Nix build/eval still uses `--refresh`;
- wrapper generation must not import CriomOS;
- wrapper generation must use CriomOS-pkgs or otherwise provide the
  `pkgs` shape CriomOS-home expects.
- wrapper generation must let CriomOS-pkgs own nixpkgs and make
  CriomOS-home follow that package universe.
- wrapper generation must satisfy the home-used `constants` argument
  through `CriomOS-lib`, or eliminate that argument from CriomOS-home.
- remote `Profile` and `Activate` must copy the closure to the target
  before running the profile command;
- remote `Profile` and `Activate` must execute through SSH as the
  requested user.

## Implementation Plan

1. Rename `HomeOnly.criomos` to `HomeOnly.home` in Rust.
2. Move shared constants needed by home into `CriomOS-lib`, then update
   CriomOS and CriomOS-home consumers to use the lib value.
3. Add a `HomeWrapperDir` materializer that writes a flake exposing one
   Home Manager activation package.
4. Split build target selection so `BuildPlan::Home` uses the generated
   home wrapper, not the CriomOS flake.
5. Teach `HomeOnly` to accept validated remote builders using the same
   builder rules as system deploys.
6. Teach home activation to copy the closure to the target for
   `Profile` and `Activate`, then run profile/activation locally only
   when the target is the dispatcher and remotely as the requested user
   otherwise.
7. Keep `FullOs` and `OsOnly` on the CriomOS path with `deployment`.
8. Update request, invocation, and eval tests so `HomeOnly` proves no
   CriomOS attr appears in the Nix target.
9. Update README to state that `HomeOnly` consumes CriomOS-home
   directly.
10. Run a direct `HomeOnly Build` first, then `Profile`, then only run
   `Activate` intentionally because live HM activation can mutate the
   graphical session.

## Decision

`HomeOnly` should bypass CriomOS. The right shape is not to add another
CriomOS output; it is to generate a small standalone Home Manager
wrapper flake that consumes `CriomOS-home.homeModules.default` directly
with lojix-projected `horizon`, selected `user`, target `system`, and
the CriomOS-pkgs package set. Shared data needed by that path must live
in `CriomOS-lib`, not in CriomOS.
