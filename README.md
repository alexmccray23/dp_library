# dp_library

A Rust library and command-line toolkit for processing survey data, ported from
a set of legacy Perl tools. It provides parsers and evaluators for the file
formats used by CfMC-style survey systems (RFL layouts, CFMC logic expressions,
Uncle E-files) along with binaries for the most common day-to-day data
processing tasks: frequency reports, counter checks, banner generation, and
case weighting.

## Features

The library (`src/lib.rs`) exposes four main modules:

- **`rfl`** — Parser for `.rfl` (Record Format Layout) files. Reads question
  metadata, response codes, and column positions, and classifies questions as
  `Fld` (categorical), `Var`, `Num`, or `Exp`.
- **`cfmc`** — Lexer, parser, and evaluator for CFMC logic expressions used in
  survey skip logic and filters. Supports comparison operators (`=`, `<>`, `<`,
  `>`, `<=`, `>=`), logical operators (`AND`, `OR`, `NOT`), arithmetic
  (`+`, `-`, `*`, `/`), and special operators such as `NUMITEMS`, `^^B`, and
  `^^NB`.
- **`crosstabs`** — Reads banner specifications from Excel workbooks
  (via `calamine`) and generates crosstab banner definitions against an RFL
  layout.
- **`weight`** — IPF (iterative proportional fitting) raking implementation
  for weighting survey data based on Uncle E-file weight tables. Includes a
  parser for E-files (`parse_e`), an Uncle expression parser (`uncle`), and
  multi-pass raking on top of [`ipf_survey`](https://crates.io/crates/ipf_survey).

## Binaries

Four command-line tools are built from `src/bin/`:

### `freq`
Generate frequency tables for one or more data fields.

```sh
freq -l p0042.rfl -d P0042.FIN QD7B AGEGROUP
freq -l p0042.rfl -d P0042.FIN -f "QD7B(02) AND AGEGROUP(1-3)" QD7B
```

Flags:
- `-l, --layout-file` — RFL layout file
- `-d, --data-file` — Fixed-width data file
- `-f, --filter` — CFMC-style case filter expression
- `-v, --verbose` — Show full (untruncated) response labels

### `chkcounts`
Verify that the responses in a data file match the counts declared in a
counters file, using CFMC logic from the layout.

```sh
chkcounts -l p0001.rfl -d p0001.fin -c counters.chk [-v]
```

### `banners`
Generate crosstab banner definitions from an Excel specification and an RFL
layout.

```sh
banners spec.xlsx layout.rfl banners.txt -f pos
```

Files can be passed either via `-l/-o` flags or positionally — the binary
detects them by extension (`.xlsx`/`.xls`, `.rfl`, `.txt`/`.e`). The `-f/--footer`
flag selects the footer style (`nbc`, `nmb`, `nbs`, `r2r`, `pos`).

### `weight`
Apply IPF weighting to a data file based on a weighting control table from an
Uncle E-file. Outputs a `.WT` file (or a custom path via `-o`).

```sh
weight 600 -e study.E -d study.fin
weight 620 -e study.E -d study.fin -o study.WT
```

The `-l/--layout-file` option is only required when the E-file uses CFMC-style
logic expressions that need to be resolved against an RFL layout.

## Building

```sh
cargo build --release
```

The release profile enables LTO and stripping. Binaries land in
`target/release/{freq,chkcounts,banners,weight}`.

### Static binaries via musl

For deployment to environments with old or mismatched glibc versions (or no
glibc at all), build against the `musl` target to produce fully
statically-linked binaries with no runtime library dependencies:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

The resulting binaries land in
`target/x86_64-unknown-linux-musl/release/{freq,chkcounts,banners,weight}` and
can be copied to any Linux x86_64 host and run without installing anything.
Verify with `ldd target/x86_64-unknown-linux-musl/release/freq` — it should
report `not a dynamic executable`.

On most distros you'll need the musl toolchain installed first (e.g.
`musl-tools` on Debian/Ubuntu, `musl` on Arch/Manjaro).

## Using as a library

Add to `Cargo.toml`:

```toml
[dependencies]
dp_library = "0.1"
```

```rust
use dp_library::{CfmcLogic, RflFile};

let layout = RflFile::parse("p0042.rfl")?;
let logic = CfmcLogic::parse("QD7B(02) AND AGEGROUP(1-3)", &layout)?;
```

The crate re-exports the most common types at the root: `RflFile`,
`RflQuestion`, `QuestionType`, `CfmcLogic`, `CfmcNode`, `CfmcOperator`,
`Banner`, `BannersTable`, `BannersTables`, `CrossTabsLogic`, and
`CrossTabsError`.

## Dependencies

- [`ipf_survey`](https://crates.io/crates/ipf_survey) — IPF raking core
- [`calamine`](https://crates.io/crates/calamine) — Excel workbook reader
- [`clap`](https://crates.io/crates/clap) — CLI argument parsing
- [`regex`](https://crates.io/crates/regex), [`ahash`](https://crates.io/crates/ahash),
  [`rayon`](https://crates.io/crates/rayon)

## License

Unlicensed / internal — add a license before publishing.
