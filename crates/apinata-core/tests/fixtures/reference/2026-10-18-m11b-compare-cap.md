# m11b compare: body size cap

Date: 2026-10-18. Spec: `docs/specs/2026-10-18-m11b-compare-design.md`.

## Method

The ignored test `compare::tests::measure_worst_cases` in `crates/apinata-core/src/compare/mod.rs`
runs `compare_with` with a 30 s timeout and no size cap, so each figure is the real cost of one
comparison, not the backstop. Run in release:

    cargo test -p apinata-core --release --lib compare::tests::measure_worst_cases -- --ignored --nocapture

Three worst cases per size, single run each, wall clock, this machine (2026-10-18):

- json: an array of small objects where every `v` differs (many JSON changes);
- text: 80-byte lines, every line different (the line diff's worst case);
- minified: one-line JSON of 0s with a single changed element.

## Results (milliseconds)

| Size    | json-many-small-changes | text-every-line-different | one-line-minified-json |
|---------|------------------------:|--------------------------:|-----------------------:|
| 256 KiB |                      19 |                        64 |                     10 |
| 512 KiB |                      53 |                       182 |                      9 |
| 1 MiB   |                      58 |                       629 |                     21 |
| 2 MiB   |                     134 |                      3507 |                     43 |
| 5 MiB   |                     364 |                     15825 |                    105 |

## Decision

The cap is the largest listed size where all three cases stay under 100 ms. That is 256 KiB;
at 512 KiB the all-lines-different text case already takes 182 ms. Text diffs also run under the
250 ms `COMPARE_TIMEOUT`, which bounds the cost of a body just under the cap on a slower machine.

COMPARE_BODY_LIMIT = 262144 bytes
