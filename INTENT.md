# INTENT — lojix-cli

*What the psyche has explicitly intended for this project.
Synthesised from psyche statements and applicable workspace
constraints; not embellished. `ARCHITECTURE.md` says what
lojix-cli IS; this file says what the psyche wants it to BE.*

## Purpose

`lojix-cli` is the archived legacy CriomOS deploy CLI. It was the
NOTA-native monolithic operator tool that projected a cluster proposal
through `horizon-rs`, materialized the small generated flake inputs a
deploy needed, and dispatched Nix build / activation work locally or
through an SSH builder.

The production replacement is the daemon-based `lojix` stack:
`lojix-daemon`, the ordinary-socket `lojix` client, the owner/meta-socket
`meta-lojix` client, and `lojix-write-configuration`.

## Constraints

- **NOTA is the canonical operator-facing data format.** The CLI
  takes a single NOTA request value (or a request file);
  `lojix-cli '(Build (Cluster goldragon) (Node prometheus))'`.
  NOTA parsing behaviour belongs to `nota-next`;
  this CLI evolves request syntax through typed records and
  upstream codec capabilities, not its own parser. Per the
  workspace NOTA discipline (`primary/ESSENCE.md`).
- **Horizon flows into CriomOS as the `horizon` flake input;
  CriomOS exposes one public surface.** CriomOS is built only
  through `nixosConfigurations.target`; the horizon override picks
  which `(cluster, node)` materialises, and the deployment input
  picks the operation shape. Generated deploy inputs are consumed
  as local `path:` flake refs carrying NAR hashes.
- **Home-only deploys bypass CriomOS** and evaluate `CriomOS-home`
  directly with the same generated `horizon` and `system` inputs.
- **This CLI does not own schema or wire design.** It owns the CLI
  surface and the typed request model; it does not own `forge`
  daemon work, `signal` wire design, `horizon-rs` schema/method
  logic, or home modules inside CriomOS.

## Stack discipline

- Full English words; no crate-name prefix on types. Per
  `primary/skills/naming.md`.
- Rust functions are methods on data-bearing nouns, not free
  functions. Per `primary/skills/rust-discipline.md`.

## Scope — archived

lojix-cli has retired as an active deploy surface. It does not receive
new deploy behavior or schema work. Historical behavior stays available
through git history; active deploy work moves to `lojix`,
`signal-lojix`, and `meta-signal-lojix`. Per `primary/ESSENCE.md` §"Today and eventually".

*Source statements live in Spirit intent records and the project's
`ARCHITECTURE.md`. Workspace-shape intent stays in
`primary/INTENT.md` and the named skills above.*
