# cli-truncate

[![Crates.io](https://img.shields.io/crates/v/cli-truncate.svg)](https://crates.io/crates/cli-truncate)
[![Documentation](https://docs.rs/cli-truncate/badge.svg)](https://docs.rs/cli-truncate)
[![CI](https://github.com/trananhtung/cli-truncate/actions/workflows/ci.yml/badge.svg)](https://github.com/trananhtung/cli-truncate/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/cli-truncate.svg)](#license)

**Truncate strings to a terminal display width** with an ellipsis — correctly
handling **wide (CJK) characters** (two columns) and **ANSI escape sequences**
(zero columns). A Rust take on Node's [`cli-truncate`](https://www.npmjs.com/package/cli-truncate).

```rust
use cli_truncate::{truncate, width, Options, Position};

assert_eq!(truncate("unicorn", 4), "uni…");
assert_eq!(truncate("古池や蛙", 6), "古池…");                  // wide chars counted as 2
assert_eq!(width("\u{1b}[31mhi\u{1b}[0m"), 2);                // ANSI = zero width

// Position + custom ellipsis
assert_eq!(Options::new().position(Position::Start).truncate("unicorn", 5), "…corn");
assert_eq!(Options::new().position(Position::Middle).truncate("unicorn", 5), "un…rn");
assert_eq!(Options::new().ellipsis("...").truncate("unicorn", 6), "uni...");
```

## Why cli-truncate?

`unicode-truncate` truncates by display width, but doesn't understand ANSI color
codes; the heavyweight `console` crate can, but pulls in a full terminal stack.
`cli-truncate` is the small, focused piece: truncate styled, wide-character text
to fit a column budget — for tables, status lines, log viewers, and TUIs.

```toml
[dependencies]
cli-truncate = "0.1"
```

## API

| Item | Purpose |
| --- | --- |
| `truncate(text, max_width)` | Truncate at the end with a `…` ellipsis |
| `Options::new().position(..).ellipsis(..).truncate(text, max_width)` | Configurable truncation |
| `width(text)` | Display width in columns (ANSI-ignored, wide chars = 2) |
| `Position` | `End` (default), `Start`, `Middle` |

## Behavior

- Output never exceeds `max_width` columns; `max_width == 0` yields `""`. An ellipsis
  wider than `max_width` is itself clamped, so the bound always holds.
- Escape sequences are recognized as zero width and never split across the cut:
  CSI (`ESC [ … m`, including the 8-bit `0x9B` form), string sequences (OSC
  hyperlinks, DCS/SOS/PM/APC with their `ST`/`BEL` terminators), and two-byte / nF
  escapes (`ESC c`, `ESC ( B`).
- A reset (`\x1b[0m`) is appended only when the kept text leaves a style open, so
  color can't "leak" past the cut and resets are never duplicated.
- Other C0/C1 control characters (newline, tab, carriage return, NUL, …) have no
  defined column width and are dropped from truncated output, so they can't break
  single-line layout.
- For `Start`/`Middle`, styling that began in the *dropped* region isn't carried
  over, and a combining mark whose base was dropped is not orphaned onto the ellipsis.
- Truncation respects character boundaries, not grapheme clusters: a multi-codepoint
  cluster (a ZWJ emoji or a flag) may be cut between codepoints — the column budget
  is always honored regardless.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
