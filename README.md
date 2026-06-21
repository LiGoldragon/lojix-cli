# lojix-cli

`lojix-cli` is archived. It was the monolithic CriomOS deploy CLI used before the daemon-based `lojix` stack reached production.

The replacement is [`github:LiGoldragon/lojix`](https://github.com/LiGoldragon/lojix):

- `lojix-daemon` — long-lived deploy orchestrator with durable `sema-engine` state.
- `lojix` — ordinary-socket client for peer-callable reads such as `Query`.
- `meta-lojix` — owner/meta-socket client for privileged deploy and retention operations.
- `lojix-write-configuration` — bootstrap helper that encodes typed NOTA configuration into the daemon's binary startup file.

## Status

Archived / read-only. Do not add new deploy behavior here.

Historical behavior stays in git history for recovery. New deploy documentation, fixes, and operational runbooks belong in `github:LiGoldragon/lojix`, `signal-lojix`, `meta-signal-lojix`, CriomOS, or the system-maintainer reports depending on the layer touched.

## Migration target

Use the daemon stack. A read-only ordinary-socket query looks like:

```sh
lojix "(Query (ByNode (goldragon ouranos None)))"
```

Privileged deploys go through `meta-lojix` on the owner socket. The exact request variants and fields are defined by the deployed `signal-lojix` and `meta-signal-lojix` schemas. Prefer those contract schemas and the `lojix` repository README over this archived repo.

## Historical note

`lojix-cli` read one NOTA request, projected a cluster proposal through `horizon-rs`, materialized the generated flake inputs CriomOS expected, ran Nix, and optionally activated the result. It supported `FullOs`, `OsOnly`, and `HomeOnly` request forms directly in one process.

That role is now split across the daemon triad: request submission over typed sockets, durable deployment state in the daemon, and thin CLI clients per authority surface.
