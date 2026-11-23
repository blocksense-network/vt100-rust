//! How an OSC longer than [`vt100::OSC_PAYLOAD_LIMIT`] is handled, and that
//! the limit changes nothing for OSC sequences within it.
//!
//! The limit follows `vte`'s own bounded (`no_std`) parser: payload bytes
//! past the limit are dropped, `;` past it opens no parameter, and the
//! sequence still ends where the terminal stream ends it and is dispatched
//! with what was kept. The parser's state outside the OSC is never affected.

use vt100::OSC_PAYLOAD_LIMIT as LIMIT;

#[derive(Default)]
struct Osc {
    title: Vec<u8>,
    unhandled: Vec<Vec<Vec<u8>>>,
    bells: usize,
}

impl vt100::Callbacks for Osc {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = title.to_vec();
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        self.unhandled
            .push(params.iter().map(|param| param.to_vec()).collect());
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bells += 1;
    }
}

fn parser() -> vt100::Parser<Osc> {
    vt100::Parser::new_with_callbacks(24, 80, 0, Osc::default())
}

/// Process `input` whole, then byte by byte, then in every two-way split,
/// and require every way to leave the same title, OSC log, bell count and
/// formatted screen: the limit must not depend on where reads end.
fn process_all_ways(input: &[u8]) -> vt100::Parser<Osc> {
    let mut whole = parser();
    whole.process(input);
    let expected = (
        whole.callbacks().title.clone(),
        whole.callbacks().unhandled.clone(),
        whole.callbacks().bells,
        whole.screen().contents_formatted(),
    );
    let mut ways: Vec<Vec<&[u8]>> =
        vec![input.chunks(1).collect(), input.chunks(7).collect()];
    // Every two-way split is quadratic; the long inputs check a spread.
    let step = (input.len() / 64).max(1);
    for at in (0..=input.len()).step_by(step) {
        let (a, b) = input.split_at(at);
        ways.push(vec![a, b]);
    }
    for chunks in ways {
        let mut split = parser();
        for chunk in &chunks {
            split.process(chunk);
        }
        let got = (
            split.callbacks().title.clone(),
            split.callbacks().unhandled.clone(),
            split.callbacks().bells,
            split.screen().contents_formatted(),
        );
        assert!(
            got == expected,
            "processing in {} chunks differs from processing whole",
            chunks.len(),
        );
    }
    whole
}

fn osc_with_title(len: usize, terminator: &[u8]) -> Vec<u8> {
    let mut input = b"\x1b]2;".to_vec();
    input.extend(std::iter::repeat(b't').take(len));
    input.extend_from_slice(terminator);
    input
}

#[test]
fn an_osc_at_the_limit_is_dispatched_whole() {
    // "2" counts toward the limit, ";" does not.
    let input = osc_with_title(LIMIT - 1, b"\x07");
    let parser = process_all_ways(&input);
    assert_eq!(parser.callbacks().title.len(), LIMIT - 1);
}

#[test]
fn an_osc_past_the_limit_keeps_its_first_bytes() {
    for terminator in [&b"\x07"[..], b"\x1b\\"] {
        let input = osc_with_title(3 * LIMIT, terminator);
        let parser = process_all_ways(&input);
        assert_eq!(
            parser.callbacks().title,
            vec![b't'; LIMIT - 1],
            "the title keeps the bytes within the limit",
        );
        // The BEL ended the OSC; it is not a bell.
        assert_eq!(parser.callbacks().bells, 0);
    }
}

#[test]
fn the_terminal_recovers_after_an_oversized_osc() {
    let mut input = osc_with_title(2 * LIMIT, b"\x07");
    input.extend_from_slice(b"before\x1b[31mred\x1b[m\x07");
    let parser = process_all_ways(&input);
    let screen = parser.screen();
    assert_eq!(screen.contents(), "beforered");
    assert_eq!(screen.cell(0, 6).unwrap().fgcolor(), vt100::Color::Idx(1));
    // The BEL after the OSC is an ordinary bell again.
    assert_eq!(parser.callbacks().bells, 1);
}

#[test]
fn an_escape_inside_the_dropped_bytes_starts_a_new_sequence() {
    // The oversized OSC is cut by a CSI, exactly as vte cuts any OSC at ESC.
    let mut input = b"\x1b]2;".to_vec();
    input.extend(std::iter::repeat(b'z').take(2 * LIMIT));
    input.extend_from_slice(b"\x1b[32mgreen");
    let parser = process_all_ways(&input);
    assert_eq!(parser.screen().contents(), "green");
    assert_eq!(
        parser.screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Idx(2)
    );
    assert_eq!(parser.callbacks().title, vec![b'z'; LIMIT - 1]);
}

#[test]
fn can_and_sub_end_an_oversized_osc() {
    for cancel in [0x18u8, 0x1A] {
        let mut input = b"\x1b]2;".to_vec();
        input.extend(std::iter::repeat(b'q').take(2 * LIMIT));
        input.push(cancel);
        input.extend_from_slice(b"text");
        let parser = process_all_ways(&input);
        assert_eq!(parser.screen().contents(), "text");
    }
}

#[test]
fn separators_past_the_limit_open_no_parameters() {
    // Parameters within the limit are kept, `;` past it is dropped with the
    // bytes around it: the dispatch sees two parameters, not thousands.
    let mut input = b"\x1b]777;".to_vec();
    input.extend(std::iter::repeat(b'p').take(LIMIT));
    input.extend(std::iter::repeat(b";p").take(LIMIT).flatten());
    input.push(0x07);
    let parser = process_all_ways(&input);
    let unhandled = &parser.callbacks().unhandled;
    assert_eq!(unhandled.len(), 1);
    assert_eq!(unhandled[0].len(), 2);
    assert_eq!(unhandled[0][0], b"777");
    assert_eq!(unhandled[0][1].len(), LIMIT - 3);
}

#[test]
fn osc_sequences_within_the_limit_are_unchanged() {
    // Many short OSCs of every terminator kind, interleaved with text and
    // controls, exactly as without a limit.
    let mut input = Vec::new();
    for i in 0..200 {
        input.extend_from_slice(format!("\x1b]2;title {i}\x07").as_bytes());
        input.extend_from_slice(
            format!("\x1b]8;;https://x/{i}\x1b\\").as_bytes(),
        );
        input.extend_from_slice(b"\x1b]9;4;1;50\x07line\r\n");
    }
    let parser = process_all_ways(&input);
    assert_eq!(parser.callbacks().title, b"title 199");
    assert_eq!(parser.callbacks().unhandled.len(), 400);
    assert_eq!(parser.callbacks().bells, 0);
}

#[test]
fn text_that_vte_prints_is_never_dropped() {
    // vte 0.15 drops an ESC in one case: a read that completes a UTF-8
    // character left incomplete by the previous read, when the bytes it
    // copies for that hold the character, an ESC and then an invalid or
    // incomplete byte. Here the second read starts "\x80 ESC \xC3": vte
    // completes "À", counts the ESC as consumed, and stays in ground, so
    // the "]" that follows is printed and so is everything after it. A
    // guard that took "ESC \xC3 ]" for an OSC opening would drop the text
    // past the limit, "TAIL" included.
    let mut parser = parser();
    parser.process(b"x\xc3");
    let mut second = b"\x80\x1b\xc3]".to_vec();
    second.extend(std::iter::repeat(b'a').take(LIMIT + 4096));
    second.extend_from_slice(b"TAIL");
    parser.process(&second);
    assert!(
        parser.screen().contents().ends_with("TAIL"),
        "the text vte printed reaches the screen"
    );
    assert!(
        parser.callbacks().unhandled.is_empty(),
        "no OSC was dispatched"
    );
}
