# ARCHITECTURE — lojix-cli

This is the **forked development repo** for the next generation of
the CriomOS deploy CLI. It starts as a copy of the working
`lojix-cli` monolith so the live tool can remain untouched
while the new shape is built and verified.

## Role

This repo owns the risky redesign work that should not land directly
in `lojix-cli`:

- Nota-native request input
- request files and local defaults
- system vs home target generalization
- local home profile / activation flows
- internal reshaping needed to make those concepts first-class

The original `lojix-cli` remains the operator's current deploy tool.

## Boundaries

What lives here:

- the copied actor pipeline from `lojix-cli`
- the CLI surface and typed request model
- CriomOS-specific build and activation behavior while the fork is being
  proven out

What does not live here:

- `forge` daemon work
- `signal` wire design
- `horizon-rs` schema or method logic
- home modules inside CriomOS itself

## Current Code Map

- [src/main.rs](src/main.rs): Nota/request-file entrypoint
- [src/deploy.rs](src/deploy.rs):
  coordinator; projection, artifact, build, copy, and activation flow
- [src/build.rs](src/build.rs): typed build plans, target attr selection,
  deployment-shape selection, and remote-builder execution
- [src/activate.rs](src/activate.rs):
  system activation plus local home profile / activation behavior
- [tests/](tests): argv-shape and
  builder-validation tests that anchor the Nota-only CLI

## Invariants

- Horizon still flows into CriomOS as the `horizon` flake input.
- CriomOS still exposes one public surface:
  `nixosConfigurations.target`.
- Home deploys build from that same surface via the embedded
  Home Manager activation package path. This fork does not add a separate
  `homeConfigurations` surface to CriomOS.
- Nota is the canonical operator-facing data format.
- The live `lojix-cli` repo is not the place for this rewrite.

## Status

**TRANSITIONAL.** Active development fork. When the fork reaches a
verified shape, the cutover can be decided intentionally; until then
both repos coexist.

## Cross-Cutting Context

- project-wide engine context: `~/git/criome/ARCHITECTURE.md`
- current design source: `~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`
- workspace registration + work survey:
  workspace report 123, the fork creation and work survey
