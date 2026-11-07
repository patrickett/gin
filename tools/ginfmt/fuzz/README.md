# Frontend fuzzing

This standalone cargo-fuzz workspace does not add libFuzzer dependencies to normal builds.
With cargo-fuzz and a suitable Rust nightly toolchain installed:

```sh
cd tools/ginfmt/fuzz
cargo +nightly fuzz run frontend -- -max_len=4096 -timeout=10
```

The combined target checks lexer span progress and UTF-8 boundaries, parser completion, and
successful formatter output reparsing and idempotence. Invalid UTF-8 bytes are skipped because
these APIs take `&str`; malformed Gin syntax is not skipped.

Minimize any finding and add it to the UI `crashes/` fixtures with a diagnostic expectation.
Keep the fuzz seed too when it usefully exercises parser or formatter recovery.

Normal offline CI can run the bounded deterministic properties without cargo-fuzz:

```sh
cargo test --locked --offline -p ginfmt --test property_round_trip
```

Those tests enumerate short malformed inputs, exercise maximum-sized repeated inputs, and
generate valid programs that must format and reparse. They are not a substitute for a sustained
fuzz campaign.
