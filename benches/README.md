# Design benchmark harness

These benchmarks are the shared public-API baseline for `lean_string` design
experiments. They cover move-aware collection, reserve and append paths, and
heap-layout-sensitive vector workloads.

Use the same target directory across the baseline and experiment worktrees so
Criterion can compare against saved samples.

Quick baseline (intended for local iteration):

```sh
CARGO_TARGET_DIR=/private/tmp/lean-string-perf-target \
  cargo bench --bench design -- --quick --save-baseline main
```

Quick comparison from an experiment branch:

```sh
CARGO_TARGET_DIR=/private/tmp/lean-string-perf-target \
  cargo bench --bench design -- --quick --baseline main
```

Full baseline (50 samples, 500 ms warm-up, and 2 s measurement per case):

```sh
CARGO_TARGET_DIR=/private/tmp/lean-string-perf-target \
  cargo bench --bench design -- --save-baseline main
```

Full comparison from an experiment branch:

```sh
CARGO_TARGET_DIR=/private/tmp/lean-string-perf-target \
  cargo bench --bench design -- --baseline main
```

Record the exact command, machine, toolchain, and raw Criterion output for every
scoreboard row. Do not compare quick samples with full samples.

## CodSpeed

The `CodSpeed` workflow runs every benchmark with the simulation and memory
instruments on pushes to `main` and on pull requests. To verify the integration
locally after installing `cargo-codspeed`:

```sh
cargo codspeed build --measurement-mode simulation --measurement-mode memory
cargo codspeed run --measurement-mode simulation
cargo codspeed run --measurement-mode memory
```

The ordinary `cargo bench` commands above still use Criterion's local
statistical measurements. CodSpeed must be connected to the public GitHub
repository before the workflow can upload results, and a run on `main` is
required before pull requests have a comparison baseline.

## Compact heap-header benchmark

The representation-level header experiment has a separate benchmark because it
constructs large vectors and takes materially longer than the shared API suite:

```sh
CARGO_TARGET_DIR=/private/tmp/lean-string-perf-target \
  cargo bench --bench compact_header -- --quick --save-baseline compact-header-base
```

CodSpeed's memory instrument records allocations for these vector workloads;
the local Criterion run measures time only.
