//! A bound on the payload the parser keeps for one OSC sequence.
//!
//! `vte` with its `std` feature (which this crate uses) buffers the payload
//! of an OSC sequence (`ESC ]` … `BEL` / `ESC \`) in a `Vec` with no limit,
//! and an OSC ends only at `BEL`, `ESC`, `CAN` or `SUB`. A stray `ESC ]`
//! followed by output holding none of them (a binary written to the
//! terminal, plain text from a long-running program) therefore made the
//! parser buffer that output for as long as it lasted: without bound.
//!
//! [`OscGuard`] sits in front of `vte`. It follows `vte`'s own transitions
//! into and out of the OSC string state, and once one sequence's payload
//! reaches [`OSC_PAYLOAD_LIMIT`] bytes it stops passing payload bytes to
//! `vte` until the sequence ends. That is the policy `vte` applies in its
//! bounded (`no_std`) build, where the payload is an `ArrayVec`: bytes past
//! the limit are dropped, a `;` past it opens no parameter, and the sequence
//! is still terminated and dispatched with the bytes kept. Everything outside
//! an OSC payload is passed through untouched.
//!
//! The transitions mirrored here are those of `vte` 0.15
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
    /// Inside an OSC string, holding `kept` payload bytes.
    Osc { kept: usize },
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
    /// Pass `bytes` to `sink` in order, leaving out the OSC payload bytes past
    /// the limit. `sink` receives contiguous runs of `bytes`, so the common
    /// case — no oversized OSC — costs one call with the whole input.
    pub fn filter(&mut self, bytes: &[u8], mut sink: impl FnMut(&[u8])) {
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
                        0x5D => State::Osc { kept: 0 },
                        0x00..=0x17 | 0x19 | 0x1B..=0x1F | 0x7F..=0xFF => {
                            State::Escape
                        }
                        _ => State::Other,
                    };
                }
                State::Osc { kept } => {
                    let byte = bytes[i];
                    match byte {
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
                            // Drop this byte and every following payload
                            // byte up to the next byte that ends the OSC.
                            if run_start < i {
                                sink(&bytes[run_start..i]);
                            }
                            let end = bytes[i..]
                                .iter()
                                .position(|&b| {
                                    matches!(b, 0x07 | 0x18 | 0x1A | 0x1B)
                                })
                                .map_or(bytes.len(), |offset| i + offset);
                            i = end;
                            run_start = end;
                        }
                        // A separator is not stored, so it does not count.
                        0x3B => i += 1,
                        _ => {
                            i += 1;
                            self.state = State::Osc { kept: kept + 1 };
                        }
                    }
                }
            }
        }
        if run_start < bytes.len() {
            sink(&bytes[run_start..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(guard: &mut OscGuard, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        guard.filter(bytes, |run| out.extend_from_slice(run));
        out
    }

    #[test]
    fn input_without_an_oversized_osc_passes_whole_in_one_run() {
        let mut guard = OscGuard::default();
        let input = b"text\x1b[31mred\x1b]0;title\x07\x1b]8;;u\x1b\\more";
        let mut runs = 0;
        guard.filter(input, |run| {
            runs += 1;
            assert_eq!(run, input);
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
}
