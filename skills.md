# Skill — lojix-cli

*Archived legacy CriomOS deploy CLI.*

---

## What this skill is for

Use this only when reading or recovering historical behavior from the
old monolithic deploy CLI. Do not add new deploy behavior here.
Active deploy work belongs in `github:LiGoldragon/lojix`,
`signal-lojix`, and `meta-signal-lojix`.

---

## The CLI is one Nota record

The whole operator surface is a single Nota record decoded by
`src/request.rs`. **No flags. No subcommands. No env-var
dispatch. No custom argv parser.** The top-level record heads
(`FullOs`, `OsOnly`, `HomeOnly`, `CheckHostKeyMaterial`) and
their positional fields ARE the API.

The first three deploy; `CheckHostKeyMaterial` is a non-deploy
read-only diff (see "Non-deploy verbs" below). `src/main.rs`
branches on the variant.

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

---

## Non-deploy verbs

Not every operator verb deploys. `CheckHostKeyMaterial`
(in `src/check.rs`) is the first: an orchestrator-side
read-only diff between horizon-rs's per-host projection and
the host's on-disk `publication.nota` (which clavifaber writes
during provisioning). It SSHes the host, cats the publication,
parses it via `clavifaber::publication::PublicKeyPublication`,
and prints per-key mismatches with operator hints.

Why orchestrator-side, not host-side: the host's clavifaber
stays cluster-unaware — it only knows its own keys, not what
the cluster DB expects of it. Asking "does the host's material
match the cluster?" only the orchestrator can answer.

Shape rules for non-deploy verbs:

- Variant on `LojixRequest` with a `NotaRecord`-derived struct.
- `src/main.rs` matches the variant and runs the right path.
  Exit code 0 on clean diff, 3 on mismatches, 1 on error (so
  `if lojix ... ; then` is a meaningful operator gate).
- No mutation. If a verb would mutate the host, route it
  through clavifaber's NOTA surface — don't grow a second.

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

## Copy carries `--substitute-on-destination`

`src/copy.rs` always passes `--substitute-on-destination` to
`nix copy`. Effect: the target tries each path against its own
substituters (the cluster HTTP cache) before accepting it from
the source. The cluster cache (`nix-serve`) signs paths over
HTTP; raw `ssh-ng` daemon-to-daemon transfer carries no
signatures unless the source already has them.

**Consequence for deploy shape**: route builds through a cache
node — `builder = <cache>` in the Nota request — so the cache
has the closure to serve. `builder = None` only works if the
dispatcher itself signs (it doesn't, in current CriomOS), so
prefer `builder = prometheus` (or whichever node serves the
cluster cache).

The full diagnosis lives in primary's
`skills/system-specialist.md` under "Cluster Nix signing,"
including the key-generation procedure and the still-pending
CriomOS module change to wire `nix.settings.secret-key-files`
that would make `builder = None` work too.

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
