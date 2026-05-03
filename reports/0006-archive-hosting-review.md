# Archive Hosting Review

## Scope

This reviews the state recorded in
`reports/0005-handoff-pure-inputs-direct-home.md` and the current
`lojix-cli`, `CriomOS-home`, and `CriomOS` implementation.

The accepted first pass is local generated inputs:

```text
generated flake directory -> NAR hash -> path:<directory>?narHash=<full-sri>
```

That shape fixes flake input purity for local redeploys, but it is not
the portable build shape.

## Implementation Gaps

1. Remote builders still receive local generated input paths.

`NixBuild::execution_invocation` wraps the same Nix command in SSH when a
builder is selected. The `--override-input horizon` and
`--override-input system` values are still `path:` refs into the
dispatcher user's cache. On a remote builder those paths only work if the
same generated directories already exist at the same absolute paths. That
is not a portable contract.

2. `FlakeInputRef` has only one address kind.

`FlakeInputRef::from_local_path` stores a local `path:` URL plus NAR hash.
There is no type for a published tarball input, no distinction between
local-only and remotely-fetchable inputs, and no policy layer that chooses
which address kind a deploy requires.

3. The README still contains stale HomeOnly wrapper text.

The code now evaluates:

```text
<home-flake>#homeConfigurations.<user>.activationPackage
```

The README's status section and `HomeOnly` subsection still mention a
generated Home Manager wrapper and `packages.<system>.activationPackage`.
Later README text says the corrected direct `CriomOS-home` shape. The
documentation is internally inconsistent.

4. `system` and `deployment` do not need the same storage answer as
`horizon`.

`system` has a tiny finite value space. A small repo with branches named
by contained system remains cleaner than publishing per-deploy archives.
`deployment` has two current values and is CriomOS-system-only. It can be
a static repo, static object, or remain generated locally until the
system deploy path needs remote portability.

5. Horizon is the real storage problem.

The projected horizon is per cluster/node/proposal and contains generated
content. Repeated git commits or branches create a long version chain of
mostly repeated data. That is an awkward source-control shape even if git
packfiles deduplicate storage internally, and Nix's `github:` fetcher
downloads tarball snapshots rather than preserving an incremental
content-addressed deployment flow.

6. Public horizon archives are a data policy decision.

A public archive URL makes the projected horizon public. Nix needs the
plain flake tree to evaluate, so this cannot be solved by encrypting the
archive unless every builder has a decryption fetch layer before Nix sees
the input. If horizon content is not public data, the storage answer needs
authenticated builders, not public buckets.

7. Lockable deployment without `lojix-cli` is not implemented.

The current CLI passes override inputs directly to Nix. It does not emit
a small deployment flake/lock that can be built later without the CLI.
Once generated inputs have remotely-fetchable refs, emitting such a
lockable bundle becomes straightforward and avoids putting generated
horizon content into git history.

8. Test coverage does not prove remote input portability.

Current tests assert command shape and local eval behavior. Missing tests:

- remote builder invocation uses no local `path:` generated inputs;
- `HomeOnly` direct evaluation never reintroduces a wrapper;
- tarball flake refs with `narHash` are accepted;
- published refs keep full hashes out of operator-facing request text.

## Nix Input Mechanics

Nix supports tarball flake refs over HTTP(S) and `file://`. Archive
extensions including `.tar.zst` are recognized as tarballs. The generic
`narHash` flake attribute is explicitly for flake types such as tarballs
that lack a unique content identifier.

Local validation on Nix 2.34.6 confirmed this shape:

```text
tarball+file://<archive>.tar.zst?narHash=<full-sri>#value
```

The Nix manual also states that `narHash` lets Nix compute the input's
store path and enables flake inputs to be substituted from a binary cache.
That means archive hosting and Nix binary caching can complement each
other:

- archive URL gives every builder an original fetch location;
- NAR hash gives reproducibility and cache substitution;
- a binary cache can avoid archive download when the source tree is
already substituted.

## Hosting Options

### GitHub Source Archives

GitHub can serve source snapshots for a branch, tag, or commit as zip or
tarball archives. These snapshots do not include repository history.
GitHub documents that source archives are generated on request, cached for
a while, and may later be regenerated. For reproducibility GitHub
recommends commit IDs; for security-stable archives GitHub recommends
release assets instead.

This works well for normal source repositories. It is not the right shape
for per-deploy projected horizons because it still needs a git history or
tag/branch namespace to name every generated version.

### GitHub Release Assets

GitHub Releases can host arbitrary assets. Current GitHub docs state up
to 1000 assets per release, each under 2 GiB, with no total release size
or bandwidth limit.

This can work as a bootstrap archive host:

```text
https://github.com/<owner>/<repo>/releases/download/<release>/<short>.tar.zst
```

It is operationally convenient if avoiding cloud object-storage accounts
matters. It is not a clean content-addressed object store:

- asset namespace management is release-shaped, not CAS-shaped;
- assets can be deleted or replaced by maintainers;
- the 1000-assets-per-release limit forces sharding;
- GitHub policy/abuse limits are less explicit than object-storage
pricing.

Nix's full `narHash` still protects correctness if an asset is replaced.

### AWS S3

S3 is still relevant. It remains the baseline object-storage API and AWS
documents strong read-after-write consistency for object writes and reads.

For this use case the downside is cost shape: public deploy inputs are
downloaded by builders over the internet, and AWS S3 pricing includes
data-transfer charges for many outbound paths. S3 is a good answer when
the system is already AWS-native; it is not the simplest public archive
host for repeated Nix fetches.

### Cloudflare R2

R2 is the best first public archive host for this problem.

Cloudflare documents R2 as S3-compatible, strongly consistent, and having
no egress bandwidth fees. Public buckets can be exposed through a custom
domain; the Cloudflare-managed `r2.dev` URL is intended for development
and is rate-limited. A custom domain can use Cloudflare Cache.

The useful shape:

```text
https://inputs.example.net/horizon/<short-nar>.tar.zst
```

The object key is content-derived. Upload is put-if-absent. Existing
objects are never overwritten. The full SRI NAR hash remains in the Nix
flake ref query where Nix needs it, while human-facing names stay short.

### Backblaze B2

B2 is S3-compatible and can serve through Cloudflare CDN with no download
fees from Backblaze. It is a reasonable fallback if B2 is already in use.
Compared with R2 it adds another provider boundary for a use case where
Cloudflare already owns the public edge and object store.

### Tigris

Tigris is S3-compatible, globally distributed, and advertises zero egress
fees. It is interesting, especially for globally distributed object
access, but it is a newer/smaller dependency than R2. Prefer R2 first
unless Tigris's global placement model becomes load-bearing.

### IPFS / Pinning Services

IPFS is genuinely content-addressed. CIDs are derived from content, and
HTTP gateways can serve `https://<gateway>/ipfs/<cid>`.

It is not the first deploy-critical answer:

- availability depends on pinning and gateway behavior;
- public gateways add another reliability variable;
- Cloudflare's old public IPFS gateway was retired in 2024;
- Nix still needs an HTTP URL unless native IPFS support is introduced
  elsewhere.

IPFS can be a mirror later, not the primary first implementation.

### Cachix / Attic

Cachix and Attic solve a different but adjacent problem: Nix binary cache
hosting. Cachix documents pushing flake inputs with `nix flake archive`.
Attic is self-hostable, backed by S3-compatible storage, and has global
deduplication over its NAR/chunk store.

For generated inputs, a binary cache should be a complement, not the only
origin. Builders still need an original flake URL on cache miss. The
clean deployment shape is:

```text
published tarball URL + narHash + optional binary-cache substitution
```

## Recommended Shape

1. Keep `lojix-cli` as an orchestrator, not an archive backend.

The CLI can select a remotely-fetchable input address, but object storage
upload logic should live behind a small storage component/library or a
dedicated publisher command. The CLI should not grow a server.

2. Add an address type before adding provider code.

The missing noun is a generated input address:

```text
GeneratedInputAddress =
  LocalPath(path, narHash)
  PublishedTarball(url, narHash)
```

`NixBuild` should consume this address type and not know whether the input
came from local cache, R2, GitHub Releases, or a test fixture.

3. Publish `horizon` first.

`system` should move to a tiny branch-per-system repo or equivalent static
flake. `deployment` can remain generated for local-only system deploys
until remote system deployment needs the same portability. Publishing all
three at once would hide the actual hard problem.

4. Use R2 with content-derived immutable object keys.

Archive creation must be deterministic. Object naming should use a short
NAR-derived code, with collision handling that extends the prefix if an
existing object has different metadata. The full NAR hash belongs in
machine-generated flake refs, not in operator requests or prose.

5. Add a lockable deployment bundle after published refs exist.

Once `horizon`, `system`, and `deployment` have URL-shaped refs, `lojix`
can emit a tiny deployment flake that pins:

- the target root flake (`CriomOS` or `CriomOS-home`);
- `horizon`;
- `system`;
- `deployment` when applicable.

That gives the user's "build without lojix-cli" property without storing
repeated generated horizons in git.

## Sources

- Nix flake reference manual: `narHash`, tarball flake refs, and binary
  cache substitution:
  https://nix.dev/manual/nix/stable/command-ref/new-cli/nix3-flake.html
- GitHub source archive behavior:
  https://docs.github.com/en/repositories/working-with-files/using-files/downloading-source-code-archives
- GitHub release asset limits:
  https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases
- Cloudflare R2 architecture:
  https://developers.cloudflare.com/r2/how-r2-works/
- Cloudflare R2 pricing:
  https://developers.cloudflare.com/r2/pricing/
- Cloudflare R2 public buckets:
  https://developers.cloudflare.com/r2/buckets/public-buckets/
- Amazon S3 overview and consistency:
  https://docs.aws.amazon.com/AmazonS3/latest/userguide/Welcome.html
- Amazon S3 pricing:
  https://aws.amazon.com/s3/pricing/
- Backblaze B2 public bucket through Cloudflare:
  https://help.backblaze.com/hc/en-us/articles/13560118643099-Delivering-Content-From-a-Public-Backblaze-B2-Bucket-via-Cloudflare-CDN
- Tigris pricing:
  https://www.tigrisdata.com/pricing/
- IPFS content addressing:
  https://docs.ipfs.tech/concepts/content-addressing/
- IPFS web gateway addressing:
  https://docs.ipfs.tech/how-to/address-ipfs-on-web/
- Cloudflare public IPFS gateway retirement:
  https://blog.cloudflare.com/cloudflares-public-ipfs-gateways-and-supporting-interplanetary-shipyard/
- Cachix pushing flake inputs:
  https://docs.cachix.org/pushing
- Attic introduction:
  https://docs.attic.rs/
