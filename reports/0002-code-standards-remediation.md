# Code Standards Remediation

## Scope

This report records the code-standard review after the first
deploy-kind implementation and the remediation plan for bringing the
fork into the lore Rust style.

The relevant standard is lore's Rust style:

- behavior lives on types;
- domain values are typed;
- boundaries take and return one object;
- errors are structured crate-local enum variants;
- naming uses full English words by default.

The repo-specific style additions are:

- no free reusable functions outside `main`;
- typed newtypes at boundaries after CLI decode;
- one object in, one object out at actor boundaries;
- tests live under `tests/`.

Test fixtures may use free helper functions. They are not part of the
runtime API and are acceptable when they keep an integration test
readable.

## Current Shape

The deployed first cut works:

- Nota exposes `FullOs`, `OsOnly`, and `HomeOnly`.
- CriomOS receives a `deployment` override input.
- `HomeOnly Build`, `Profile`, and `Activate` work locally.
- Builder validation rejects unsupported home builders.
- Formatting, clippy, and tests pass.

That passing state does not mean the code satisfies the style
contract. The failures are structural rather than compiler-lint
failures.

## Findings

### Command Invocation Is Untyped

Several public or semi-public methods return anonymous command tuples:

```text
(&str, Vec<String>)
(String, Vec<String>)
Option<(&str, Vec<String>)>
```

This appears in the Nix build, closure copy, system activation, and
home activation paths. The lore standard rejects anonymous tuple
returns at boundaries because the tuple fields are unnamed and because
the command being invoked is itself a domain object.

Target shape:

```text
ProcessInvocation {
  program,
  arguments,
}
```

The invocation owns execution behavior:

```text
invocation.capture_stdout(...)
invocation.inherit_stdio(...)
invocation.to_remote_shell_command()
```

### Build Outputs Erase Domain Meaning

The current build phase stores the eval result as a plain string and
the deploy outcome as one `stdout` string for both eval and build
outputs. That loses the distinction between:

- a Nix derivation path;
- a realized store path;
- the CLI text printed for the operator.

Target shape:

```text
DerivationPath
StorePath
DeployOutcome::Evaluated { derivation_path }
DeployOutcome::Realized { store_path }
```

The CLI printing step is the only place where those values become
plain output text.

### Error Variants Are Too Generic

Several errors are currently reported through an adjacent failure
domain:

- activation API misuse is reported as a Nix failure;
- local hostname failure is reported as SSH failure;
- system profile symlink parsing is reported as SSH failure;
- actor RPC failures collapse into a string.

Target shape:

```text
InvalidSystemActivation { action, reason }
LocalHostnameFailed { status, stderr }
InvalidSystemProfileLink { got }
ActorRpcFailed { operation }
ActorMessagingFailed { operation, message }
```

The exact variant names can change, but each error should name the
failure domain directly.

### Reusable Free Functions Own Real Behavior

The runtime has reusable free functions for:

- running local and SSH processes;
- quoting shell arguments;
- parsing a system profile link;
- unwrapping actor RPC results;
- computing a directory NAR hash.

Small local helpers are allowed, but these are not small local
fragments. They are process, shell, profile, RPC, or artifact concepts
and need owning types.

Target owners:

```text
ProcessInvocation
ShellCommand
SystemProfileLink
ActorCall
ArtifactDirectory
```

### Artifact Materialization Uses A ZST Method Holder

`HorizonArtifact` is both a ractor actor marker and a namespace for a
real materialization method. Lore permits ZSTs as framework markers,
but not as method holders for runtime work.

Target shape:

```text
ArtifactMaterialization {
  horizon,
  cluster,
  node,
  deployment_shape,
}
```

`HorizonArtifact` remains only the actor implementation. It delegates
to the data-bearing materialization request.

### Actor Messages Carry Wide Payloads

Some actor message variants carry multiple independent fields instead
of one named request object. That weakens the actor boundary and
duplicates construction logic.

Target shape:

```text
ArtifactMsg::Materialize {
  materialization,
  reply,
}
```

The same principle applies to deploy-internal staging and finishing
inputs.

### Naming Still Uses Abbreviations

Runtime identifiers should use full English words unless an exception
is named. `argv`, `drv`, `gen`, `out`, and generic single-letter
variables should be replaced in runtime code.

Accepted exceptions:

- `ssh`, `efi`, `nix`, `os`, `uri`, and `json` are general technical
  acronyms;
- short local loop names are allowed in tight scopes;
- test fixture helpers may be pragmatic when they do not leak into the
  runtime API.

## Remediation Order

1. Introduce typed process invocation and shell command objects.
2. Replace anonymous command tuple returns with `ProcessInvocation`.
3. Introduce `DerivationPath`, typed deploy outcomes, and explicit CLI
   rendering.
4. Move process execution helpers onto `ProcessInvocation`.
5. Move system profile parsing onto `SystemProfileLink`.
6. Replace generic error variants at the identified sites.
7. Introduce data-bearing artifact materialization and actor message
   payloads.
8. Rename runtime abbreviations touched by the refactor.
9. Run formatting, clippy, tests, and flake check.

## Non-Goals

This remediation does not change the operator-facing Nota schema.

This remediation does not redesign remote `HomeOnly`; it remains
unsupported for builders.

This remediation does not make every system action SSH-break-proof.
Report 0001 records that desired shape, but this pass is about code
standards and typed boundaries.
