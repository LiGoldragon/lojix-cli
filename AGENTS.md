# Agent Bootstrap — lojix-cli

> ## 🚨 DO NOT REWRITE THIS REPO 🚨
>
> **This `lojix-cli` repo (local dir `~/git/lojix-cli/`) is Li's
> working CriomOS deploy orchestrator.** Monolithic-by-design
> for now: CLI + ractor actors + horizon projection +
> `nixos-rebuild` invocation all in one Rust crate. Used by
> hand to drive deploys *today*.
>
> **The criome architecture at
> criome/ARCHITECTURE.md
> describes an END STATE** where `forge` is the daemon and
> `lojix-cli` is a thin signal-speaking client of it. That is
> a **target**, not a current-state invariant. Do not:
>
> - ❌ Delete the `src/` tree and replace it with a README
> - ❌ Move actors to a non-existent `forge` crate
> - ❌ Restructure prematurely toward the long-term thin-client
>      shape
>
> Transition is gradual and lives in
> [ARCHITECTURE.md "Migration phases"](ARCHITECTURE.md). Read
> it before editing anything in `src/`.

## First thing

Run `bd list --status open` to see what's already on the table.
The implementation queue follows the design at
`/home/li/git/CriomOS/reports/2026-04-24-ractor-tool-design.md`.

## Scope

CriomOS deploy orchestrator. Reads a cluster proposal nota, projects
through `horizon-lib` in-process, writes a content-addressed horizon
flake (locally as `path:` or remotely as `tarball+url?narHash=...`),
and invokes `nixos-rebuild` against CriomOS with that horizon as
`--override-input horizon ...`.

The horizon dependency direction is **horizon flows INTO CriomOS as
an input**. forge produces the horizon; CriomOS consumes it. Same
horizon content → same narHash → nix eval/build cache hits across
machines.

## Architecture

ractor actor pipeline:

```
DeployCoordinator (supervisor; OneForOne)
  ├── ProposalReader     reads + caches the source nota
  ├── HorizonProjector   horizon-lib in-process; NOT subprocess
  ├── HorizonArtifact    writes flake.nix + horizon.json; computes narHash; tars; optionally uploads
  └── NixBuilder         spawns nix; streams stdout/stderr
```

See the design report for message shapes and lifecycle.

## Style

- Rust style canon: `~/git/lore/rust/style.md`.
- Methods on types, no free functions outside `main`.
- Typed newtypes (`ClusterName`, `NodeName` from `horizon-lib`;
  `ProposalSource`, `HorizonArtifact`, `BuildOutcome`, `FlakeRef`
  defined locally — no bare `String`/`PathBuf` at message
  boundaries).
- Single object in, single object out at actor boundaries.
- `Error` is one `thiserror`-derived enum per crate; inner errors
  wrap via `#[from]`. No `anyhow`, no `eyre`.
- Edition 2024.
- Tests live under `tests/`, not `#[cfg(test)]` blocks.

## Hard process rules

- Jujutsu only. Never `git` CLI.
- Push immediately after every change.
- Mentci commit format: see
  workspace/AGENTS.md.
- Beads issues are one-liners — never paragraphs of design /
  implementation / rationale.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:ca08a54f -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
