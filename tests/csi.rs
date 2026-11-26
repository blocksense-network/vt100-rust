mod helpers;

#[test]
fn absolute_movement() {
    helpers::fixture("absolute_movement");
}

#[test]
fn row_clamp() {
    let mut vt = vt100::Parser::default();
    assert_eq!(vt.screen().cursor_position(), (0, 0));
    vt.process(b"\x1b[15d");
    assert_eq!(vt.screen().cursor_position(), (14, 0));
    vt.process(b"\x1b[150d");
    assert_eq!(vt.screen().cursor_position(), (23, 0));
}

#[test]
fn relative_movement() {
    helpers::fixture("relative_movement");
}

#[test]
fn ed() {
    helpers::fixture("ed");
}

#[test]
fn el() {
    helpers::fixture("el");
}

#[test]
fn ich_dch_ech() {
    helpers::fixture("ich_dch_ech");
}

#[test]
fn il_dl() {
    helpers::fixture("il_dl");
}

#[test]
fn scroll() {
    helpers::fixture("scroll");
}

#[test]
fn xtwinops() {
    struct Callbacks;
    impl vt100::Callbacks for Callbacks {
        fn resize(
            &mut self,
            screen: &mut vt100::Screen,
            (rows, cols): (u16, u16),
        ) {
            screen.set_size(rows, cols);
        }
    }

    let mut vt = vt100::Parser::new_with_callbacks(24, 80, 0, Callbacks);
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process(b"\x1b[8;24;80t");
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process(b"\x1b[8t");
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process(b"\x1b[8;80;24t");
    assert_eq!(vt.screen().size(), (80, 24));
    vt.process(b"\x1b[8;24t");
    assert_eq!(vt.screen().size(), (24, 24));

    let mut vt = vt100::Parser::new_with_callbacks(24, 80, 0, Callbacks);
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(
        vt.screen().rows(0, 80).next().unwrap(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(vt.screen().rows(0, 80).nth(1).unwrap(), "aaaaaaaaaa");
    vt.process(
        b"\x1b[H\x1b[8;24;15tbbbbbbbbbbbbbbbbbbbb\x1b[8;24;80tcccccccccccccccccccc",
    );
    assert_eq!(vt.screen().rows(0, 80).next().unwrap(), "bbbbbbbbbbbbbbb");
    assert_eq!(
        vt.screen().rows(0, 80).nth(1).unwrap(),
        "bbbbbcccccccccccccccccccc"
    );
}

/// The visible text of `row`, without trailing blanks.
fn row_text(parser: &vt100::Parser, row: u16) -> String {
    let (_, cols) = parser.screen().size();
    parser.screen().rows(0, cols).nth(usize::from(row)).unwrap()
}

// A character in the last column leaves the cursor past it, waiting to wrap
// (cursor_position() reports the column count). xterm and tmux end that wait
// on a cursor up or down, and leave the cursor in the last column of the row
// moved to, so the next character is printed there and does not wrap onto
// the row below (https://invisible-island.net/xterm/ctlseqs/ctlseqs.html,
// CUU and CUD; tmux 3.6a gives the same screens with these bytes).
#[test]
fn vertical_move_ends_pending_wrap() {
    let full = "a".repeat(80);
    for (name, mv, row) in [
        ("down", "\x1b[1B", 3),
        ("down, default count", "\x1b[B", 3),
        ("down by two", "\x1b[2B", 4),
        ("up", "\x1b[1A", 1),
        ("up, default count", "\x1b[A", 1),
        ("down past the bottom", "\x1b[99B", 23),
        ("up past the top", "\x1b[99A", 0),
    ] {
        let input = format!("\x1b[3;1H{full}{mv}");
        let mut parser = vt100::Parser::default();
        parser.process(input.as_bytes());
        assert_eq!(
            parser.screen().cursor_position(),
            (row, 79),
            "{name}: the cursor is in the last column of the row moved to"
        );
        helpers::assert_reproduces_state(input.as_bytes());

        let input = format!("{input}Y");
        let mut parser = vt100::Parser::default();
        parser.process(input.as_bytes());
        let expected = if row == 2 {
            format!("{}Y", "a".repeat(79))
        } else {
            format!("{}Y", " ".repeat(79))
        };
        assert_eq!(
            row_text(&parser, row),
            expected,
            "{name}: Y is in the last column of the row moved to"
        );
        if row < 23 {
            assert_eq!(
                row_text(&parser, row + 1),
                if row + 1 == 2 {
                    full.clone()
                } else {
                    String::new()
                },
                "{name}: nothing wraps onto the row below"
            );
        }
        assert_eq!(parser.screen().cursor_position(), (row, 80));
        helpers::assert_reproduces_state(input.as_bytes());
    }

    // A move that does not follow a last-column character keeps the column.
    let mut parser = vt100::Parser::default();
    parser.process(b"\x1b[3;1Habc\x1b[1BY");
    assert_eq!(row_text(&parser, 3), "   Y");
    assert_eq!(parser.screen().cursor_position(), (3, 4));
}

// xterm defines VPR (CSI Ps e) and HPR (CSI Ps a) as relative moves of the
// row and the column (https://invisible-island.net/xterm/ctlseqs/ctlseqs.html),
// but tmux 3.6a ignores both: with these bytes on an 8x20 screen it prints the
// next character where the cursor was, after a last-column character too,
// and never scrolls. The parser ignores them as tmux does (they reach
// Callbacks::unhandled_csi), and these cases pin tmux's screens.
#[test]
fn vpr_hpr_are_ignored_as_in_tmux() {
    let full = "A".repeat(20);
    for (name, input, row, text, cursor) in [
        (
            "VPR, middle",
            "\x1b[3;5Hab\x1b[2eX".to_string(),
            2,
            "    abX",
            (2, 7),
        ),
        (
            "HPR, middle",
            "\x1b[3;5Hab\x1b[3aX".to_string(),
            2,
            "    abX",
            (2, 7),
        ),
        (
            "VPR past the bottom",
            "\x1b[3;5Hab\x1b[30eX".to_string(),
            2,
            "    abX",
            (2, 7),
        ),
        (
            "HPR past the last column",
            "\x1b[3;15Hab\x1b[30aX".to_string(),
            2,
            "              abX",
            (2, 17),
        ),
        (
            "VPR after a last-column character",
            format!("\x1b[3;1H{full}\x1b[2eX"),
            3,
            "X",
            (3, 1),
        ),
        (
            "HPR after a last-column character",
            format!("\x1b[3;1H{full}\x1b[3aX"),
            3,
            "X",
            (3, 1),
        ),
        (
            "VPR on the bottom row",
            "\x1b[1;1Htop\x1b[8;3Hbot\x1b[eX".to_string(),
            7,
            "  botX",
            (7, 6),
        ),
        (
            "VPR on the bottom margin of a scroll region",
            "\x1b[1;1Hr1\x1b[2;1Hr2\x1b[3;1Hr3\x1b[4;1Hr4\x1b[2;4r\
             \x1b[4;3Hbot\x1b[eX"
                .to_string(),
            3,
            "r4botX",
            (3, 6),
        ),
    ] {
        let mut parser = vt100::Parser::new(8, 20, 0);
        parser.process(input.as_bytes());
        assert_eq!(row_text(&parser, row), text, "{name}: the row");
        assert_eq!(
            parser.screen().cursor_position(),
            cursor,
            "{name}: the cursor"
        );
        if name.starts_with("VPR on the bottom row") {
            assert_eq!(row_text(&parser, 0), "top", "{name}: no scroll");
        }
        if name.contains("scroll region") {
            assert_eq!(row_text(&parser, 1), "r2", "{name}: no scroll");
        }
        if name.contains("last-column") {
            assert_eq!(row_text(&parser, 2), full, "{name}: the full row");
        }
    }
}

// CBT (CSI Ps Z, ECMA-48 8.3.7) moves the cursor back Ps 8-column tab stops
// on the same row, clamping at column 0. After a character in the last column
// (pending wrap), tmux 3.6a ends the pending wrap first — placing the cursor
// on the last column (cols - 1) — and then steps back from there.
#[test]
fn cbt_moves_back_by_tab_stops_and_ends_pending_wrap() {
    for (name, cols, input, expected_row, expected_cursor) in [
        (
            "default count from col 10",
            20,
            "\x1b[1;11H\x1b[ZX",
            "        X",
            (0, 9),
        ),
        (
            "zero count defaults to 1",
            20,
            "\x1b[1;11H\x1b[0ZX",
            "        X",
            (0, 9),
        ),
        (
            "from tab stop 8 goes to 0",
            20,
            "\x1b[1;9H\x1b[ZX",
            "X",
            (0, 1),
        ),
        ("from col 7 goes to 0", 20, "\x1b[1;8H\x1b[ZX", "X", (0, 1)),
        ("from col 0 stays at 0", 20, "\x1b[1;1H\x1b[ZX", "X", (0, 1)),
        (
            "multiple stops from col 17",
            20,
            "\x1b[1;18H\x1b[2ZX",
            "        X",
            (0, 9),
        ),
        (
            "clamps at col 0 for huge count",
            20,
            "\x1b[1;18H\x1b[999ZX",
            "X",
            (0, 1),
        ),
        (
            "pending wrap at cols=20 ends at 19 then moves to 16",
            20,
            "aaaaaaaaaaaaaaaaaaaa\x1b[ZX",
            "aaaaaaaaaaaaaaaaXaaa",
            (0, 17),
        ),
        (
            "pending wrap at cols=24 ends at 23 then moves to 16",
            24,
            "aaaaaaaaaaaaaaaaaaaaaaaa\x1b[ZX",
            "aaaaaaaaaaaaaaaaXaaaaaaa",
            (0, 17),
        ),
        (
            "pending wrap at cols=25 ends at 24 then moves to 16",
            25,
            "aaaaaaaaaaaaaaaaaaaaaaaaa\x1b[ZX",
            "aaaaaaaaaaaaaaaaXaaaaaaaa",
            (0, 17),
        ),
    ] {
        let mut parser = vt100::Parser::new(4, cols, 0);
        parser.process(input.as_bytes());
        assert_eq!(row_text(&parser, 0), expected_row, "{name}: row 0");
        assert_eq!(
            parser.screen().cursor_position(),
            expected_cursor,
            "{name}: cursor"
        );
    }

    helpers::assert_reproduces_state(b"\x1b[1;20Hhello\x1b[2Zworld");
}

// HPA (CSI Ps `, ECMA-48 8.3.57) sets the 1-based column on the current row,
// matching CHA (CSI Ps G) and tmux 3.6a.
#[test]
fn hpa_moves_to_absolute_column_and_ends_pending_wrap() {
    for (name, input, expected_row, expected_cursor) in [
        ("explicit column", "abcdef\x1b[4`X", "abcXef", (0, 4)),
        ("default column is 1", "abcde\x1b[`X", "Xbcde", (0, 1)),
        ("zero defaults to 1", "abcde\x1b[0`X", "Xbcde", (0, 1)),
        (
            "clamps to last column",
            "abcde\x1b[999`X",
            "abcde              X",
            (0, 20),
        ),
        (
            "ends pending wrap on last column",
            "aaaaaaaaaaaaaaaaaaaa\x1b[20`X",
            "aaaaaaaaaaaaaaaaaaaX",
            (0, 20),
        ),
    ] {
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(input.as_bytes());
        assert_eq!(row_text(&parser, 0), expected_row, "{name}: row 0");
        assert_eq!(
            parser.screen().cursor_position(),
            expected_cursor,
            "{name}: cursor"
        );
    }

    helpers::assert_reproduces_state(b"\x1b[2;1Hhello\x1b[3`world");
}

// REP (CSI Ps b, ECMA-48 8.3.103) repeats the immediately preceding printed
// graphic character Ps times (default 1), capped by remaining columns on the
// row (cols - pos.col) as tmux 3.6a does, and clears the remembered character
// after repeating or on any intervening non-print control/escape/CSI/OSC.
#[test]
fn rep_repeats_preceding_graphic_char_as_in_tmux() {
    for (name, input, expected_rows, expected_cursor) in [
        ("basic repeat", "a\x1b[4bX", vec!["aaaaaX"], (0, 6)),
        ("default count is 1", "a\x1b[bX", vec!["aaX"], (0, 3)),
        ("zero defaults to 1", "a\x1b[0bX", vec!["aaX"], (0, 3)),
        (
            "no preceding char is a no-op",
            "\x1b[4bX",
            vec!["X"],
            (0, 1),
        ),
        (
            "consecutive REP clears last char",
            "a\x1b[2b\x1b[2bX",
            vec!["aaaX"],
            (0, 4),
        ),
        ("cleared by SGR", "a\x1b[31m\x1b[3bX", vec!["aX"], (0, 2)),
        (
            "cleared by cursor motion CSI",
            "a\x1b[2C\x1b[3bX",
            vec!["a  X"],
            (0, 4),
        ),
        (
            "cleared by DECSET",
            "a\x1b[?2026h\x1b[2bX",
            vec!["aX"],
            (0, 2),
        ),
        (
            "cleared by C0 controls",
            "a\r\n\x1b[3bX",
            vec!["a", "X"],
            (1, 1),
        ),
        ("cleared by BEL", "a\x07\x1b[2bX", vec!["aX"], (0, 2)),
        ("cleared by ESC", "a\x1b7\x1b[2bX", vec!["aX"], (0, 2)),
        (
            "cleared by OSC",
            "a\x1b]8;;https://example.com\x07\x1b[2bX",
            vec!["aX"],
            (0, 2),
        ),
        (
            "cleared by DCS",
            "a\x1bPfoo\x1b\\\x1b[2bX",
            vec!["aX"],
            (0, 2),
        ),
        (
            "clamped to remaining columns on row",
            "a\x1b[9999bX",
            vec!["aaaaaaaaaaaaaaaaaaaa", "X"],
            (1, 1),
        ),
        (
            "clamped near right margin",
            "\x1b[1;18Ha\x1b[5bX",
            vec!["                 aaa", "X"],
            (1, 1),
        ),
        (
            "zero repeats when already pending wrap",
            "aaaaaaaaaaaaaaaaaaaa\x1b[3bX",
            vec!["aaaaaaaaaaaaaaaaaaaa", "X"],
            (1, 1),
        ),
        (
            "repeats wide character",
            "a\u{4e2d}\x1b[2bX",
            vec!["a\u{4e2d}\u{4e2d}\u{4e2d}X"],
            (0, 8),
        ),
        (
            "wide char count clamped by cols - pos.col as in tmux",
            "\x1b[1;18H\u{4e2d}\x1b[10bX",
            vec!["                 \u{4e2d}", "\u{4e2d}X"],
            (1, 3),
        ),
        (
            "wide char from col 15 clamped to 4 repeats as in tmux",
            "\x1b[1;15H\u{4e2d}\x1b[10bX",
            vec![
                "              \u{4e2d}\u{4e2d}\u{4e2d}",
                "\u{4e2d}\u{4e2d}X",
            ],
            (1, 5),
        ),
        (
            "wide char pending wrap gives 0 repeats",
            "\x1b[1;19H\u{4e2d}\x1b[10bX",
            vec!["                  \u{4e2d}", "X"],
            (1, 1),
        ),
        (
            "repeats combining character",
            "e\u{0301}\x1b[2bX",
            vec!["e\u{0301}\u{0301}\u{0301}X"],
            (0, 2),
        ),
    ] {
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(input.as_bytes());
        let got_rows: Vec<String> = parser
            .screen()
            .rows(0, 20)
            .take(expected_rows.len())
            .collect();
        assert_eq!(got_rows, expected_rows, "{name}: rows");
        assert_eq!(
            parser.screen().cursor_position(),
            expected_cursor,
            "{name}: cursor"
        );
    }

    helpers::assert_reproduces_state(b" \x1b[6bhello -\x1b[4b world");
}

// tmux 3.6a ignores CHT (CSI Ps I) and leaves the cursor and pending wrap
// unchanged; pin that behavior alongside VPR/HPR.
#[test]
fn cht_is_ignored_as_in_tmux() {
    let mut parser = vt100::Parser::new(4, 20, 0);
    parser.process(b"abc\x1b[2IX");
    assert_eq!(row_text(&parser, 0), "abcX");
    assert_eq!(parser.screen().cursor_position(), (0, 4));
}
