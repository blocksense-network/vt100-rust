//! A bound on the payload the parser keeps for one OSC sequence.
//!
//! `vte` with its `std` feature (which this crate uses) buffers the payload
//! of an OSC sequence (`ESC ]` … `BEL` / `ESC \`) in a `Vec` with no limit,
//! and an OSC ends only at `BEL`, `ESC`, `CAN` or `SUB`. A stray `ESC ]`
//! followed by output holding none of them (a binary written to the
//! terminal, plain text from a long-running program) therefore made the
//! parser buffer that output for as long as it lasted: without bound.
//!
//! [`OscGuard`] sits in front of `vte`. It follows `vte`'s transitions into
//! and out of the OSC string state, and once one sequence's payload reaches
//! [`OSC_PAYLOAD_LIMIT`] bytes it stops passing payload bytes to `vte` until
//! the sequence ends. That is the policy `vte` applies in its bounded
//! (`no_std`) build, where the payload is an `ArrayVec`: bytes past the
//! limit are dropped, a `;` past it opens no parameter, and the sequence is
//! still terminated and dispatched with the bytes kept. Everything outside
//! an oversized OSC payload is passed through untouched.
//!
//! The transitions followed here are those of `vte` 0.15
//! (`Parser::advance_esc`, `Parser::advance_osc_string`, `Parser::anywhere`
//! and the ground state; <https://github.com/alacritty/vte>), which follow
//! Paul Williams' DEC ANSI parser state diagram
//! (<https://vt100.net/emu/dec_ansi_parser>):
//!
//! - `ESC` (0x1B) enters the escape state from every state;
//! - in the escape state, C0 controls other than CAN/SUB, `ESC`, `DEL` and
//!   bytes 0x80..=0xFF leave it in the escape state; `]` (0x5D) enters the
//!   OSC string state; every other byte leaves it;
//! - in the OSC string state, `BEL` (0x07), `CAN` (0x18) and `SUB` (0x1A)
//!   end the sequence and return to ground, `ESC` ends it and enters the
//!   escape state, the other C0 controls are ignored, `;` (0x3B) separates
//!   parameters without being stored, and every other byte is payload.
//!
//! `vte` does not treat the 8-bit C1 `OSC` (0x9D) as an OSC opening (in
//! UTF-8 mode a lone 0x9D is executed as a control), so neither does this.
//!
//! # Never dropping what `vte` would show
//!
//! One case departs from those transitions: when a read ends inside a UTF-8
//! character, `vte` completes the character at the start of the next read,
//! and if the bytes it copies for that hold the character, an `ESC` and then
//! an invalid or incomplete byte, it counts the `ESC` as part of the
//! character and drops it. The guard then sees an `ESC ]` that `vte` never
//! acted on. Rather than reproduce `vte`'s UTF-8 decoding, the guard asks
//! `vte` itself: the caller's sink reports whether `vte` performed any
//! action (printed, executed, dispatched) on the bytes it was given. Inside
//! a real OSC string `vte` performs none until the sequence ends, so before
//! the guard drops a single byte it hands `vte` the payload so far and
//! checks; if `vte` acted, it is not in an OSC, and the guard stops guarding
//! this sequence. The guard therefore drops bytes only when `vte` is known to
//! be discarding them into its OSC buffer.

/// The most OSC payload bytes the parser keeps for one sequence.
///
/// Titles, hyperlinks (OSC 8), progress (OSC 9;4) and notifications fit with
/// room to spare; a clipboard write (OSC 52) keeps up to ~48 KiB of text.
pub const OSC_PAYLOAD_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not inside an escape or an OSC sequence as far as OSC bounds go.
    Other,
    /// After `ESC`: `]` would open an OSC.
    Escape,
    /// Inside an OSC string holding `kept` payload bytes. `acted` records
    /// whether `vte` performed any action on the bytes handed to it since
    /// the OSC opened (which would mean it is not in an OSC after all).
    Osc { kept: usize, acted: bool },
}

/// Tracks whether the bytes passed to `vte` are OSC payload, and drops the
/// payload past [`OSC_PAYLOAD_LIMIT`].
#[derive(Clone, Debug)]
pub struct OscGuard {
    state: State,
}

impl Default for OscGuard {
    fn default() -> Self {
        Self {
            state: State::Other,
        }
    }
}

impl OscGuard {
    /// Pass `bytes` to `sink` in order, leaving out the OSC payload bytes
    /// past the limit. `sink` receives contiguous runs of `bytes` and returns
    /// whether `vte` performed any action while processing the run. In the
    /// common case — no OSC opening — `sink` is called once with the whole
    /// input; an OSC opening splits the input right after its `]`.
    pub fn filter(
        &mut self,
        bytes: &[u8],
        mut sink: impl FnMut(&[u8]) -> bool,
    ) {
        let mut run_start = 0;
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                State::Other => {
                    // Fast path: only ESC matters here.
                    match bytes[i..].iter().position(|&b| b == 0x1B) {
                        Some(offset) => {
                            i += offset + 1;
                            self.state = State::Escape;
                        }
                        None => i = bytes.len(),
                    }
                }
                State::Escape => {
                    let byte = bytes[i];
                    i += 1;
                    self.state = match byte {
                        0x5D => {
                            // Hand `vte` everything up to the opening, so
                            // that what it reports from here on is about
                            // this OSC's payload alone.
                            sink(&bytes[run_start..i]);
                            run_start = i;
                            State::Osc {
                                kept: 0,
                                acted: false,
                            }
                        }
                        0x00..=0x17 | 0x19 | 0x1B..=0x1F | 0x7F..=0xFF => {
                            State::Escape
                        }
                        _ => State::Other,
                    };
                }
                State::Osc { kept, acted } => match bytes[i] {
                    0x07 | 0x18 | 0x1A => {
                        i += 1;
                        self.state = State::Other;
                    }
                    0x1B => {
                        i += 1;
                        self.state = State::Escape;
                    }
                    // Ignored by vte inside an OSC; harmless to pass.
                    0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F => {
                        i += 1;
                    }
                    _ if kept >= OSC_PAYLOAD_LIMIT => {
                        // Before dropping anything, hand `vte` the payload
                        // so far and make sure it really is in an OSC. That
                        // check is made at a point that does not end inside
                        // a UTF-8 character: if `vte` is not in an OSC, a
                        // split there could change how it decodes the bytes
                        // around it. The point is at most 3 bytes before
                        // `i`, and in ground `vte` acts on every byte but an
                        // incomplete character's, so the bytes before it
                        // show whether `vte` is in ground.
                        let check =
                            utf8_safe_split_before(bytes, run_start, i);
                        let acted = sink(&bytes[run_start..check]) || acted;
                        run_start = check;
                        if acted {
                            // `vte` is not in an OSC; stop guarding until
                            // the next opening.
                            i = check;
                            self.state = State::Other;
                            continue;
                        }
                        // `vte` is in the OSC, where a split anywhere is
                        // harmless: hand it the payload up to the limit.
                        if run_start < i {
                            sink(&bytes[run_start..i]);
                        }
                        // Drop every payload byte up to the next byte that
                        // ends the OSC.
                        let end = bytes[i..]
                            .iter()
                            .position(|&b| {
                                matches!(b, 0x07 | 0x18 | 0x1A | 0x1B)
                            })
                            .map_or(bytes.len(), |offset| i + offset);
                        i = end;
                        run_start = end;
                        self.state = State::Osc { kept, acted: false };
                    }
                    // A separator is not stored, so it does not count.
                    0x3B => i += 1,
                    _ => {
                        i += 1;
                        self.state = State::Osc {
                            kept: kept + 1,
                            acted,
                        };
                    }
                },
            }
        }
        if run_start < bytes.len() {
            let acted_now = sink(&bytes[run_start..]);
            // The run starts at or after the opening of the OSC the guard is
            // in (an opening always ends the run before it).
            if let State::Osc { acted, .. } = &mut self.state {
                *acted |= acted_now;
            }
        }
    }
}

/// The last position at or before `at` (and at most 3 bytes before it, never
/// before `start`) where `bytes[start..]` can be split without leaving an
/// incomplete UTF-8 character at the end of the first part.
fn utf8_safe_split_before(bytes: &[u8], start: usize, at: usize) -> usize {
    let mut split = at;
    while split > start
        && at - split < 3
        && ends_inside_utf8_char(&bytes[start..split])
    {
        split -= 1;
    }
    split
}

/// Whether `bytes` ends with the first bytes of a multi-byte UTF-8
/// character: a lead byte followed by fewer continuation bytes than it
/// announces (<https://www.rfc-editor.org/rfc/rfc3629#section-3>).
fn ends_inside_utf8_char(bytes: &[u8]) -> bool {
    let tail_start = bytes.len().saturating_sub(3);
    for (offset, &byte) in bytes[tail_start..].iter().enumerate().rev() {
        let following = bytes.len() - tail_start - offset - 1;
        let needed = match byte {
            0x80..=0xBF => continue,
            0xC2..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF4 => 3,
            _ => return false,
        };
        return following < needed;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(guard: &mut OscGuard, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        guard.filter(bytes, |run| {
            out.extend_from_slice(run);
            false
        });
        out
    }

    #[test]
    fn input_without_an_osc_passes_whole_in_one_run() {
        let mut guard = OscGuard::default();
        let input = b"text\x1b[31mred\x1b(Bmore\xc3\xa9";
        let mut runs = 0;
        guard.filter(input, |run| {
            runs += 1;
            assert_eq!(run, input);
            false
        });
        assert_eq!(runs, 1);
    }

    #[test]
    fn payload_past_the_limit_is_dropped_up_to_the_terminator() {
        let mut guard = OscGuard::default();
        let mut input = b"\x1b]0;".to_vec();
        input.extend(std::iter::repeat(b'x').take(OSC_PAYLOAD_LIMIT + 10));
        input.extend_from_slice(b"\x07after");
        let out = filtered(&mut guard, &input);
        let mut expected = b"\x1b]0;".to_vec();
        // "0" counts toward the limit, ";" does not.
        expected.extend(std::iter::repeat(b'x').take(OSC_PAYLOAD_LIMIT - 1));
        expected.extend_from_slice(b"\x07after");
        assert_eq!(out, expected);
    }

    #[test]
    fn the_limit_holds_across_calls() {
        let mut guard = OscGuard::default();
        let mut total = filtered(&mut guard, b"\x1b");
        total.extend(filtered(&mut guard, b"]"));
        for _ in 0..3 {
            total
                .extend(filtered(&mut guard, &vec![b'y'; OSC_PAYLOAD_LIMIT]));
        }
        total.extend(filtered(&mut guard, b"\x1b\\z"));
        assert_eq!(total.len(), 2 + OSC_PAYLOAD_LIMIT + 3);
    }

    #[test]
    fn nothing_is_dropped_when_vte_acted_on_the_payload() {
        // The sink reports that vte acted (it was not in an OSC): the guard
        // passes everything through.
        let mut guard = OscGuard::default();
        let mut input = b"\x1b]".to_vec();
        input.extend(std::iter::repeat(b'z').take(2 * OSC_PAYLOAD_LIMIT));
        let mut passed = 0;
        guard.filter(&input, |run| {
            passed += run.len();
            run.contains(&b'z')
        });
        assert_eq!(passed, input.len());
    }

    #[test]
    fn a_split_never_ends_inside_a_utf8_character() {
        assert!(ends_inside_utf8_char(b"a\xc3"));
        assert!(ends_inside_utf8_char(b"\xe2\x82"));
        assert!(ends_inside_utf8_char(b"\xf0\x9f\x98"));
        assert!(!ends_inside_utf8_char(b"\xf0\x9f\x98\x80"));
        assert!(!ends_inside_utf8_char(b"a\xc3\xa9"));
        assert!(!ends_inside_utf8_char(b"\x80\x80\x80"));
        assert!(!ends_inside_utf8_char(b""));
        assert_eq!(utf8_safe_split_before(b"ab\xe2\x82\xacc", 0, 4), 2);
        assert_eq!(utf8_safe_split_before(b"ab\xe2\x82\xacc", 0, 5), 5);
        assert_eq!(utf8_safe_split_before(b"ab\xe2\x82\xacc", 3, 4), 4);
    }
}
