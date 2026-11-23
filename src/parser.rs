/// A parser for terminal output which produces an in-memory representation of
/// the terminal contents.
pub struct Parser<CB: crate::callbacks::Callbacks = ()> {
    parser: vte::Parser,
    /// Keeps the OSC payload `vte` buffers within
    /// [`OSC_PAYLOAD_LIMIT`](crate::OSC_PAYLOAD_LIMIT).
    osc_guard: crate::osc_guard::OscGuard,
    screen: crate::perform::WrappedScreen<CB>,
}

impl Parser {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback.
    #[must_use]
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self {
            parser: vte::Parser::new(),
            osc_guard: crate::osc_guard::OscGuard::default(),
            screen: crate::perform::WrappedScreen::new(
                rows,
                cols,
                scrollback_len,
            ),
        }
    }
}

impl<CB: crate::callbacks::Callbacks> Parser<CB> {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback. Terminal events will be reported via method
    /// calls on the provided [`Callbacks`](crate::callbacks::Callbacks)
    /// implementation.
    pub fn new_with_callbacks(
        rows: u16,
        cols: u16,
        scrollback_len: usize,
        callbacks: CB,
    ) -> Self {
        Self {
            parser: vte::Parser::new(),
            osc_guard: crate::osc_guard::OscGuard::default(),
            screen: crate::perform::WrappedScreen::new_with_callbacks(
                rows,
                cols,
                scrollback_len,
                callbacks,
            ),
        }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state.
    ///
    /// The payload kept for one OSC sequence is limited to
    /// [`OSC_PAYLOAD_LIMIT`](crate::OSC_PAYLOAD_LIMIT) bytes; the rest of an
    /// oversized OSC is dropped, so the parser's memory does not grow with
    /// its input.
    pub fn process(&mut self, bytes: &[u8]) {
        let Self {
            parser,
            osc_guard,
            screen,
        } = self;
        osc_guard.filter(bytes, |run| {
            let mut observed = Observed {
                inner: screen,
                acted: false,
            };
            parser.advance(&mut observed, run);
            observed.acted
        });
    }

    /// Returns a reference to a [`Screen`](crate::Screen) object containing
    /// the terminal state.
    #[must_use]
    pub fn screen(&self) -> &crate::Screen {
        &self.screen.screen
    }

    /// Returns a mutable reference to a [`Screen`](crate::Screen) object
    /// containing the terminal state.
    #[must_use]
    pub fn screen_mut(&mut self) -> &mut crate::Screen {
        &mut self.screen.screen
    }

    /// Returns a reference to the [`Callbacks`](crate::callbacks::Callbacks)
    /// state object passed into the constructor.
    pub fn callbacks(&self) -> &CB {
        &self.screen.callbacks
    }

    /// Returns a mutable reference to the
    /// [`Callbacks`](crate::callbacks::Callbacks) state object passed into
    /// the constructor.
    pub fn callbacks_mut(&mut self) -> &mut CB {
        &mut self.screen.callbacks
    }
}

impl Default for Parser {
    /// Returns a parser with dimensions 80x24 and no scrollback.
    fn default() -> Self {
        Self::new(24, 80, 0)
    }
}

impl std::io::Write for Parser {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A [`vte::Perform`] that forwards every action to `inner` and records
/// whether there was any. The OSC guard uses it to learn whether `vte` acted
/// on bytes the guard took for OSC payload (see `osc_guard`).
struct Observed<'a, P: vte::Perform> {
    inner: &'a mut P,
    acted: bool,
}

impl<P: vte::Perform> vte::Perform for Observed<'_, P> {
    fn print(&mut self, c: char) {
        self.acted = true;
        self.inner.print(c);
    }

    fn execute(&mut self, byte: u8) {
        self.acted = true;
        self.inner.execute(byte);
    }

    fn hook(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.acted = true;
        self.inner.hook(params, intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        self.acted = true;
        self.inner.put(byte);
    }

    fn unhook(&mut self) {
        self.acted = true;
        self.inner.unhook();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.acted = true;
        self.inner.osc_dispatch(params, bell_terminated);
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.acted = true;
        self.inner
            .csi_dispatch(params, intermediates, ignore, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.acted = true;
        self.inner.esc_dispatch(intermediates, ignore, byte);
    }

    fn terminated(&self) -> bool {
        self.inner.terminated()
    }
}
