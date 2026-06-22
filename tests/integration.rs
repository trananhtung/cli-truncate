//! End-to-end behavioral spec for the public `cli-truncate` API.

use cli_truncate::{truncate, width, Options, Position};

// ---------------------------------------------------------------------------
// width()
// ---------------------------------------------------------------------------

#[test]
fn width_counts_display_columns() {
    assert_eq!(width("unicorn"), 7);
    assert_eq!(width(""), 0);
    assert_eq!(width("古"), 2); // wide (CJK) char = 2 columns
    assert_eq!(width("古池"), 4);
    assert_eq!(width("\u{1b}[31mhi\u{1b}[0m"), 2); // ANSI escapes are zero-width
}

// ---------------------------------------------------------------------------
// truncate() — End (default)
// ---------------------------------------------------------------------------

#[test]
fn truncate_end_default() {
    assert_eq!(truncate("unicorn", 4), "uni\u{2026}"); // "uni" + "…"
    assert_eq!(truncate("unicorn", 1), "\u{2026}"); // only room for the ellipsis
}

#[test]
fn no_truncation_when_it_fits() {
    assert_eq!(truncate("unicorn", 7), "unicorn");
    assert_eq!(truncate("unicorn", 100), "unicorn");
    assert_eq!(truncate("", 5), "");
}

#[test]
fn truncate_zero_width_is_empty() {
    assert_eq!(truncate("unicorn", 0), "");
}

#[test]
fn truncate_respects_wide_chars() {
    // avail = 6 - 1 = 5; 古(2)+池(2)=4 fits, や(2) would make 6 > 5 → stop
    assert_eq!(truncate("古池や蛙", 6), "古池\u{2026}");
    assert_eq!(width(&truncate("古池や蛙", 6)), 5);
}

#[test]
fn truncate_preserves_ansi_and_resets() {
    // red "unicorn"; truncated to 4 keeps the color code, adds ellipsis + reset
    let got = truncate("\u{1b}[31municorn\u{1b}[39m", 4);
    assert_eq!(got, "\u{1b}[31muni\u{2026}\u{1b}[0m");
}

// ---------------------------------------------------------------------------
// Options: ellipsis + position
// ---------------------------------------------------------------------------

#[test]
fn custom_ellipsis() {
    assert_eq!(
        Options::new().ellipsis("...").truncate("unicorn", 6),
        "uni..."
    );
}

#[test]
fn position_start_keeps_suffix() {
    assert_eq!(
        Options::new()
            .position(Position::Start)
            .truncate("unicorn", 5),
        "\u{2026}corn"
    );
}

#[test]
fn position_middle_keeps_both_ends() {
    assert_eq!(
        Options::new()
            .position(Position::Middle)
            .truncate("unicorn", 5),
        "un\u{2026}rn"
    );
}

#[test]
fn output_never_exceeds_max_width() {
    for w in 0..=10 {
        let out = truncate("a long-ish unicorn string", w);
        assert!(
            width(&out) <= w,
            "width {} for max {w}: {out:?}",
            width(&out)
        );
    }
}

// ---------------------------------------------------------------------------
// Regression: robust escape handling (OSC, two-byte escapes, 8-bit C1, double ESC)
// ---------------------------------------------------------------------------

#[test]
fn osc_hyperlinks_are_zero_width_and_never_split() {
    // ESC ]8;;URL BEL  label  ESC ]8;; BEL  — only "label" is visible (5 cols).
    let link = "\u{1b}]8;;https://example.com\u{7}label\u{1b}]8;;\u{7}";
    assert_eq!(width(link), 5);
    let out = truncate(link, 3);
    assert!(width(&out) <= 3, "width {} for {out:?}", width(&out));
    // The opening hyperlink escape (incl. its URL) is preserved intact, not sliced.
    assert!(
        out.starts_with("\u{1b}]8;;https://example.com\u{7}"),
        "{out:?}"
    );
}

#[test]
fn two_byte_and_lone_escapes_are_not_split_or_leaked() {
    // ESC D (IND) is a complete two-byte escape; it styles the dropped "EF" only.
    let out = truncate("ABC\u{1b}DEF", 4);
    assert_eq!(out, "ABC\u{2026}");
    assert!(!out.contains('\u{1b}'), "dangling escape leaked: {out:?}");
    // A lone trailing ESC is zero-width, not a visible glyph.
    assert_eq!(width("AB\u{1b}"), 2);
    // An incomplete CSI (no final byte) is still zero-width.
    assert_eq!(width("AB\u{1b}["), 2);
}

#[test]
fn double_csi_introducer_is_parsed_as_two_sequences() {
    // ESC[ ESC[31m x  → empty CSI + red SGR + "x"; only "x" is visible.
    assert_eq!(width("\u{1b}[\u{1b}[31mx"), 1);
}

#[test]
fn eight_bit_c1_csi_is_recognized() {
    // 0x9B is the 8-bit CSI introducer; "31m" is its body, not visible text.
    assert_eq!(width("\u{9b}31mHELLO"), 5);
    let out = truncate("\u{9b}31mHELLO", 4);
    assert!(width(&out) <= 4, "width {} for {out:?}", width(&out));
    assert!(out.starts_with("\u{9b}31m"), "{out:?}");
}

// ---------------------------------------------------------------------------
// Regression: control characters never leak into single-line output
// ---------------------------------------------------------------------------

#[test]
fn control_chars_are_dropped_and_count_as_zero_width() {
    assert_eq!(width("a\tb\nc"), 3);
    assert_eq!(truncate("ab\ncdef", 4), "abc\u{2026}");
    assert_eq!(truncate("a\tbcdef", 4), "abc\u{2026}");
    let out = truncate("x\ny\rz\tw extra text here", 5);
    assert!(
        !out.chars().any(|c| c.is_control() && c != '\u{1b}'),
        "control char leaked: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression: width invariant holds even for an oversized ellipsis
// ---------------------------------------------------------------------------

#[test]
fn oversized_ellipsis_is_clamped_to_max_width() {
    for pos in [Position::End, Position::Start, Position::Middle] {
        for w in 1..=4 {
            let out = Options::new()
                .ellipsis("...")
                .position(pos)
                .truncate("hello world foo", w);
            assert!(
                width(&out) <= w,
                "pos {pos:?} w {w}: {out:?} has width {}",
                width(&out)
            );
        }
    }
    assert_eq!(Options::new().ellipsis("...").truncate("unicorn", 2), "..");
    let out = Options::new().ellipsis("\u{53e4}").truncate("unicorn", 1); // 古 = 2 cols
    assert!(width(&out) <= 1, "{out:?}");
}

// ---------------------------------------------------------------------------
// Regression: SGR reset emitted exactly once, only when a style is open
// ---------------------------------------------------------------------------

#[test]
fn reset_emitted_once_only_when_style_is_open() {
    // Kept text leaves the default style → no synthetic reset.
    assert_eq!(truncate("\u{1b}[0mABCDEFG", 4), "\u{1b}[0mABC\u{2026}");
    // Styled kept text → exactly one reset.
    let out = truncate("\u{1b}[31mABCDEFG", 4);
    assert_eq!(out.matches("\u{1b}[0m").count(), 1, "{out:?}");
    // Start: input already ends in a reset → no doubled reset.
    let out = Options::new()
        .position(Position::Start)
        .truncate("\u{1b}[31mABCDEF\u{1b}[0m", 4);
    assert!(
        out.matches("\u{1b}[0m").count() <= 1,
        "double reset: {out:?}"
    );
}

#[test]
fn end_truncation_excludes_styling_of_dropped_text() {
    // The red SGR styles only the dropped "CD"; it must not bleed onto the ellipsis.
    assert_eq!(truncate("AB\u{1b}[31mCD\u{1b}[0m", 3), "AB\u{2026}");
}

// ---------------------------------------------------------------------------
// Regression: Start/Middle don't orphan a combining mark onto the ellipsis
// ---------------------------------------------------------------------------

#[test]
fn start_truncation_drops_orphan_combining_mark() {
    // 'Y' (whose acute accent follows) is dropped → the bare accent must not survive.
    let out = Options::new()
        .position(Position::Start)
        .truncate("XY\u{0301}Z", 2);
    assert_eq!(out, "\u{2026}Z");
    assert!(!out.chars().any(|c| c == '\u{0301}'), "{out:?}");
}
