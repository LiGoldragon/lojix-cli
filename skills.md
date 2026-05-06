# Skill — lojix-cli

*The Nota-native CriomOS deploy CLI. Operator intent enters as
one Nota record; nothing else.*

---

## What this skill is for

Use this when adding, modifying, or debugging deploy behavior.
lojix-cli reads one Nota request, projects through horizon-rs,
materialises override flake inputs, runs `nix`, and optionally
copies/activates. Architectural shape is in `ARCHITECTURE.md`;
runtime semantics are in `README.md`.

---

## The CLI is one Nota record

The whole operator surface is a single Nota record decoded by
`src/request.rs`. **No flags. No subcommands. No env-var
dispatch. No custom argv parser.** The three top-level record
heads (`FullOs`, `OsOnly`, `HomeOnly`) and their positional
fields ARE the API.

When adding a new deploy behavior:

1. Add a typed field to the relevant request struct
   (`FullOs` / `OsOnly` / `HomeOnly`) in source-declaration
   order. New fields go at the tail as `Option<T>` so existing
   request files keep parsing.
2. Plumb the field through `into_deploy_request` to
   `DeployRequest`, then to whichever actor in `src/deploy.rs`
   consumes it.
3. Document the schema change in `README.md` next to the
   existing field table.

If the temptation is "this is a one-off, just take a flag," the
answer is no — that one-off undoes the property the CLI was
shaped around. The Nota record is reproducible, auditable, and
the same on every machine. A flag is none of those.

The trailing `Option<...>` fields (`builder?`, `substituters?`)
are the standard convention for optional positional fields;
they may be omitted entirely or written as `None` (for
`Option<NodeName>`) or `[]` / `[ name … ]` (for
`Option<Vec<NodeName>>`). Adding `None` for a list-shaped slot
is a type error.

---

## Local builds are unsigned

When `builder = None`, the build runs on the dispatcher and
`nix copy --to ssh-ng://<target>` carries the closure. If the
dispatcher has no `nix.settings.secret-key-files` (the current
CriomOS default — see CriomOS's `modules/nixos/nix.nix`), the
closure has no transferable signature and the target rejects
with "lacks a signature by a trusted key."

The full diagnosis and workarounds live in primary's
`skills/system-specialist.md` under "Cluster Nix signing."

The relevant code here is `src/copy.rs` — today plain `nix
copy --to`, no `--no-check-sigs` or pre-sign step. Changing
that is a deploy-shape change; do not patch it silently.

---

## Push and freshness

Push immediately after every commit. Operator-facing deploy
requests use human flake refs such as
`github:LiGoldragon/CriomOS/main`; freshness is handled by Nix
`--refresh` (which lojix passes to both `nix eval` and `nix
build`). Do not paste resolved commit hashes into request
examples, docs, or chat to satisfy freshness — the resolver
takes care of it.

---

## See also

- this repo's `ARCHITECTURE.md` and `README.md`.
- primary's `skills/system-specialist.md` for the cluster Nix
  signing situation that bites local-builder deploys.
- primary's `skills/autonomous-agent.md`.
- lore's `AGENTS.md` (workspace contract).
