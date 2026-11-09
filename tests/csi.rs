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
