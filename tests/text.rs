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

/// Whether `bytes` restore a saved cursor (DECRC, `ESC 8`). tmux 3.6a
/// restores a cursor saved past the end of a row into the row's last column,
/// so the next character is printed over that cell instead of wrapping.
fn restores_a_saved_cursor(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == b"\x1b8")
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

// An emoji ZWJ sequence, an emoji with a skin-tone modifier and a flag (a
// pair of regional indicators) are each one extended grapheme cluster
// (https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries, GB11,
// GB9, GB12/GB13) that unicode-width measures two columns wide, as tmux 3.6a
// lays each out and as ratatui draws it: one wide cell, and the text after
// it starts two columns on.
#[test]
fn an_emoji_cluster_takes_one_wide_cell() {
    for cluster in [
        // HEAVY BLACK HEART, VS16, ZWJ, FIRE
        "\u{2764}\u{fe0f}\u{200d}\u{1f525}",
        // WARNING SIGN, VS16, ZWJ, FIRE
        "\u{26a0}\u{fe0f}\u{200d}\u{1f525}",
        // MAN, ZWJ, WOMAN, ZWJ, GIRL
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        // WAVING WHITE FLAG, VS16, ZWJ, RAINBOW
        "\u{1f3f3}\u{fe0f}\u{200d}\u{1f308}",
        // THUMBS UP SIGN, EMOJI MODIFIER FITZPATRICK TYPE-4
        "\u{1f44d}\u{1f3fd}",
        // REGIONAL INDICATOR SYMBOL LETTERS B, G
        "\u{1f1e7}\u{1f1ec}",
    ] {
        // Over stale text: the second column is the wide continuation.
        let input = format!("\x1b[1;1HZZZZZZZZ\x1b[1;1H{cluster}X");
        let mut parser = vt100::Parser::default();
        parser.process(input.as_bytes());
        assert_eq!(
            cells(&parser, 0, 4),
            [
                wide_cell(cluster),
                continuation(),
                narrow_cell("X"),
                narrow_cell("Z")
            ],
            "{cluster:?}"
        );
        assert_eq!(parser.screen().cursor_position(), (0, 3), "{cluster:?}");
        assert_eq!(
            parser.screen().contents(),
            format!("{cluster}XZZZZZ"),
            "{cluster:?}"
        );
        helpers::assert_reproduces_state(input.as_bytes());

        // Cut between two writes after each code point, and inside the
        // UTF-8 bytes of each.
        let bytes = format!("{cluster}X").into_bytes();
        for cut in 1..bytes.len() {
            let mut parser = vt100::Parser::default();
            parser.process(&bytes[..cut]);
            parser.process(&bytes[cut..]);
            assert_eq!(
                cells(&parser, 0, 3),
                [wide_cell(cluster), continuation(), narrow_cell("X")],
                "{cluster:?} cut after {cut} bytes"
            );
        }
    }

    // A third regional indicator starts a new cluster: one column, as
    // unicode-width counts a lone one.
    let mut parser = vt100::Parser::default();
    parser.process("\u{1f1e7}\u{1f1ec}\u{1f1e7}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            wide_cell("\u{1f1e7}\u{1f1ec}"),
            continuation(),
            narrow_cell("\u{1f1e7}"),
            narrow_cell("X")
        ]
    );
}

// Characters that do not form an emoji cluster with the cell before them
// keep their own cells: a ZWJ between two characters that are not emoji
// (UAX #29 joins only Extended_Pictographic ones), and a cluster that
// unicode-width measures wider than two columns, which no cell can hold and
// which ratatui also lays out wider than two: HEAVY BLACK HEART, ZWJ, FIRE
// without VS16 (three columns) and `a` with a skin-tone modifier.
#[test]
fn characters_outside_an_emoji_cluster_keep_their_cells() {
    let mut parser = vt100::Parser::default();
    parser.process("a\u{200d}bX".as_bytes());
    assert_eq!(
        cells(&parser, 0, 3),
        [narrow_cell("a\u{200d}"), narrow_cell("b"), narrow_cell("X")]
    );

    let mut parser = vt100::Parser::default();
    parser.process("\u{30c7}\u{200d}\u{30c7}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 5),
        [
            wide_cell("\u{30c7}\u{200d}"),
            continuation(),
            wide_cell("\u{30c7}"),
            continuation(),
            narrow_cell("X")
        ]
    );

    let mut parser = vt100::Parser::default();
    parser.process("\u{2764}\u{200d}\u{1f525}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            narrow_cell("\u{2764}\u{200d}"),
            wide_cell("\u{1f525}"),
            continuation(),
            narrow_cell("X")
        ]
    );

    let mut parser = vt100::Parser::default();
    parser.process("a\u{1f3fd}X".as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            narrow_cell("a"),
            wide_cell("\u{1f3fd}"),
            continuation(),
            narrow_cell("X")
        ]
    );
}

// At the end of a row, as tmux 3.6a lays the clusters out on a 20-column
// screen.
#[test]
fn an_emoji_cluster_at_the_end_of_a_row() {
    // A ZWJ sequence that fits in the last two columns: the cursor waits
    // after it, and the next character wraps.
    let input = "\x1b[1;79H\u{2764}\u{fe0f}\u{200d}\u{1f525}X";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 80)[78..],
        [
            wide_cell("\u{2764}\u{fe0f}\u{200d}\u{1f525}"),
            continuation()
        ]
    );
    assert_eq!(cells(&parser, 1, 2), [narrow_cell("X"), narrow_cell("")]);
    assert!(parser.screen().row_wrapped(0));
    helpers::assert_reproduces_state(input.as_bytes());

    // A cluster that starts with a wide character in the last column wraps
    // whole onto the next row.
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    let input = format!("\x1b[1;80H{family}X");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 1, 3),
        [wide_cell(family), continuation(), narrow_cell("X")]
    );
    helpers::assert_reproduces_state(input.as_bytes());

    // A flag that starts in the last column has no second column there:
    // like a character with VS16, it stays one column wide, the cursor
    // stays on it instead of waiting to wrap, and the next character is
    // printed over it.
    let input = "\x1b[1;80H\u{1f1e7}\u{1f1ec}";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("\u{1f1e7}\u{1f1ec}"));
    assert_eq!(parser.screen().cursor_position(), (0, 79));
    helpers::assert_reproduces_state(input.as_bytes());
    parser.process(b"X");
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("X"));
    assert_eq!(cells(&parser, 1, 1), [narrow_cell("")]);
    assert!(!parser.screen().row_wrapped(0));
}

// A character that would join an emoji cluster joins the cell before the
// cursor on the cursor's row only. With the cursor moved back to the first
// column of a row the row above wrapped onto (here by a carriage return),
// the cluster in the last column of the row above is not the character the
// cursor follows: tmux 3.6a does not join across the row there, and prints
// the character in the first column, over what was there.
#[test]
fn an_emoji_cluster_does_not_join_the_row_above() {
    // A regional indicator in the last column, then one in the first
    // column of the next row.
    let input = format!("{}\u{1f1e7}X\r\u{1f1ec}Z", "a".repeat(79));
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(cells(&parser, 0, 80)[79], narrow_cell("\u{1f1e7}"));
    assert_eq!(
        cells(&parser, 1, 3),
        [narrow_cell("\u{1f1ec}"), narrow_cell("Z"), narrow_cell("")]
    );
    assert_eq!(parser.screen().cursor_position(), (1, 2));

    // An emoji with VS16 and a ZWJ in the last two columns, then FIRE in
    // the first column of the next row.
    let input =
        format!("{}\u{2764}\u{fe0f}\u{200d}X\r\u{1f525}Z", "a".repeat(78));
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 80)[78..],
        [wide_cell("\u{2764}\u{fe0f}\u{200d}"), continuation()]
    );
    assert_eq!(
        cells(&parser, 1, 3),
        [wide_cell("\u{1f525}"), continuation(), narrow_cell("Z")]
    );
    assert_eq!(parser.screen().cursor_position(), (1, 3));
}

// A screen one row high scrolls that row away when a character wraps past
// its last column, as a taller screen scrolls its top row away at the
// bottom: the row the character wrapped from leaves the screen, and the
// character is printed in the first column of the new row. tmux 3.6a does
// the same in a pane one row high. The row that left is kept in the
// scrollback, marked as wrapped, when the screen has one.
#[test]
fn a_one_row_screen_wraps() {
    let mut parser = vt100::Parser::new(1, 10, 0);
    parser.process(b"abcdefghijKL");
    assert_eq!(parser.screen().contents(), "KL");
    assert_eq!(parser.screen().cursor_position(), (0, 2));

    // a wide character that does not fit in the last column wraps too
    let mut parser = vt100::Parser::new(1, 10, 0);
    parser.process("abcdefghi\u{4e2d}".as_bytes());
    assert_eq!(parser.screen().contents(), "\u{4e2d}");
    assert_eq!(parser.screen().cursor_position(), (0, 2));

    let mut parser = vt100::Parser::new(1, 10, 10);
    parser.process(b"abcdefghijKLMNOPQRSTuv");
    assert_eq!(parser.screen().contents(), "uv");
    assert!(!parser.screen().row_wrapped(0));
    parser.screen_mut().set_scrollback(1);
    assert_eq!(parser.screen().contents(), "KLMNOPQRST");
    assert!(parser.screen().row_wrapped(0));
    parser.screen_mut().set_scrollback(2);
    assert_eq!(parser.screen().contents(), "abcdefghij");
    assert!(parser.screen().row_wrapped(0));

    // a wide character that does not fit after an empty last column moves
    // to the next row without wrapping the row it left (text() only wraps
    // after a last column with contents): the row in the scrollback is not
    // marked as wrapped
    let mut parser = vt100::Parser::new(1, 10, 10);
    parser.process("abcdefghi\u{4e2d}".as_bytes());
    assert_eq!(parser.screen().contents(), "\u{4e2d}");
    parser.screen_mut().set_scrollback(1);
    assert_eq!(parser.screen().contents(), "abcdefghi");
    assert!(!parser.screen().row_wrapped(0));
}

// THUMBS UP SIGN, then EMOJI MODIFIER FITZPATRICK TYPE-4 and REGIONAL
// INDICATOR SYMBOL LETTER B: each joins the cell before the cursor when it is
// printed right after it.
const THUMBS_UP: &str = "\u{1f44d}";
const SKIN_TONE: &str = "\u{1f3fd}";
const RI_B: &str = "\u{1f1e7}";

// Two cells that would join keep their own cells when the program writes
// them apart (the join depends only on where the cursor is, as in tmux), and
// the formatted contents (contents_formatted, rows_formatted, contents_diff
// and state_diff) rebuild that screen: replayed, the second cell is not
// printed right after the first. tmux 3.6a keeps each of these screens
// apart, in a detached pane written with `stty -opost`: the modifier written
// before its base gives `👍🏽Z` with Z in column 4, where written in order the
// two join and Z stays in column 4 after two blank columns.
#[test]
fn emoji_cells_written_apart_are_replayed_apart() {
    // The modifier first, then its base before it.
    let input = format!("\x1b[1;3H{SKIN_TONE}\x1b[1;1H{THUMBS_UP}\x1b[1;5HZ");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 5),
        [
            wide_cell(THUMBS_UP),
            continuation(),
            wide_cell(SKIN_TONE),
            continuation(),
            narrow_cell("Z")
        ]
    );
    helpers::assert_reproduces_state(input.as_bytes());

    // A cell ending in a ZWJ (HEAVY BLACK HEART, VS16, ZWJ), with FIRE
    // written after it beforehand.
    let input =
        "\x1b[1;3H\u{1f525}\x1b[1;1H\u{2764}\u{fe0f}\u{200d}\x1b[1;5HZ";
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 5),
        [
            wide_cell("\u{2764}\u{fe0f}\u{200d}"),
            continuation(),
            wide_cell("\u{1f525}"),
            continuation(),
            narrow_cell("Z")
        ]
    );
    helpers::assert_reproduces_state(input.as_bytes());

    // Three regional indicators written right to left: each would join the
    // one before it into a flag. tmux 3.6a gives `🇧🇬🇧Z`, Z in column 3.
    let input =
        format!("\x1b[1;3H{RI_B}\x1b[1;2H\u{1f1ec}\x1b[1;1H{RI_B}\x1b[1;4HZ");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 4),
        [
            narrow_cell(RI_B),
            narrow_cell("\u{1f1ec}"),
            narrow_cell(RI_B),
            narrow_cell("Z")
        ]
    );
    helpers::assert_reproduces_state(input.as_bytes());

    // The pair in the last two cells of a row that did not wrap.
    let input =
        format!("\x1b[1;79H{SKIN_TONE}\x1b[1;77H{THUMBS_UP}\x1b[2;1HZ");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(
        cells(&parser, 0, 80)[76..],
        [
            wide_cell(THUMBS_UP),
            continuation(),
            wide_cell(SKIN_TONE),
            continuation()
        ]
    );
    assert!(!parser.screen().row_wrapped(0));
    helpers::assert_reproduces_state(input.as_bytes());
}

// The same across the end of a wrapped row. A character printed in the
// first column of a row the row above wrapped onto, after the cursor was
// moved there, does not join the last cell of the row above, as in tmux
// 3.6a (80x24: `a`×78 and 👍 on the row, then `x` wraps, a carriage return
// and the modifier give 🏽 in column 0 of the next row and Z in column 2).
// A replay prints that character right after the row above, with the cursor
// waiting to wrap, so it must not be joined there either.
#[test]
fn an_emoji_cell_written_apart_across_a_wrapped_row_is_replayed_apart() {
    let input =
        format!("\x1b[2;1H{}{THUMBS_UP}x\r{SKIN_TONE}Z", "a".repeat(78));
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert!(parser.screen().row_wrapped(1));
    assert_eq!(
        cells(&parser, 1, 80)[78..],
        [wide_cell(THUMBS_UP), continuation()]
    );
    assert_eq!(
        cells(&parser, 2, 3),
        [wide_cell(SKIN_TONE), continuation(), narrow_cell("Z")]
    );
    helpers::assert_reproduces_state(input.as_bytes());
}

// A pair written apart in the last cells of a wrapped row: the replay prints
// the row's last cell last, so the cursor waits to wrap after it and the next
// row's first character wraps the row, as in the screen it came from. tmux
// 3.6a keeps the cells apart (80x24: 👍 in columns 76-77, 🏽 in 78-79, the
// row wrapped onto `x`); so it does for three regional indicators.
#[test]
fn emoji_cells_written_apart_at_the_end_of_a_wrapped_row_are_replayed_apart()
{
    let input =
        format!("\x1b[2;79H{SKIN_TONE}x\x1b[2;77H{THUMBS_UP}\x1b[2;1HZ");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert!(parser.screen().row_wrapped(1));
    assert_eq!(
        cells(&parser, 1, 80)[76..],
        [
            wide_cell(THUMBS_UP),
            continuation(),
            wide_cell(SKIN_TONE),
            continuation()
        ]
    );
    helpers::assert_reproduces_state(input.as_bytes());
    // the row's other cells are printed after the rows below, not between a
    // saved and a restored cursor
    assert!(!restores_a_saved_cursor(
        &parser.screen().contents_formatted()
    ));
    assert!(!restores_a_saved_cursor(
        &parser
            .screen()
            .contents_diff(vt100::Parser::default().screen())
    ));

    let input = format!(
        "\x1b[2;80H{RI_B}x\x1b[2;79H{RI_B}\x1b[2;78H{RI_B}\x1b[2;1HZ"
    );
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert!(parser.screen().row_wrapped(1));
    assert_eq!(
        cells(&parser, 1, 80)[77..],
        [narrow_cell(RI_B), narrow_cell(RI_B), narrow_cell(RI_B)]
    );
    helpers::assert_reproduces_state(input.as_bytes());
    assert!(!restores_a_saved_cursor(
        &parser.screen().contents_formatted()
    ));

    // The row wraps only in the new screen, with the pair apart and
    // unchanged in both: contents_diff draws the row's last cell again to
    // wrap it. (The new screen erases the base first, so that the modifier
    // written after it does not join it.)
    helpers::assert_reproduces_state_from(
        format!(
            "\x1b[2;77H\x1b[2X\x1b[2;79H{SKIN_TONE}x\x1b[2;77H{THUMBS_UP}\x1b[2;1HZ"
        )
        .as_bytes(),
        format!("\x1b[2;79H{SKIN_TONE}\x1b[2;77H{THUMBS_UP}").as_bytes(),
    );
}

// With the cursor waiting to wrap after a pair written apart in the last
// cells of its row (LF keeps it waiting, as in tmux 3.6a, whose cursor ends
// at column 80 of row 1 on this input), the replay leaves the cursor waiting
// there too, and the cells apart. It gets there as the program did, from the
// last cell of a row above and a line feed; with no such row above, by
// saving and restoring the cursor, which this parser restores past the end.
#[test]
fn the_cursor_waits_to_wrap_after_emoji_cells_written_apart() {
    let input =
        format!("\x1b[2;79H{SKIN_TONE}\x1b[2;77H{THUMBS_UP}\x1b[1;80HQ\n");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(parser.screen().cursor_position(), (1, 80));
    assert_eq!(
        cells(&parser, 1, 80)[76..],
        [
            wide_cell(THUMBS_UP),
            continuation(),
            wide_cell(SKIN_TONE),
            continuation()
        ]
    );
    helpers::assert_reproduces_state(input.as_bytes());
    assert!(!restores_a_saved_cursor(&parser.screen().state_formatted()));
    assert!(!restores_a_saved_cursor(
        &parser.screen().cursor_state_formatted()
    ));

    // The same on the bottom row, whose run is printed after the rows too.
    let input =
        format!("\x1b[24;79H{SKIN_TONE}\x1b[24;77H{THUMBS_UP}\x1b[23;80HQ\n");
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(parser.screen().cursor_position(), (23, 80));
    helpers::assert_reproduces_state(input.as_bytes());
    assert!(!restores_a_saved_cursor(&parser.screen().state_formatted()));
    assert!(!restores_a_saved_cursor(
        &parser
            .screen()
            .state_diff(vt100::Parser::default().screen())
    ));

    // The pair on the first row: VPA keeps the cursor waiting to wrap.
    let input = format!(
        "\x1b[1;79H{SKIN_TONE}\x1b[1;77H{THUMBS_UP}\x1b[3;80HQ\x1b[1d"
    );
    let mut parser = vt100::Parser::default();
    parser.process(input.as_bytes());
    assert_eq!(parser.screen().cursor_position(), (0, 80));
    helpers::assert_reproduces_state(input.as_bytes());
}

// contents_diff writes only the cells that changed. When the first cell of
// a pair written apart is unchanged, the second must still not be printed
// right after it.
#[test]
fn a_diff_keeps_emoji_cells_written_apart_apart() {
    // 👍 is unchanged; the modifier replaces X after it.
    helpers::assert_reproduces_state_from(
        format!("\x1b[1;1H  \x1b[1;3H{SKIN_TONE}\x1b[1;1H{THUMBS_UP}")
            .as_bytes(),
        format!("{THUMBS_UP}X").as_bytes(),
    );
    // The last cell of a wrapped row and the first cell of the row below
    // change: the diff prints the second right after the first, with the
    // cursor waiting to wrap (THUMBS DOWN SIGN replaces THUMBS UP SIGN).
    helpers::assert_reproduces_state_from(
        format!("\x1b[2;79H\u{1f44e}\x1b[3;1H{SKIN_TONE}").as_bytes(),
        format!("\x1b[2;1H{}{THUMBS_UP}x", "a".repeat(78)).as_bytes(),
    );
}
