# Corpus diagnostics

The opt-in `corpus` integration test checks each package once and checks standalone `.gin`
files separately. It invokes real compiler executables in check-only mode. Invalid programs
are observations, not a failing corpus run; a compiler crash, timeout, or malformed diagnostic
protocol fails the run.

Record the current compiler's results:

```sh
GIN_CORPUS_PATHS="$PWD/modules" \
GIN_CORPUS_REPORT=/tmp/gin-corpus.json \
cargo test --locked --offline -p ginc --test corpus corpus_diagnostics -- --ignored --nocapture
```

Compare against a real, separately built baseline:

```sh
GIN_CORPUS_BASELINE=/absolute/path/to/baseline/ginc \
GIN_CORPUS_REPORT=/tmp/gin-corpus-diff.json \
cargo test --locked --offline -p ginc --test corpus corpus_diagnostics -- --ignored --nocapture
```

Any changed acceptance or diagnostic makes a comparison fail after writing the report.
Both additions and removals are changes: review them rather than automatically accepting them.
Without `GIN_CORPUS_BASELINE`, the report says `"compared": false`; it is not differential
evidence. The retired compiler is not rebuilt or emulated.

Options:

- `GIN_CORPUS_COMPILER`: current compiler executable; defaults to Cargo's built `ginc`.
- `GIN_CORPUS_PATHS`: platform-separated input paths; defaults to `modules` and the native
  regression fixtures. Sources are compiled as written, not expanded using UI revision gates.
- `GIN_CORPUS_ARGS`, `GIN_CORPUS_BASELINE_ARGS`: JSON arrays of compiler arguments. Defaults
  are `["--emit","check","--message-format=json","--warnings=deps","--quiet"]`; the input path
  is appended. A baseline must expose the same JSON diagnostic protocol.
- `GIN_CORPUS_REPORT`: report destination; otherwise printed with `--nocapture`.

Traversal excludes symlinks, hidden entries, `target`, and `node_modules`. Inputs and diagnostics
are sorted. Diagnostic paths inside this repository are made relative; codes, severity, ranges,
messages, dependency provenance, and acceptance are compared. Compiler paths and timing are not.
Each invocation has a 60-second limit and a 16 MiB limit on each output stream.
