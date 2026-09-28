# Benchmarks

Production applications compare fusor, React, Svelte, Solid, Vue, Preact and
Leptos. Leptos builds in its own Cargo workspace under `workloads/leptos/`, so
it does not change fusor's dependencies, manifests or MSRV.
Read [the methodology](METHODOLOGY.md) before interpreting results. Performance
thresholds are not CI assertions; correctness and evidence integrity are.

## Layout

| Directory | Responsibility |
| --- | --- |
| `workloads/` | Equivalent applications and their production build inputs |
| `harness/` | Timing boundaries, correctness checks, memory, bundles and SSR |
| `tools/` | Shared recording, comparison, verification, diagnostics and publishing |
| `schemas/` | Versioned experiment, index and research-note formats, and the full-protocol registry |
| `results/history/<id>/` | JSON record plus its raw JSON evidence; no scripts or binaries |
| `results/index.json` | History catalog and explicit published-report pointer |
| `target/benchmarks/` (repository root) | Disposable exports, logs and profiling output |
| `.cache/benchmarks/cli.lock` (repository root) | Writer lock held while a command changes records |

## Retaining evidence

Keep recorded raw samples, memory snapshots, bundle hashes, build receipts and
decision notes. Different runs remain separate observations even when their
sources match. Completed records and their evidence are immutable: reformatting
JSON changes its recorded byte length and checksum. Historical downloads and
the published report are generated from this archive.

Use one investigation with multiple candidates for a related comparison, as
shown below. Preserve rejected candidates and failed diagnostics alongside the
decision. Keep scratch exports, logs and profiles under `target/benchmarks/`;
attach the evidence needed to support a decision before publishing it.

Generate future comparison tables from retained reports with the shared tools;
record the input reports and tool revision when citing them. Existing attached
comparisons remain part of their immutable records. Generated results and history
under `apps/benchmarks/public/` are disposable and stay ignored. Generated evidence
JSON is marked in `.gitattributes` to reduce review noise while notes and
validation summaries remain visible.

## Record a full run

From the repository root, install dependencies and build once:

```sh
npm ci
npm ci --prefix benchmarks
cargo fusor install -p fusor-playground
just bench-build
just bench-run my-comparison "Description of the change"
```

The run command preselects its sole full report, captures production hashes, runs
SSR and browser measurements sequentially, verifies sources/artifacts did not
change, and records the original JSON. It does **not** publish automatically.
Use `PLAYWRIGHT_CHANNEL=chromium` for Playwright's bundled browser; the default
is installed Chrome. Reproduce the published Binaryen settings described in the
methodology when comparing that build.

Review the data and add a note, then publish:

```sh
just bench note --id my-comparison --file /path/to/note.json
just bench publish --id my-comparison
just site
just preview
```

A note is ordinary data, not a new report-writing script:

```json
{
  "kind": "decision",
  "title": "Outcome",
  "format": "markdown",
  "body": "Describe the measured outcome, correctness checks, tradeoffs and limitations."
}
```

The shared renderer builds the latest report and history pages from these records.
Add all notes before publishing; completed records are immutable. Corrections go
in a follow-up experiment that identifies the previous record.

## Investigations with multiple candidates

```sh
just bench init --id investigation --title "Investigate a specific cost"
just bench note --id investigation --file hypothesis.json
just bench select --id investigation --name full-b --reason "Second complete run, chosen before measurement"
just bench add --id investigation --name candidate-a --report /path/to/screen.json --kind screen
just bench add --id investigation --name full-a --report /path/to/full-a.json --kind full
just bench add --id investigation --name full-b --report /path/to/full-b.json --kind full
just bench attach --id investigation --name correctness --file /path/to/validation.json --kind validation
just bench note --id investigation --file decision.json
just bench publish --id investigation
```

Record hypotheses before implementation and timing. Select the publication report
before it is measured; its recorded measurement timestamp must follow selection.
Record rejected candidates and failed diagnostics too. `attach` accepts diagnostic,
receipt, validation and failure JSON without pretending it is a ranked measurement.
A screen or error-bearing report cannot become the published full result.

Direct evaluator commands accept `BENCH_OUTPUT_DIR` and write there instead of
changing publication. Keep each output directory separate. The default scratch
location is `target/benchmarks/current/`. Do not run builds, profilers or multiple
benchmark processes concurrently.

## Shared tools

```sh
just bench verify
just bench compare --baseline baseline.json --candidate candidate.json --output comparison.json
just bench receipt --output receipt.json
just bench render --id investigation --output report.html
node benchmarks/tools/paired-native.mjs --baseline /path/to/baseline-ssr --candidate /path/to/candidate-ssr --output /path/to/pairs.json
node benchmarks/tools/paired-browser.mjs --baseline http://127.0.0.1:9001/workloads/fusor/ --candidate http://127.0.0.1:9002/workloads/fusor/ --output /path/to/pairs.json
node benchmarks/tools/profile-updates.mjs target/benchmarks/new-profile
```

Paired browser controls require separately served frozen builds. They exercise
initial render, bulk updates and fan-out with full value/identity/native-event
checks. The profiler is instrumented diagnostic work, never publication timing.
Native controls record executable hashes and alternate process order. Comparisons
retain environment differences and do not claim statistical significance.

For new paired diagnostics, record the exact producer command and source revision,
and attach build receipts for both frozen variants. A local checkout hash does
not identify the builds served at two URLs. Preserve the producer in shared tools
so another contributor can repeat its protocol. The all-metric and isolated paired
reports in `20260927-runtime-optimizations-3` used a different producer from the
current three-metric `paired-browser.mjs`; that command cannot reproduce their
protocol. Keep those historical observations with this limitation.

The CLI refuses collisions and serializes catalog writers with an exclusive lock.
After an interrupted process, inspect `.cache/benchmarks/cli.lock/owner.json` and
remove that lock directory only when its process is no longer running.

## Contributing

Full reports declare their protocol (`schemas/full-protocol.json`). The
six-framework `20260922-baseline` report is schema 1 without a `protocol` field
and stays valid under `six-framework-v1`. That protocol closed at
`2026-09-23T00:00:00Z`: a report generated later must be schema 2, declare
`seven-framework-v2`, and contain all seven frameworks' metrics, memory and
bundles to be recorded as full. A subset run remains a screen/smoke report.
Adding a framework means adding a new protocol version and closing the current one,
never editing a recorded report.

Change shared tools here; never add a script inside an experiment. Keep run
records data-only. Extend schemas deliberately when adding fields or protocols.
Add tests for evidence integrity and publication behavior, not timing thresholds.
Run:

```sh
just test-tools
just bench verify
just test benchmarks
```

The browser UI test requires built fixtures (`just fixtures`). The first
two checks run without building Rust or starting a browser.
