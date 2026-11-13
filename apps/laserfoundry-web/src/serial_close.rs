//! The order in which a Web Serial port is shut down, kept free of browser types so it can be tested natively.
//!
//! A port that is not closed stays open and locked by the tab until the tab goes away, so the next
//! Connect fails with "Failed to open serial port". The browser calls are in `lib.rs`.
//! See <https://developer.mozilla.org/en-US/docs/Web/API/SerialPort/close> (close needs both locks released first).

/// One step of shutting a port down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseStep {
    /// Cancel the reader so the pending read finishes.
    CancelReader,
    /// Close the writer after what it has queued.
    CloseWriter,
    ReleaseReader,
    ReleaseWriter,
    ClosePort,
}

/// What was acquired before shutting down (a failed connect may have got only part of it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub reader: bool,
    pub writer: bool,
}

/// Runs the shutdown once: the first [`Teardown::steps`] call returns the plan, later calls (a disconnect
/// followed by the drop of the transport) return nothing so the port is never closed twice.
#[derive(Debug, Default)]
pub struct Teardown {
    done: std::cell::Cell<bool>,
}

impl Teardown {
    pub fn steps(&self, held: Held) -> Vec<CloseStep> {
        if self.done.replace(true) {
            return Vec::new();
        }
        let mut steps = Vec::new();
        if held.reader {
            steps.push(CloseStep::CancelReader);
        }
        if held.writer {
            steps.push(CloseStep::CloseWriter);
        }
        if held.reader {
            steps.push(CloseStep::ReleaseReader);
        }
        if held.writer {
            steps.push(CloseStep::ReleaseWriter);
        }
        steps.push(CloseStep::ClosePort);
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_the_port_last_after_both_locks_are_released() {
        let steps = Teardown::default().steps(Held { reader: true, writer: true });
        assert_eq!(steps.last(), Some(&CloseStep::ClosePort));
        let at = |s| steps.iter().position(|x| *x == s);
        assert!(at(CloseStep::CancelReader) < at(CloseStep::ReleaseReader));
        assert!(at(CloseStep::CloseWriter) < at(CloseStep::ReleaseWriter));
        assert_eq!(steps.len(), 5);
    }

    #[test]
    fn a_failed_connect_that_got_no_locks_still_closes_the_port() {
        assert_eq!(Teardown::default().steps(Held::default()), vec![CloseStep::ClosePort]);
        let reader_only = Teardown::default().steps(Held { reader: true, writer: false });
        assert!(!reader_only.contains(&CloseStep::ReleaseWriter));
        assert_eq!(reader_only.last(), Some(&CloseStep::ClosePort));
    }

    #[test]
    fn disconnect_then_drop_closes_once_and_a_fresh_connection_closes_again() {
        let held = Held { reader: true, writer: true };
        let first = Teardown::default();
        assert!(!first.steps(held).is_empty());
        assert!(first.steps(held).is_empty(), "the drop after a disconnect must not close twice");
        // Connect again on the same tab: a new connection has its own teardown.
        assert!(!Teardown::default().steps(held).is_empty());
    }
}
