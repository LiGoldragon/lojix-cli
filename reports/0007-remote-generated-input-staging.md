# Remote Generated Input Staging

## Context

Remote builder execution previously wrapped the same Nix command in SSH
that local builds used. That meant generated `horizon`, `system`, and
`deployment` override inputs stayed as dispatcher-local `path:` refs.

For a remote builder, that only works if the same generated directories
already exist at the same absolute paths on the builder. That is not a
valid remote-build contract.

## Shape

When a builder node is selected, `lojix-cli` now stages generated inputs
onto that builder before running Nix there.

The staging path is content-keyed:

```text
/var/tmp/lojix/generated-inputs/<input-name>-<short-nar>_...
```

Each generated input is copied with:

```text
ssh <builder> mkdir -p <remote-input-directory>
rsync -a --delete <local-input-directory>/ <builder>:<remote-input-directory>/
```

The Nix invocation that runs on the builder receives `path:` refs to the
remote staged directories, preserving the existing NAR hash suffixes.

## Effect

`builder == node` now means:

1. project and materialize generated inputs on the dispatcher;
2. rsync those small generated inputs to the target/builder;
3. run `nix build` on the target/builder;
4. skip closure copy because the closure is already on the activation
   target;
5. activate on the target.

This is the immediate safe path for deploying hosts whose closures
contain large locally-cached model files. The large closure is built and
kept on the target builder instead of being realized on the dispatcher.

## Remaining Storage Design

This does not replace the archive-hosting design in
`reports/0006-archive-hosting-review.md`.

Rsync staging is the practical SSH-builder path. Published tarball inputs
are still the portable build path for builders that should not depend on
the dispatcher being able to stage files over SSH.
