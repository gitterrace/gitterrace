//! Export progress on stderr: one rewritten line on a terminal, a plain line now and then
//! when piped, nothing with `--quiet`.

use std::{
    io::Write,
    time::{Duration, Instant},
};

/// Seconds between plain lines when stderr is not a terminal.
const PLAIN_EVERY: Duration = Duration::from_secs(5);
const TTY_EVERY: Duration = Duration::from_millis(100);

/// How progress reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// One line, rewritten in place.
    Tty,
    /// A full line every few seconds, for logs and pipes.
    Plain,
    Quiet,
}

impl Mode {
    /// The mode for this process: quiet when asked, else by whether stderr is a terminal.
    pub(crate) fn detect(quiet: bool) -> Self {
        use std::io::IsTerminal as _;
        if quiet {
            Self::Quiet
        } else if std::io::stderr().is_terminal() {
            Self::Tty
        } else {
            Self::Plain
        }
    }
}

/// The reporter one export drives.
pub(crate) struct Progress {
    mode: Mode,
    out: Box<dyn Write + Send>,
    start: Instant,
    last: Option<Instant>,
    /// Whether a terminal line is open and needs a newline before other output.
    open: bool,
    /// Width of the open terminal line, so a shorter one can blank what it leaves.
    width: usize,
}

impl Progress {
    pub(crate) fn new(mode: Mode, out: Box<dyn Write + Send>) -> Self {
        Self { mode, out, start: Instant::now(), last: None, open: false, width: 0 }
    }

    /// A reporter that prints nothing, for tests.
    #[cfg(test)]
    pub(crate) fn quiet() -> Self {
        Self::new(Mode::Quiet, Box::new(std::io::sink()))
    }

    /// A phase change: always shown, so the user sees what runs now.
    pub(crate) fn phase(&mut self, what: &str) {
        let line = format!("{what} ({})", self.elapsed());
        self.emit(&line, true);
    }

    /// Pages written so far against the pages known so far, which grows as links are found.
    pub(crate) fn pages(&mut self, written: usize, planned: usize) {
        let now = Instant::now();
        let every = if self.mode == Mode::Tty { TTY_EVERY } else { PLAIN_EVERY };
        if self.last.is_some_and(|t| now.duration_since(t) < every) {
            return;
        }
        let line = format!("writing pages: {written} / {planned} ({})", self.elapsed());
        self.emit(&line, false);
    }

    /// The final count, ending the terminal line.
    pub(crate) fn done(&mut self, written: usize) {
        let line = format!("wrote {written} pages ({})", self.elapsed());
        self.emit(&line, true);
        if self.open {
            self.write("\n");
            self.open = false;
        }
    }

    fn elapsed(&self) -> String {
        let s = self.start.elapsed().as_secs();
        format!("{}:{:02}", s / 60, s % 60)
    }

    fn emit(&mut self, line: &str, force: bool) {
        // A phase line does not hold back the first count after it.
        self.last = (!force).then(Instant::now);
        match self.mode {
            Mode::Quiet => {}
            Mode::Plain => self.write(&format!("{line}\n")),
            Mode::Tty => {
                // \r back to column 0; spaces, not an escape code, blank a longer old line.
                let pad = self.width.saturating_sub(line.len());
                self.write(&format!("\r{line}{}", " ".repeat(pad)));
                self.width = line.len();
                self.open = true;
                if force {
                    // A phase stays on screen; the next update starts a new line.
                    self.write("\n");
                    self.open = false;
                    self.width = 0;
                }
            }
        }
    }

    fn write(&mut self, s: &str) {
        // Progress is a courtesy: a closed stderr must not fail the export.
        if self.out.write_all(s.as_bytes()).is_ok() {
            self.out.flush().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{Mode, Progress};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn run(mode: Mode) -> String {
        let buf = Shared::default();
        let mut p = Progress::new(mode, Box::new(buf.clone()));
        p.phase("building the cache");
        p.pages(1, 10);
        p.pages(2, 12);
        p.done(12);
        let bytes = buf.0.lock().expect("lock").clone();
        String::from_utf8(bytes).expect("utf8")
    }

    #[test]
    fn a_terminal_gets_one_rewritten_line_with_counts_and_time() {
        let s = run(Mode::Tty);
        assert!(!s.contains('\x1b'), "no escape codes: {s:?}");
        assert!(s.contains("\rwriting pages: 1 / 10 (0:00)"), "{s:?}");
        assert!(s.contains("building the cache (0:00)"), "{s:?}");
        // The count line is rewritten in place by the final line, padded over its tail.
        assert!(s.contains("(0:00)\rwrote 12 pages (0:00)"), "{s:?}");
        assert!(s.ends_with("pages (0:00)       \n"), "{s:?}");
        // Throttled: the second update inside the redraw gap is dropped.
        assert!(!s.contains("2 / 12"), "{s:?}");
    }

    #[test]
    fn a_pipe_gets_plain_lines_with_no_escapes() {
        let s = run(Mode::Plain);
        assert!(!s.contains('\r') && !s.contains('\x1b'), "{s:?}");
        assert_eq!(
            s.lines().collect::<Vec<_>>(),
            ["building the cache (0:00)", "writing pages: 1 / 10 (0:00)", "wrote 12 pages (0:00)"]
        );
    }

    #[test]
    fn quiet_prints_nothing() {
        assert_eq!(run(Mode::Quiet), "");
    }
}
