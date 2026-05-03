# Published Generated Inputs

## Question

Generated lojix inputs should not enter Nix as mutable local paths. Local
paths make the effective flake input identity depend on the dispatcher's
filesystem and undercut the intended eval-cache property:

```text
same generated content -> same NAR hash -> same flake input identity
```

The deploy path should be the same for local and remote builders.

## Decision

`lojix-cli` now treats each generated input as an independently published
flake source:

| Input | Role |
|---|---|
| `horizon` | Projected cluster/node data. |
| `system` | Target Nix system tuple. |
| `deployment` | System deploy shape such as home-on/home-off. |
| `home-wrapper` | Direct Home Manager wrapper for `HomeOnly`. |

Each input is written as a small source tree, hashed with Nix, archived,
published, and then referenced by Nix as an archive flake ref with the
full `narHash` in the query string.

Archive filenames use shortened content codes. The shortened code is
only a human-readable immutable name; Nix still verifies the full NAR
hash from the flake ref.

## Repo-Per-Input Direction

The archive shape deliberately models each generated input as its own
repository-shaped source tree: it has its own `flake.nix`, content
identity, published address, and lockable flake ref. That preserves the
cache-axis split without requiring GitHub repo automation in the first
implementation.

A later publisher backend can replace tarball publication with actual
generated Git repositories or branches per input kind. The deploy
pipeline should not change when that happens; only the publisher's
returned flake ref changes.

## Current Publisher Backend

The first backend publishes tar archives over SSH/rsync.

Defaults:

| Environment | Default |
|---|---|
| `LOJIX_ARCHIVE_SSH_TARGET` | `root@prometheus.goldragon.criome` |
| `LOJIX_ARCHIVE_REMOTE_DIR` | `/var/lib/lojix-inputs` |
| `LOJIX_ARCHIVE_BASE_URL` | `http://prometheus.goldragon.criome/lojix-inputs` |

The web server mapping for `LOJIX_ARCHIVE_BASE_URL` must exist on the
archive host. If publication or later fetching fails, the deploy fails;
there is no implicit local-path fallback.

## Home-Only Impact

`HomeOnly` still bypasses CriomOS. The wrapper flake no longer embeds
relative `path:./horizon` and `path:./system` inputs. It references the
published `horizon` and `system` archive refs. This keeps wrapper
staging independent from generated data directories and lets a remote
builder fetch the same input graph as the dispatcher.

## Remaining Work

The tarball backend is the first pass, not the durable end state.

Follow-up work:

- Add the archive host/service module in CriomOS so the default publish
  location is guaranteed to be served.
- Decide whether generated Git repositories replace the tarball backend
  or coexist as a second publisher.
- Add a lock-file export command for deploying from a generated input
  set when `lojix-cli` is unavailable.
