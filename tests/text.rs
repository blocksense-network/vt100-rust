mod helpers;

#[test]
fn ascii() {
    helpers::fixture("ascii");
}

#[test]
fn utf8() {
    helpers::fixture("utf8");
}

#[test]
fn newlines() {
    helpers::fixture("newlines");
}

#[test]
fn wide() {
    helpers::fixture("wide");
}

#[test]
fn combining() {
    helpers::fixture("combining");
}

#[test]
fn wrap() {
    helpers::fixture("wrap");
}

#[test]
fn wrap_weird() {
    helpers::fixture("wrap_weird");
}

/// The cells of `row`'s first `cols` columns, a wide character's second
/// column as `"<wide>"`, each with whether it is wide.
fn cells(parser: &vt100::Parser, row: u16, cols: u16) -> Vec<(String, bool)> {
    (0..cols)
        .map(|col| {
            let cell = parser.screen().cell(row, col).unwrap();
            if cell.is_wide_continuation() {
                ("<wide>".to_string(), false)
            } else {
                (cell.contents().to_string(), cell.is_wide())
            }
        })
        .collect()
}

fn narrow_cell(s: &str) -> (String, bool) {
    (s.to_string(), false)
}

fn wide_cell(s: &str) -> (String, bool) {
    (s.to_string(), true)
}

fn continuation() -> (String, bool) {
    ("<wide>".to_string(), false)
}

// VARIATION SELECTOR-16 (U+FE0F) asks for the emoji presentation of the
// character before it, and an emoji base with it is two columns wide, as
// unicode-width counts it (https://www.unicode.org/reports/tr51/#Presentation_Style)
// and as tmux 3.6a lays it out: the character becomes a wide cell, and the
// text after it starts two columns on.
#[test]
fn vs16_makes_an_emoji_base_wide() {
    // Over stale text: the second column is the wide continuation.
    let input = "\x1b[1;1HZZZZZZ\x1b[1;1H\u{26a0}\u{fe0f}X";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            wide_cell("\u{26a0}\u{fe0f}"),
            continuation(),
            narrow_cell("X"),
            narrow_cell("Z")
        ]
    );
    assert_eq!(parser.screen().cursor_position(), (0, 3));
    // Text taken from the screen has the character once, with no blank
    // after it.
    assert_eq!(parser.screen().contents(), "\u{26a0}\u{fe0f}XZZZ");
    helpers::assert_reproduces_state(input.as_bytes());

    // Cut between two writes, and inside its own UTF-8 bytes.
    let bytes = "\u{2714}\u{fe0f}X".as_bytes();
    let mut parser = vt100::Parser::default();
    parser.process(&bytes[..3]);
    parser.process(&bytes[3..5]);
    parser.process(&bytes[5..]);
    assert_eq!(
        cells(&parser, 0, 3),
        [
            wide_cell("\u{2714}\u{fe0f}"),
            continuation(),
            narrow_cell("X")
        ]
    );

    // Over the first half of a wide character: its second half is cleared,
    // as when a wide character is printed over it.
    let input = "a\u{30c7}b\x1b[1;1H\u{26a0}\u{fe0f}";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            wide_cell("\u{26a0}\u{fe0f}"),
            continuation(),
            narrow_cell(""),
            narrow_cell("b")
        ]
    );
    assert_eq!(parser.screen().cursor_position(), (0, 2));
    helpers::assert_reproduces_state(input.as_bytes());

    // In the second-to-last column the cursor waits after the second
    // column, and the next character wraps.
    let input = "\x1b[1;79H\u{26a0}\u{fe0f}X";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 80)[78..],
        [wide_cell("\u{26a0}\u{fe0f}"), continuation()]
    );
    assert_eq!(cells(&parser, 1, 1), [narrow_cell("X")]);
    assert!(parser.screen().row_wrapped(0));
    helpers::assert_reproduces_state(input.as_bytes());
}

// A character already two columns wide, or one with no emoji presentation
// (unicode-width counts `a` U+FE0F one column), keeps its width.
#[test]
fn vs16_keeps_other_widths() {
    let mut parser = vt100::Parser::default();
    parser.process("\u{1f600}\u{fe0f}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 3),
        [
            wide_cell("\u{1f600}\u{fe0f}"),
            continuation(),
            narrow_cell("X")
        ]
    );

    let mut parser = vt100::Parser::default();
    parser.process("a\u{fe0f}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 2),
        [narrow_cell("a\u{fe0f}"), narrow_cell("X")]
    );
    assert_eq!(parser.screen().cursor_position(), (0, 2));
}

// A character in the last column has no second column. tmux 3.6a keeps it
// one column wide there and ends the pending wrap: the cursor stays on it,
// and the next character is printed over it, not on the next row.
#[test]
fn vs16_in_the_last_column() {
    let input = "\x1b[1;80H\u{26a0}\u{fe0f}";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("\u{26a0}\u{fe0f}"));
    assert_eq!(parser.screen().cursor_position(), (0, 79));
    helpers::assert_reproduces_state(input.as_bytes());

    parser.process(b"X");
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("X"));
    assert_eq!(cells(&parser, 1, 1), [narrow_cell("")]);
    assert!(!parser.screen().row_wrapped(0));

    // Appended across a wrap, to the last column of the row above: there
    // is no pending wrap to end, and the character stays one column wide.
    let input = format!("{}\u{26a0}b\x08\u{fe0f}", "a".repeat(79));
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("\u{26a0}\u{fe0f}"));
    assert_eq!(cells(&parser, 1, 2), [narrow_cell("b"), narrow_cell("")]);
    assert_eq!(parser.screen().cursor_position(), (1, 0));
    // This screen is not checked against its formatted contents: printing
    // them prints the selector after the character in the last column,
    // which ends the pending wrap there, so the `b` after it would be
    // printed over the character instead of wrapping.
}
