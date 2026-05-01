# ARCHITECTURE — lojix-cli-v2

`lojix-cli-v2` is the **forked development repo** for the next
generation of the CriomOS deploy CLI. It starts as a copy of the
working `lojix-cli` monolith so the live tool can remain untouched
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
- the v2 CLI surface and typed request model
- CriomOS-specific build and activation behavior while v2 is being
  proven out

What does not live here:

- `forge` daemon work
- `signal` wire design
- `horizon-rs` schema or method logic
- home modules inside CriomOS itself

## Current Code Map

- [src/main.rs](/home/li/git/lojix-cli-v2/src/main.rs): current
  Clap-first entrypoint and `DeployRequest` construction
- [src/deploy.rs](/home/li/git/lojix-cli-v2/src/deploy.rs):
  coordinator; projection, artifact, build, copy, and activation flow
- [src/build.rs](/home/li/git/lojix-cli-v2/src/build.rs): hardcoded
  system-toplevel build attr and remote-builder execution
- [src/activate.rs](/home/li/git/lojix-cli-v2/src/activate.rs):
  system-only activation behavior
- [tests/](/home/li/git/lojix-cli-v2/tests): argv-shape and
  builder-validation tests that currently anchor the existing CLI

## Invariants

- Horizon still flows into CriomOS as the `horizon` flake input.
- CriomOS still exposes one public surface:
  `nixosConfigurations.target`.
- Home deploys build from that same surface via the embedded
  Home Manager activation package path. V2 does not add a separate
  `homeConfigurations` surface to CriomOS.
- Nota is the canonical operator-facing data format.
- The live `lojix-cli` repo is not the place for this rewrite.

## Status

**TRANSITIONAL.** Active development fork. When v2 reaches a verified
shape, the cutover can be decided intentionally; until then both repos
coexist.

## Cross-Cutting Context

- project-wide engine context: `~/git/criome/ARCHITECTURE.md`
- current v2 design source: `~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`
- workspace registration + work survey:
  `~/git/workspace/reports/123-lojix-cli-v2-repo-creation-and-work-survey-2026-05-01.md`
