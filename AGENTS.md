# Agent Bootstrap — lojix-cli

You **MUST** read `~/git/lore/AGENTS.md` first. This file is the
repo-specific carve-out only.

## First thing

Run `bd list --status open` to see what's already on the table.
The active design context starts with:

- CriomOS report 0039, the full work survey and reading order
- `~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`

## Scope

This repo is the **safe rewrite fork** of the working `lojix-cli`.
The original repo remains Li's live deploy tool. This
repo is where the Nota-native CLI surface, local request/config
loading, home deploy support, and deeper internal reshaping land.

The starting point is a copy of the current monolith: CLI + ractor
actors + horizon projection + nix invocation. Unlike the original
repo, this one may be restructured aggressively.

Hard boundary:

- do not edit the live `~/git/lojix-cli/` repo as part of fork work;
- land exploratory or breaking redesign work here first;
- only backport intentionally once this fork has a verified shape.

## Architecture

Today the copied code still has the old actor pipeline:

```
DeployCoordinator (supervisor; OneForOne)
  ├── ProposalReader     reads + caches the source nota
  ├── HorizonProjector   horizon-lib in-process; NOT subprocess
  ├── HorizonArtifact    writes flake.nix + horizon.json; computes narHash; tars; optionally uploads
  └── NixBuilder         spawns nix; streams stdout/stderr
```

The first architectural move is **not** "thin client to forge". It is:

1. make Nota the canonical request surface;
2. generalize the build target beyond system toplevel only;
3. add local home build/profile/activate flows;
4. only then revisit larger daemon/client separations.

## Style

- Rust style canon: `~/git/lore/rust/style.md`.
- Nix packaging canon: `~/git/lore/rust/nix-packaging.md`.
- Methods on types, no free functions outside `main`.
- Typed newtypes at boundaries. No bare `String`/`PathBuf` once the
  CLI decode step is crossed.
- Single object in, single object out at actor boundaries.
- `Error` is one `thiserror`-derived enum per crate; inner errors
  wrap via `#[from]`. No `anyhow`, no `eyre`.
- Edition 2024.
- Tests live under `tests/`, not `#[cfg(test)]` blocks.

## Hard process rules

- Jujutsu only. Never `git` CLI.
- Push immediately after every change.
- Operator-facing deploy requests use human flake refs such as
  `github:LiGoldragon/CriomOS/main`; do not paste resolved commit
  hashes into chat, request examples, docs, or configs to satisfy
  freshness. Freshness is handled by Nix `--refresh`.
- Commit message style follows the workspace contract.
- Beads issues are one-liners — never paragraphs of design /
  implementation / rationale.
