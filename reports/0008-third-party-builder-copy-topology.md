# Third-party Builder Copy Topology

`lojix-cli` supports a builder node distinct from the dispatcher and the
deployment target. The current implementation builds on that builder, but
the closure copy is still orchestrated by the dispatcher.

## Current Shapes

```
builder omitted

dispatcher
  nix build
  nix copy --to target
  ssh target activate
```

```
builder == target

dispatcher
  ssh target nix build
  ssh target activate

copy skipped because the result is already on the target
```

```
builder != target

dispatcher
  ssh builder nix build
  nix copy --from builder --to target
  ssh target activate
```

The third shape is the bad one. `nix copy --from builder --to target`
runs on the dispatcher. Nix opens one SSH store connection to the builder
and one SSH store connection to the target, then streams the NAR data
through the dispatcher process.

The logical data path is:

```
builder -> dispatcher -> target
```

On a network where the dispatcher reaches the target through the builder
as a router, the physical path becomes:

```
builder -> dispatcher -> builder-as-router -> target
```

That is a poor fit for large system closures. It avoids realizing the
closure in the dispatcher's store, but it still puts all copied bytes on
the dispatcher's network path.

## Current Best Use

For large closures whose intended source is a powerful builder, run
`lojix-cli` on that builder and leave the builder field omitted or set to
`None`. Then the builder is also the dispatcher:

```
builder
  nix build
  nix copy --to target
  ssh target activate
```

This gives the desired data path:

```
builder -> target
```

## Replacement Shape

The third-party builder path should move the copy phase onto the builder:

```
dispatcher
  ssh builder nix build
  ssh builder nix copy --to target
  ssh target activate
```

The builder then pushes directly to the target:

```
builder -> target
```

The SSH credential question is the load-bearing part of that change.
Two viable shapes:

- Forward the dispatcher's SSH agent into the builder for the copy
  command. This keeps private keys off the builder, but a fully trusted
  builder root can use the forwarded agent while the session is alive.
- Give trusted builders their own deploy identity authorized on targets.
  This is cleaner for unattended operation and avoids agent-forwarding
  exposure, but requires explicit key distribution and revocation.

The implementation point is `src/copy.rs`: `ClosureCopy` currently emits
dispatcher-side `nix copy --from <builder> --to <target>` for
`builder != target`. The replacement needs a builder-side invocation for
that case, plus tests that assert the copy command runs through SSH to
the builder.
