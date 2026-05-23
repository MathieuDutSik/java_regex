# java_regex
An implementation of the java regular expressions in Rust.

## Fuzzing

Three complementary fuzzing strategies are wired up:

| Strategy | Hunts for | Toolchain | Speed |
|---|---|---|---|
| **proptest** invariants (`tests/proptest_invariants.rs`) | engine self-consistency bugs (find positions, quote roundtrip, replace-with-identity, no-panic) | stable Rust | fast — runs in `cargo test` |
| **cargo-fuzz** targets (`fuzz/`) | parser/engine panics, infinite loops, OOM | nightly + `cargo install cargo-fuzz` | coverage-guided, runs indefinitely |
| **Differential fuzzer** (`examples/diff_fuzz.rs` + `DiffOracle.java`) | semantic divergence from OpenJDK `java.util.regex` | stable Rust + a JDK | ~5k–20k cases/sec |

The shared piece is `src/gen.rs`, a renderable `RegexNode` AST that all three strategies use to generate syntactically plausible patterns.

### Running the proptest invariants

```sh
cargo test --test proptest_invariants

# heavier run (40k cases per invariant)
PROPTEST_CASES=5000 cargo test --release --test proptest_invariants
```

If an invariant fails, proptest automatically shrinks the AST to a minimal failing example and prints both the original and the shrunk case.

### Running cargo-fuzz targets

One-time setup:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Then, from the project root:

```sh
cd fuzz
cargo +nightly fuzz run fuzz_parser   # raw-bytes parser robustness
cargo +nightly fuzz run fuzz_engine   # structured AST + input + flags
cargo +nightly fuzz run fuzz_replace  # replacement-string mini-syntax
```

Crashes land in `fuzz/artifacts/<target>/`. Reproduce with `cargo +nightly fuzz run <target> <artifact-path>`.

### Running the differential fuzzer

One-time setup (compile the JVM oracle):

```sh
javac DiffOracle.java
```

Then run the fuzzer (the `fuzz-gen` feature is required because the example uses `RegexNode`'s `Arbitrary` derives):

```sh
cargo run --release --example diff_fuzz --features fuzz-gen -- 20000
```

Optional environment knobs:

| Var | Default | Purpose |
|---|---|---|
| `DIFF_SEED` | random | Re-run a previous session deterministically |
| `DIFF_JAVA` | `java` (from PATH) | Path to a specific JVM binary |
| `DIFF_CLASSPATH` | `.` | Where to find `DiffOracle.class` |
| `DIFF_LOG` | (off) | Append each mismatch as a JSON line to this file |

Sample output:

```
diff_fuzz: seed=42 total=500 java=java
=== MISMATCH (split) ===
{ "pattern": "\\Q\\E", "flags": "i", "input": "\t", "op": "split" }
...
=== summary (seed=42) ===
  total iters         500
  ok                  411
  mismatches          89
  rust-only compile errs  0
  java-only compile errs  5
  both compile errs   1
```

The fuzzer compares matched text for every operation and compares match positions only on BMP-only inputs (Java's `Matcher` reports UTF-16 code-unit offsets, the Rust impl reports char offsets; the two agree on BMP). The known divergences documented in `DIFFERENCES.md` may surface as mismatches and can be filtered out at triage time.
