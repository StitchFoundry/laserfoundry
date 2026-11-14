//! The modem-control lines (DTR and RTS) a Web Serial port is left in, kept free of browser types so it can be tested natively.
//!
//! Many ESP32 boards wire DTR and RTS to the reset (EN) and boot (GPIO0) pins, and the ESP32-S3 / C3 native USB port reacts
//! to the same line changes. Opening a port can therefore reset the board, and a board that reset may start GRBL in Alarm.
//! The browser has no open option for the initial line state, so the lines are driven low right after `open` and again before
//! `close`. Only `setSignals` is used: nothing is written to the controller, so no S word, M3 or M4 can come from here.
//! See <https://developer.mozilla.org/en-US/docs/Web/API/SerialPort/setSignals>.

/// The requested state of the two output lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signals {
    pub data_terminal_ready: bool,
    pub request_to_send: bool,
}

/// Both lines low: the state in which a typical auto-reset circuit holds the board running instead of resetting it.
pub const IDLE: Signals = Signals { data_terminal_ready: false, request_to_send: false };

/// Where in the life of the port the lines are set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    AfterOpen,
    BeforeClose,
}

/// The lines to set at `moment`. Both moments want them low, so a board is never left holding reset or the boot pin.
pub fn lines_at(moment: Moment) -> Signals {
    match moment {
        Moment::AfterOpen | Moment::BeforeClose => IDLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_lines_are_low_after_open_and_before_close() {
        for moment in [Moment::AfterOpen, Moment::BeforeClose] {
            let signals = lines_at(moment);
            assert!(!signals.data_terminal_ready, "DTR high can reset the board at {moment:?}");
            assert!(!signals.request_to_send, "RTS high can hold the board in reset at {moment:?}");
        }
    }
}
