# INTENT — lojix-cli

*What the psyche has explicitly intended for this project.
Synthesised from psyche statements and applicable workspace
constraints; not embellished. `ARCHITECTURE.md` says what
lojix-cli IS; this file says what the psyche wants it to BE.*

## Purpose

`lojix-cli` is the CriomOS deploy CLI: the NOTA-native operator
tool that projects a cluster proposal through `horizon-rs`,
materializes the small generated flake inputs a deploy needs, and
dispatches Nix build / activation work locally or through an SSH
builder. It is the current production deploy entry point for
CriomOS.

## Constraints

- **NOTA is the canonical operator-facing data format.** The CLI
  takes a single NOTA request value (or a request file);
  `lojix-cli '(Build (Cluster goldragon) (Node prometheus))'`.
  NOTA parsing behaviour belongs to `nota-codec` and `nota-derive`;
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

## Scope — explicitly transitional

lojix-cli deploys on today's Nix-based stack while CriomOS is
pre-duct-tape. It is built rightly for today's deploy needs, not as
a draft of the eventual. It stays at its current schema for the
duration of the horizon re-engineering arc and **retires** after
CriomOS migrates to consume the new `lojix` daemon's projection —
it does not gradually grow into a client of that daemon. When the
schema-engine cutover lands, this CLI becomes a client of the
`signal-lojix` contract's schema-emitted record types (the daemon
is the schema owner; the CLI is its first client); if retirement
arrives first, the cutover coincides with retirement rather than a
mid-life refactor. Per `primary/ESSENCE.md` §"Today and eventually".

*Source statements live in Spirit intent records and the project's
`ARCHITECTURE.md`. Workspace-shape intent stays in
`primary/INTENT.md` and the named skills above.*
