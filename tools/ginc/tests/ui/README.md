# ginc UI tests

Every `.gin` fixture has a sibling `.diag` file, empty when there are no diagnostics.
Snapshots contain severity, diagnostic code, one-based line/Unicode-column span, and message.
They omit temporary paths and presentation such as colors and timings.

Run and review updates with:

```sh
cargo test --locked --offline -p ginc --test ui
GIN_BLESS=1 cargo test --locked --offline -p ginc --test ui
```

Blessing stages the whole suite's outputs until status, annotations, runtime output, IR
emission, and tidy have passed. It never repairs expectations for a target skipped by this
host. Review the changed snapshots; blessing is not approval.

Directives are standalone Gin comments. Legacy `// ui:` headers are also understood, but
are blanked before compilation; ordinary Gin `-- ui:` comments require no preprocessing.

```gin
-- ui: compile-fail
value() in 0 to 9:
    return missing --~ ERROR source-check-unknown-name
```

- `check-pass` (default): syntax and semantic checks succeed; lowering is not required.
- `compile-fail`: exit 1 with an error diagnostic. A panic, signal, malformed JSON protocol,
  missing completion record, or other abnormal status is always a test failure.
- `run-pass exit-code=5 stdout=""`: native build and run; defaults are exit 0 and empty
  stdout. Quoted text supports `\n`, `\\`, and `\"`.
- `snapshot=mlir`: additionally snapshot the recorded checked bodies (`.body`) and lowered
  MLIR (`.mlir`), the input and output of lowering. These are not snapshots of every LLVM
  optimization pass. Available for passing tests only.
- `only-target=x86_64-unknown-linux-gnu`: run only on that exact host triple, with an explicit
  skip report elsewhere. Target names must be supported by Gin. Snapshots must still exist.
- `revisions=one,two`: run separate variants with `<fixture>.one.diag`, `.one.body`, etc.
  Put `--@[one]` and `--@[two]` on their own lines. A gate selects subsequent lines until
  the next gate; lines before the first gate belong to every revision. Each named revision
  must have a gate. Excluded lines are blanked without shifting UTF-8 offsets or line numbers.

Inline `--~ ERROR <code>` annotations refer to the same line when placed after source.
A standalone annotation refers to the immediately preceding line. Markers inside strings
are not annotations. Multiple error codes may match one line; annotations consume distinct
diagnostics. Full snapshots catch unexpected additional diagnostics.

Fixtures in `crashes/` must fail compilation with a normal exit status; a signal
termination or panic is a test failure.

Tidy checks every fixture's required snapshots, including skipped targets, and rejects orphan
`.diag`, `.body`, `.mlir`, `.stderr`, and `.stdout` files. File discovery rejects symlinks and
has size/count limits. Each compiler or program invocation has a 60-second deadline, null
stdin, and 16 MiB limits on each captured stream.

These fixtures seed a migration, not a claim that all existing Rust checker tests were
converted. Keep specialized API, ownership, and negative-control tests until equivalent
fixture coverage has been demonstrated.
