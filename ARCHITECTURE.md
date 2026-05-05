# ARCHITECTURE — lojix-cli

This is the CriomOS deploy CLI. It is the Nota-native operator tool for
projecting a cluster proposal through `horizon-rs`, materializing the
small generated flake inputs needed for a deploy, and dispatching Nix build /
activation work locally or through an SSH builder.

## Role

This repo owns:

- Nota-native request input
- request files and local defaults
- system vs home target generalization
- full-OS, OS-only, and direct home-only deploy flows
- local and remote home profile / activation flows
- SSH dispatch and closure-copy behavior for remote builders

## Boundaries

What lives here:

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
- Home-only deploys bypass CriomOS and evaluate `CriomOS-home`
  directly with the same generated `horizon` and `system` inputs.
- Generated deploy inputs are consumed as local `path:` flake refs
  carrying NAR hashes.
- Nota is the canonical operator-facing data format.
- Nota parsing behavior belongs to `nota-codec` and `nota-derive`;
  lojix-cli evolves request syntax through typed records and upstream
  codec capabilities.

## Status

Active deploy tool. The first-generation implementation is archived as
`lojix-archive`.

## Cross-Cutting Context

- project-wide engine context: `~/git/criome/ARCHITECTURE.md`
- current design source: `~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`
- workspace registration + work survey:
  workspace report 123, the fork creation and work survey
