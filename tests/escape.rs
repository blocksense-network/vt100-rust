mod helpers;

#[test]
fn deckpam() {
    helpers::fixture("deckpam");
}

#[test]
fn ri() {
    helpers::fixture("ri");
}

#[test]
fn ris() {
    helpers::fixture("ris");
}

#[test]
fn vb() {
    struct State {
        vb: usize,
    }

    impl vt100::Callbacks for State {
        fn visual_bell(&mut self, _: &mut vt100::Screen) {
            self.vb += 1;
        }
    }

    let mut parser =
        vt100::Parser::new_with_callbacks(24, 80, 0, State { vb: 0 });
    assert_eq!(parser.callbacks().vb, 0);

    let screen = parser.screen().clone();
    parser.process(b"\x1bg");
    assert_eq!(parser.callbacks().vb, 1);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process(b"\x1bg");
    assert_eq!(parser.callbacks().vb, 2);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process(b"\x1bg\x1bg\x1bg");
    assert_eq!(parser.callbacks().vb, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process(b"foo");
    assert_eq!(parser.callbacks().vb, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"foo");

    let screen = parser.screen().clone();
    parser.process(b"ba\x1bgr");
    assert_eq!(parser.callbacks().vb, 6);
    assert_eq!(parser.screen().contents_diff(&screen), b"bar");
}

#[test]
fn decsc() {
    helpers::fixture("decsc");
}

#[test]
fn decsc_resize() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(b"foo\x1b[20;70Hbar\x1b7");
    assert_eq!(parser.screen().contents(), "foo\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n                                                                     bar");
    assert_eq!(parser.screen().cursor_position(), (19, 72));
    parser.process(b"\x1b[H");
    assert_eq!(parser.screen().cursor_position(), (0, 0));
    parser.screen_mut().set_size(15, 60);
    assert_eq!(parser.screen().contents(), "foo");
    assert_eq!(parser.screen().cursor_position(), (0, 0));
    parser.process(b"y\x1b8z");
    assert_eq!(parser.screen().contents(), "yoo\n\n\n\n\n\n\n\n\n\n\n\n\n\n                                                           z");
    assert_eq!(parser.screen().cursor_position(), (14, 60));
}

/// The visible text of every row, without trailing blanks.
fn rows_text(parser: &vt100::Parser) -> Vec<String> {
    let (_, cols) = parser.screen().size();
    parser.screen().rows(0, cols).collect()
}

/// Rows r1 to r6 on rows 1 to 6 of an 8x20 screen, and a scroll region
/// from row 2 to row 4 (`CSI 2 ; 4 r`).
const REGION: &str =
    "\x1b[1;1Hr1\x1b[2;1Hr2\x1b[3;1Hr3\x1b[4;1Hr4\x1b[5;1Hr5\
                      \x1b[6;1Hr6\x1b[2;4r";

// IND (ESC D) moves the cursor down one row, keeping its column, and scrolls
// the scroll region up when the cursor is on its bottom margin; NEL (ESC E)
// is a carriage return and an IND
// (https://invisible-island.net/xterm/ctlseqs/ctlseqs.html, IND and NEL).
// After a character in the last column, IND, like LF, keeps the cursor
// waiting to wrap, so the next character wraps one row further; NEL's
// carriage return ends the wait. Each case gives the screen and cursor
// tmux 3.6a gives for the same bytes on an 8x20 screen.
#[test]
fn ind_nel() {
    let full = "A".repeat(20);
    let blank = String::new;
    for (name, input, rows, cursor) in [
        (
            "IND, middle",
            "\x1b[3;5Hab\x1bDX".to_string(),
            vec![blank(), blank(), "    ab".into(), "      X".into()],
            (3, 7),
        ),
        (
            "IND, after a last-column character",
            format!("\x1b[3;1H{full}\x1bDX"),
            vec![blank(), blank(), full.clone(), blank(), "X".into()],
            (4, 1),
        ),
        (
            "IND, bottom row",
            "\x1b[1;1Htop\x1b[8;3Hbot\x1bDX".to_string(),
            vec![
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                "  bot".into(),
                "     X".into(),
            ],
            (7, 6),
        ),
        (
            "IND, bottom row, after a last-column character",
            format!("\x1b[1;1Htop\x1b[8;1H{full}\x1bDX"),
            vec![
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                full.clone(),
                blank(),
                "X".into(),
            ],
            (7, 1),
        ),
        (
            "IND, bottom margin of a scroll region",
            format!("{REGION}\x1b[4;3Hbot\x1bDX"),
            vec![
                "r1".into(),
                "r3".into(),
                "r4bot".into(),
                "     X".into(),
                "r5".into(),
                "r6".into(),
            ],
            (3, 6),
        ),
        (
            "IND, bottom margin of a scroll region, after a last-column \
             character",
            format!("{REGION}\x1b[4;1H{full}\x1bDX"),
            vec![
                "r1".into(),
                full.clone(),
                blank(),
                "X".into(),
                "r5".into(),
                "r6".into(),
            ],
            (3, 1),
        ),
        (
            "IND, bottom row below a scroll region",
            "\x1b[1;1Hr1\x1b[2;4r\x1b[8;3Hbot\x1bDX".to_string(),
            vec![
                "r1".into(),
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                "  botX".into(),
            ],
            (7, 6),
        ),
        (
            "NEL, middle",
            "\x1b[3;5Hab\x1bEX".to_string(),
            vec![blank(), blank(), "    ab".into(), "X".into()],
            (3, 1),
        ),
        (
            "NEL, after a last-column character",
            format!("\x1b[3;1H{full}\x1bEX"),
            vec![blank(), blank(), full.clone(), "X".into()],
            (3, 1),
        ),
        (
            "NEL, bottom row",
            "\x1b[1;1Htop\x1b[8;3Hbot\x1bEX".to_string(),
            vec![
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                "  bot".into(),
                "X".into(),
            ],
            (7, 1),
        ),
        (
            "NEL, bottom row, after a last-column character",
            format!("\x1b[1;1Htop\x1b[8;1H{full}\x1bEX"),
            vec![
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                blank(),
                full.clone(),
                "X".into(),
            ],
            (7, 1),
        ),
        (
            "NEL, bottom margin of a scroll region",
            format!("{REGION}\x1b[4;3Hbot\x1bEX"),
            vec![
                "r1".into(),
                "r3".into(),
                "r4bot".into(),
                "X".into(),
                "r5".into(),
                "r6".into(),
            ],
            (3, 1),
        ),
    ] {
        let mut parser = vt100::Parser::new(8, 20, 0);
        parser.process(input.as_bytes());
        let mut expected = rows;
        expected.resize(8, String::new());
        assert_eq!(rows_text(&parser), expected, "{name}: the screen");
        assert_eq!(
            parser.screen().cursor_position(),
            cursor,
            "{name}: the cursor"
        );
    }

    // The cursor still waiting to wrap after IND, and the character that
    // wraps after it, are reproduced by the formatted output.
    let full = "a".repeat(80);
    helpers::assert_reproduces_state(
        format!("\x1b[3;1H{full}\x1bD").as_bytes(),
    );
    helpers::assert_reproduces_state(
        format!("\x1b[3;1H{full}\x1bDY").as_bytes(),
    );
    helpers::assert_reproduces_state(
        format!("\x1b[3;1H{full}\x1bEY").as_bytes(),
    );
}
