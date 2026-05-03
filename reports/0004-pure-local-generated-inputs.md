# Pure Local Generated Inputs

## Question

Generated lojix inputs should preserve the flake-cache identity property
without introducing a publication backend before the storage boundary is
settled.

## Decision

`lojix-cli` materializes generated inputs locally and passes them to Nix
as `path:` flake refs with `narHash` query parameters.

Generated inputs:

| Input | Role |
|---|---|
| `horizon` | Projected cluster/node data. |
| `system` | Target Nix system tuple. |
| `deployment` | CriomOS system deploy shape, used by `FullOs` and `OsOnly`. |

The full NAR hash stays in the flake ref so Nix can key the input by
content even though the address is local.

## Home Boundary

`HomeOnly` evaluates `CriomOS-home` directly. There is no generated
Home Manager wrapper in `lojix-cli`.

`CriomOS-home` owns its standalone `homeConfigurations` output and
receives the same generated `horizon` and `system` inputs as CriomOS.
When CriomOS consumes `CriomOS-home` for `FullOs`, CriomOS passes those
inputs through with `follows`.

## System Input Direction

`system` can later become a real generated-input repo with branches
named by their contained system, such as `x86_64-linux` and
`aarch64-linux`. That is a clean simplification because the system input
has a tiny finite value space and is not per-node data.
