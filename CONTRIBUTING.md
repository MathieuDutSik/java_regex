# Contributing to `java_regex`

Thanks for your interest. This crate exists to be **byte-for-byte compatible
with OpenJDK 25's `java.util.regex.Pattern`** — that constraint shapes most
contribution decisions, so this file walks through what that means in practice.

## Quick start

```sh
git clone https://github.com/MathieuDutSik/java_regex
cd java_regex
cargo test              # 109 lib tests + 61 jsonl + 8 proptest + 5 doctest
cargo doc --no-deps     # builds the rustdoc, including SPEC/QUIRKS/etc.
```

The crate is `no_std` with `alloc`; tests run on stable Rust 1.65+. No native
dependencies needed for the standard test suite.

## The compatibility commitment

The single most important property of this crate is that for any pattern,
input, and flag set, our output is identical to OpenJDK 25's. This is enforced
by:

- **`tests/jsonl_tests.rs`** — 61 hand-picked + fuzz-derived regression cases
  encoded as JSONL, each with a pattern, input, flags, expected match results.
- **`tests/proptest_invariants.rs`** — 8 self-consistency invariants run via
  `proptest` (no oracle needed; catches engine drift between operations).
- **`examples/diff_fuzz.rs` + `DiffOracle.java`** — the live differential
  fuzzer that compares against a running JVM. See
  [FUZZING.md](https://github.com/MathieuDutSik/java_regex/blob/main/FUZZING.md)
  for full details.

**If your change might affect match results, run the differential fuzzer for at
least 100 000 cases against your branch before submitting a PR.**

## Project layout

```
src/
  lib.rs        Public API: Regex, MatchInfo, PatternSyntaxError, Replacer
  parser.rs     Pattern string → AST (`Pattern` of `Node`s)
  engine.rs     AST + input → matches/captures (backtracking NFA)
  types.rs      AST node types, capture state, flags
  unicode.rs    `\p{…}` property tables, case folding, line terminators
  gen.rs        Arbitrary-driven AST generator (behind `fuzz-gen` feature)

tests/
  jsonl_tests.rs              Runs every case in *.jsonl
  proptest_invariants.rs      8 engine-self-consistency invariants
  *.jsonl                     Test corpora

examples/
  diff_fuzz.rs    Differential fuzzer vs OpenJDK
  diff_one.rs     Replay a single JSON-encoded failure case
  bench_engines.rs   Benchmark vs regex / fancy-regex / onig

.github/workflows/
  ci_NN_*.yml     One workflow per file, runs monthly on day NN
```

## Documentation map

- [README.md](README.md) — front door, install, examples.
- [SPEC.md](https://github.com/MathieuDutSik/java_regex/blob/main/SPEC.md) —
  full reference spec of OpenJDK regex behavior. Read this if you're touching
  parser/engine logic for a construct whose Javadoc semantics are unclear.
- [QUIRKS.md](https://github.com/MathieuDutSik/java_regex/blob/main/QUIRKS.md)
  — the 8 OpenJDK quirks we reproduce on purpose. Be aware of these; they look
  like bugs but they're correct.
- [DIFFERENCES.md](https://github.com/MathieuDutSik/java_regex/blob/main/DIFFERENCES.md)
  — the one intentional deviation (UTF-16 vs UTF-8 offsets).
- [FUZZING.md](https://github.com/MathieuDutSik/java_regex/blob/main/FUZZING.md)
  — how to run each of the four fuzz strategies.

## What good PRs look like

**Bug fixes** — by far the most welcome. The pattern is:

1. Reduce the failing input to a minimal pattern + input + flags triple.
2. Add a `#[test]` to `src/lib.rs` named `test_<descriptive_name>` that
   asserts the OpenJDK-correct behavior. Cross-reference the OpenJDK
   `Pattern.java` source class in the test docstring (e.g., "mirrors Java's
   `GroupCurly.match0` behavior").
3. Fix the code.
4. Run `cargo test`. If your test was a regression of an existing case,
   re-run the differential fuzzer on the seed that exposed it.

**Performance improvements** — welcome, with constraints:

- The backtracking NFA architecture is fixed; we deliberately do **not** use a
  DFA (it would conflict with backreferences and lookarounds).
- Run `cargo run --release --example bench_engines` before and after; include
  the numbers in the PR.

**API additions** — discuss in an issue first. The crate's public API mirrors
`java.util.regex.Matcher` deliberately; new methods should have a Java analog
to be considered.

**Architectural changes** — open an issue first. The engine encodes 8 OpenJDK
quirks and the documented capture-state semantics from
[SPEC.md §4](https://github.com/MathieuDutSik/java_regex/blob/main/SPEC.md#4-capture-group-state-model);
any rewrite needs to preserve all of them.

## What good PRs *don't* look like

- **"Fixing" an OpenJDK quirk.** The quirks in [QUIRKS.md] are intentional.
  Removing them breaks the compatibility commitment. If you think a quirk is
  wrong, that's a discussion for an issue, not a PR.
- **Adding `std`-only features.** The crate is `no_std`; new functionality
  should not require `std`.
- **Removing tests.** Even tests that look redundant stay — they're cheap to
  run and they pin down behavior the fuzz infrastructure has confirmed.
- **Mass reformatting / style-only changes** unrelated to a fix. They make
  history hard to read.

## Coding conventions

- `rustfmt` defaults. Run `cargo fmt --all` before submitting.
- `clippy` clean with `-D warnings`. Run `cargo clippy --all-targets
  --all-features -- -D warnings`. CI workflow `ci_02_clippy.yml` enforces
  this monthly.
- Comments explain **why**, not **what**. Most comments in `src/engine.rs`
  reference the corresponding OpenJDK source class (e.g., "mirrors
  `Curly.match0`'s i32-wrapping `temp < maxL` test") — match that style.
- `no_std`: use `alloc::` paths, not `std::`. The compiler enforces this
  except when the `fuzz-gen` feature is on.

## Running the test fleet

```sh
# Standard tests (used in CI and as the baseline for any change)
cargo test --release

# With the fuzz-gen feature (extra coverage of the gen.rs renderer)
cargo test --release --features fuzz-gen

# Proptest invariants with more cases
PROPTEST_CASES=5000 cargo test --release --test proptest_invariants

# Coverage report (requires cargo-llvm-cov)
cargo llvm-cov --all-features --all-targets --workspace --summary-only

# Differential fuzzer (requires javac and a JVM)
javac DiffOracle.java
cargo run --release --example diff_fuzz --features fuzz-gen -- 100000

# Benchmark (requires libonig; `sudo apt-get install -y libonig-dev` on Linux)
cargo run --release --example bench_engines
```

See
[FUZZING.md](https://github.com/MathieuDutSik/java_regex/blob/main/FUZZING.md)
for `cargo-fuzz` targets and the JVM-oracle setup in detail.

## CI

`.github/workflows/` contains 12 monthly workflows (one per file, one job per
file, cron at midnight on day N for `ci_NN_*.yml`):

| Day | Workflow | What it runs |
|---|---|---|
| 1 | `ci_01_tests.yml` | `cargo test --release --all-targets` |
| 2 | `ci_02_clippy.yml` | `cargo clippy --all-targets --all-features -- -D warnings` |
| 3 | `ci_03_benchmark.yml` | `cargo run --release --example bench_engines` |
| 4 | `ci_04_doc.yml` | `cargo doc --no-deps -D warnings`, missing-docs check |
| 5 | `ci_05_msrv.yml` | Rust 1.65 build + lib tests |
| 6 | `ci_06_no_std.yml` | `cargo build --target thumbv7em-none-eabi` |
| 7 | `ci_07_publish_dryrun.yml` | `cargo publish --dry-run` + exclude-list audit |
| 8 | `ci_08_coverage.yml` | `cargo-llvm-cov` summary + lcov artifact |
| 9 | `ci_09_audit.yml` | `cargo audit` against RustSec |
| 10 | `ci_10_semver.yml` | `cargo semver-checks` |
| 11 | `ci_11_macos.yml` | Build + tests on `macos-latest` |
| 12 | `ci_12_windows.yml` | Build + tests on `windows-latest` |

Each workflow has `workflow_dispatch` enabled so you can trigger it manually
from a fork.

## License

By contributing, you agree your contributions are licensed under the same
terms as the rest of the crate: **MIT OR Apache-2.0** (dual). See `LICENSE-MIT`
and `LICENSE-APACHE`.

## Getting help

- Open an issue at https://github.com/MathieuDutSik/java_regex/issues for bugs,
  feature requests, or questions about an OpenJDK behavior we may not be
  reproducing correctly.
- For mismatches you've reduced from a real Java codebase, attach the pattern,
  input, flags, and the Java and Rust outputs — that's the fastest path to a
  fix.
