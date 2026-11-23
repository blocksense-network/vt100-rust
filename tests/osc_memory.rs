//! The parser's memory stays bounded however long an OSC sequence runs.
//!
//! An OSC (`ESC ]`) sequence ends only at BEL, ST (`ESC \`), CAN or SUB, so
//! a stray `ESC ]` followed by output that contains none of them (a binary
//! file written to the terminal, a program that prints plain text for hours)
//! keeps the parser inside the OSC for as long as that output lasts. `vte`
//! with its `std` feature buffers every such byte, so before the limit the
//! parser's heap grew by exactly the size of the output: in the field one
//! recording process reached tens of gigabytes this way.
//!
//! This file holds a single test on purpose: it measures the process heap
//! with a counting global allocator, and a second test running on another
//! thread would allocate into the same counters.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The system allocator, counting the bytes currently allocated and the
/// highest that count has been since the last [`reset_peak`].
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to `System` unchanged; only counts sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst)
                + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> *mut u8 {
        let new = System.realloc(ptr, layout, new_size);
        if !new.is_null() {
            if new_size >= layout.size() {
                let live = LIVE
                    .fetch_add(new_size - layout.size(), Ordering::SeqCst)
                    + (new_size - layout.size());
                PEAK.fetch_max(live, Ordering::SeqCst);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::SeqCst);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn reset_peak() -> usize {
    let live = LIVE.load(Ordering::SeqCst);
    PEAK.store(live, Ordering::SeqCst);
    live
}

/// How much output follows each OSC opening: 64 MiB, 1024 times the limit,
/// so a parser that buffers the OSC cannot pass the budget by accident.
const OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Output arrives in chunks of this size, as a PTY read delivers it.
const CHUNK: usize = 64 * 1024;

/// Feed `opening`, then [`OUTPUT_BYTES`] of `filler` repeated in [`CHUNK`]-sized
/// pieces, to a fresh parser, and return how far the heap rose above where
/// it was before the parser existed.
fn heap_growth(opening: &[u8], filler: &[u8]) -> usize {
    let chunk: Vec<u8> = filler.iter().copied().cycle().take(CHUNK).collect();
    let before = reset_peak();
    {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(opening);
        for _ in 0..OUTPUT_BYTES / CHUNK {
            parser.process(&chunk);
        }
        // The parser still works once the OSC finally ends.
        parser.process(b"\x07after");
        assert!(
            parser.screen().contents().contains("after"),
            "the parser recovers after the OSC ends",
        );
    }
    PEAK.load(Ordering::SeqCst).saturating_sub(before)
}

#[test]
fn an_unterminated_osc_keeps_the_heap_bounded() {
    // The budget: the parser's own grid plus a few times the OSC limit
    // (`Vec` doubling can leave up to twice the limit allocated) — four
    // orders of magnitude below the output it reads.
    let budget = 4 * vt100::OSC_PAYLOAD_LIMIT + 1024 * 1024;
    let cases: &[(&str, &[u8], &[u8])] = &[
        ("a plain OSC opening", b"\x1b]", b"a"),
        ("a title OSC", b"\x1b]0;", b"x"),
        // Separators between payload bytes: every `;` opens a parameter.
        ("an OSC of short parameters", b"\x1b]", b"a;"),
        // Bytes vte ignores inside an OSC (here LF) do not end it.
        ("an OSC of lines", b"\x1b]2;", b"line of text\n"),
        // C0 controls between ESC and `]` keep vte in the escape state, so
        // this still opens an OSC.
        ("an opening interrupted by C0", b"\x1b\r\x00]", b"z"),
        // An OSC opened from inside a CSI sequence and after a DCS.
        ("an opening inside a CSI", b"\x1b[12\x1b]", b"c"),
        ("an opening after a DCS", b"\x1bPq#0\x1b]", b"d"),
        // Bytes above 0x7F are OSC payload too.
        ("an OSC of high bytes", b"\x1b]8;;", b"\xc3\xa9"),
    ];
    for (what, opening, filler) in cases {
        let growth = heap_growth(opening, filler);
        assert!(
            growth <= budget,
            "{what}: the heap rose {growth} bytes while the parser read \
             {OUTPUT_BYTES} bytes of OSC payload; the budget is {budget}",
        );
    }
}
