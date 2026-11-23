//! Fire: a momentary, low-power laser for focusing and aiming. It is hold-to-fire, never a toggle: the caller sends
//! [`Link::fire_on`] when the button goes down and [`Link::fire_off`] when it comes up (or the window blurs, the tab hides, ...).
//! The link adds its own limits, which do not depend on the caller behaving:
//!
//! - the power is capped at [`FIRE_MAX_PERCENT`] of the controller's `$30` (max spindle speed) and is never a layer's power;
//! - the laser goes off after [`FIRE_MAX_DURATION`] even if still held, and fires again only after a release;
//! - only GRBL in laser mode (`$32=1`; in spindle mode `M3` would run the laser constantly), idle, with no job and no command
//!   still unanswered (a frame, a jog), and with a known `$30`;
//! - an alarm, any state other than idle, an `error:` or `ALARM:` reply, a stop, a lost connection or a dropped link switch it
//!   off (the last two with a soft reset), and jogs and other lines are refused while it is on.
//!
//! GRBL laser-mode `M3` emits while the head is still, which is what makes aiming work.
//! <https://github.com/gnea/grbl/wiki/Grbl-v1.1-Laser-Mode>

use super::*;

/// What the Fire power setting starts at, in percent of the controller's maximum power.
pub const FIRE_DEFAULT_PERCENT: f64 = 1.0;
/// The most Fire may ever use, in percent. Not configurable above this.
pub const FIRE_MAX_PERCENT: f64 = 5.0;
/// How long the laser stays on while the button is held.
pub const FIRE_MAX_DURATION: Duration = Duration::from_secs(10);

/// Laser off, and the programmed power back to zero.
const FIRE_OFF_LINE: &str = "M5 S0";
/// Largest `$30` believed; anything above is a misread.
const MAX_S_RANGE: f64 = 1_000_000.0;

/// The line that switches the laser on at `percent` of `max_s` (`$30`), refusing non-finite or out-of-range values.
pub fn fire_line(percent: f64, max_s: f64) -> Result<String, LinkError> {
    if !percent.is_finite() || percent <= 0.0 || percent > FIRE_MAX_PERCENT {
        return Err(LinkError::Rejected(format!("Fire power must be above 0 and at most {FIRE_MAX_PERCENT}%")));
    }
    if !max_s.is_finite() || max_s <= 0.0 || max_s > MAX_S_RANGE {
        return Err(LinkError::Rejected("the controller's maximum power ($30) is not known".into()));
    }
    let s = (percent / 100.0 * max_s).round();
    if s < 1.0 {
        return Err(LinkError::Rejected("the controller's power range is too coarse for that Fire power".into()));
    }
    // At most 5% of MAX_S_RANGE, so the conversion is exact.
    Ok(format!("M3 S{}", s as u32))
}

impl Link {
    /// Switches the laser on at `percent` of the controller's maximum power (hold-to-fire: pair every call with
    /// [`fire_off`](Link::fire_off)). Refused, with the reason, unless every condition in the module docs holds. Does nothing while
    /// already firing; after the time cap or an alarm it is refused until `fire_off` has been called.
    pub fn fire_on(&self, percent: f64) -> Result<(), LinkError> {
        if self.protocol != Protocol::Grbl {
            return Err(LinkError::Rejected("Fire is only available on GRBL controllers".into()));
        }
        let snap = self.snapshot();
        if !snap.connected {
            return Err(LinkError::Disconnected);
        }
        if matches!(snap.job, JobState::Running | JobState::Paused) {
            return Err(LinkError::Busy);
        }
        match snap.status.as_ref().map(|s| s.state) {
            Some(MachineState::Idle) => {}
            Some(state) => return Err(LinkError::Rejected(format!("the machine is not idle ({state:?})"))),
            None => return Err(LinkError::Rejected("the machine has not reported its state yet".into())),
        }
        let number = |id: u32| snap.settings.get(&id).and_then(|v| v.trim().parse::<f64>().ok());
        if number(32) != Some(1.0) {
            return Err(LinkError::Rejected("Fire needs laser mode ($32=1); in spindle mode the laser could stay on".into()));
        }
        let line = fire_line(percent, number(30).unwrap_or(0.0))?;
        {
            let mut s = self.shared.lock().map_err(|_| LinkError::Disconnected)?;
            if s.fire_latched {
                return Err(LinkError::Rejected("Fire was cut off; release it and press again".into()));
            }
            if s.firing {
                return Ok(());
            }
            s.firing = true;
        }
        self.send(Cmd::FireOn(line)).inspect_err(|_| self.clear_firing(false))
    }

    /// Switches the laser off (the button was released, or anything else made firing unsafe). Always safe to call.
    pub fn fire_off(&self) -> Result<(), LinkError> {
        self.clear_firing(false);
        if let Ok(mut s) = self.shared.lock() {
            s.fire_latched = false;
        }
        self.send(Cmd::FireOff)
    }

    /// Whether Fire is on (or about to be).
    pub fn is_firing(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.firing)
    }

    fn clear_firing(&self, latch: bool) {
        if let Ok(mut s) = self.shared.lock() {
            s.firing = false;
            s.fire_latched |= latch;
        }
    }
}

impl Worker {
    /// Switches the laser on, unless a job or an unanswered command (a frame, a jog) is in the way.
    pub(super) fn fire_on(&mut self, line: String) -> Result<(), DeviceError> {
        if self.job.is_some() || !self.pending.is_empty() || self.fire_started.is_some() {
            self.log("! Fire refused: the machine is busy");
            self.update(|s| s.firing = false);
            return Ok(());
        }
        // Counted as on before the write, so a failed write is followed by an off.
        self.fire_started = Some(Instant::now());
        self.send_immediate(line)
    }

    /// Switches the laser off if Fire has it on. `latch` keeps Fire from firing again until it is released.
    pub(super) fn fire_off(&mut self, why: &str, latch: bool) -> Result<(), DeviceError> {
        if self.fire_started.take().is_none() {
            return Ok(());
        }
        self.update(|s| {
            s.firing = false;
            s.fire_latched |= latch;
        });
        self.log(format!("fire off: {why}"));
        self.send_immediate(FIRE_OFF_LINE.into())
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    mod refusals {
        use super::*;

        #[test]
        fn over_the_cap_or_not_a_number_sends_nothing() {
            let (link, wire) = ready("1", "Idle");
            for p in [5.01, 50.0, 100.0, 0.0, -1.0, f64::NAN, f64::INFINITY] {
                assert!(link.fire_on(p).is_err(), "{p}");
            }
            settle(&link);
            assert!(!link.is_firing());
            assert!(sent(&wire).is_empty(), "{:?}", sent(&wire));
            assert!(fire_line(1.0, 0.0).is_err() && fire_line(1.0, f64::NAN).is_err() && fire_line(1.0, 40.0).is_err());
            assert_eq!(fire_line(FIRE_DEFAULT_PERCENT, 1000.0).unwrap(), "M3 S10");
            assert_eq!(fire_line(FIRE_MAX_PERCENT, 1000.0).unwrap(), "M3 S50");
        }

        #[test]
        fn spindle_mode_or_an_unknown_mode_is_refused() {
            for mode in ["0", "x"] {
                let (link, wire) = ready(mode, "Idle");
                assert!(matches!(link.fire_on(1.0), Err(LinkError::Rejected(m)) if m.contains("$32")));
                assert!(sent(&wire).is_empty());
            }
        }

        #[test]
        fn alarm_and_other_busy_states_are_refused() {
            for state in ["Alarm", "Run", "Hold:0", "Door:0", "Jog"] {
                let (link, wire) = ready("1", state);
                assert!(link.fire_on(1.0).is_err(), "{state}");
                assert!(sent(&wire).is_empty());
            }
        }

        #[test]
        fn a_job_or_an_unanswered_command_blocks_it() {
            let (link, wire) = ready("1", "Idle");
            link.start_job("G21\nG0 X1\n".into()).unwrap();
            settle(&link);
            assert_eq!(link.fire_on(1.0), Err(LinkError::Busy));
            assert!(!sent(&wire).iter().any(|l| l.starts_with("M3")));
            // A frame or jog still unanswered in the controller's buffer.
            let (link, wire) = ready("1", "Idle");
            link.jog(5.0, 0.0, 1000.0).unwrap();
            assert!(link.fire_on(1.0).is_ok(), "the check is the worker's, in order");
            settle(&link);
            assert!(!link.is_firing(), "the worker refused it");
            assert_eq!(sent(&wire), vec!["$J=G91 G21 X5 F1000"]);
        }

        #[test]
        fn marlin_is_refused() {
            let wire = Arc::new(Mutex::new(Wire::default()));
            let link = Link::connect_inline_with(Box::new(Scripted(Arc::clone(&wire))), Protocol::Marlin);
            assert!(link.fire_on(1.0).is_err());
        }
    }

    mod hold_to_fire {
        use super::*;

        #[test]
        fn press_sends_one_fire_command_and_release_sends_m5() {
            let (link, wire) = ready("1", "Idle");
            link.fire_on(1.0).unwrap();
            link.fire_on(1.0).unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10"], "one command however often the press is repeated");
            assert!(link.is_firing());
            reply(&wire, "ok");
            link.fire_off().unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10", "M5 S0"]);
            assert!(!link.is_firing());
        }

        #[test]
        fn the_time_cap_switches_it_off_and_needs_a_fresh_press() {
            let (link, wire) = ready("1", "Idle");
            set_limit(&link, Duration::from_millis(30));
            link.fire_on(1.0).unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10"]);
            std::thread::sleep(Duration::from_millis(60));
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10", "M5 S0"]);
            assert!(!link.is_firing());
            // Still held: a repeated press does nothing until it has been released.
            assert!(link.fire_on(1.0).is_err());
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10", "M5 S0"]);
            reply(&wire, "ok");
            reply(&wire, "ok");
            link.fire_off().unwrap();
            settle(&link);
            link.fire_on(1.0).unwrap();
            settle(&link);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("M3 S10"));
        }

        #[test]
        fn an_alarm_or_an_error_switches_it_off() {
            for report in ["<Alarm|MPos:0.000,0.000,0.000|FS:0,0>", "<Hold:0|MPos:0.000,0.000,0.000|FS:0,0>", "error:8", "ALARM:1"] {
                let (link, wire) = ready("1", "Idle");
                link.fire_on(1.0).unwrap();
                settle(&link);
                reply(&wire, report);
                settle(&link);
                assert_eq!(sent(&wire).last().map(String::as_str), Some("M5 S0"), "{report}");
                assert!(!link.is_firing(), "{report}");
            }
        }

        #[test]
        fn stop_and_dropping_the_link_soft_reset_the_controller() {
            let (link, wire) = ready("1", "Idle");
            link.fire_on(1.0).unwrap();
            settle(&link);
            link.stop().unwrap();
            settle(&link);
            assert!(wire.lock().unwrap().written.contains(&realtime::SOFT_RESET));
            assert!(!link.is_firing());

            let (link, wire) = ready("1", "Idle");
            link.fire_on(1.0).unwrap();
            settle(&link);
            assert!(!wire.lock().unwrap().written.contains(&realtime::SOFT_RESET));
            drop(link);
            assert!(wire.lock().unwrap().written.contains(&realtime::SOFT_RESET), "disconnecting resets the controller");
        }

        #[test]
        fn jogs_are_blocked_while_firing_and_off_goes_out_before_the_next_jog() {
            let (link, wire) = ready("1", "Idle");
            link.fire_on(1.0).unwrap();
            assert_eq!(link.jog(5.0, 0.0, 1000.0), Err(LinkError::Firing));
            assert_eq!(link.send_line("G0 X1"), Err(LinkError::Firing));
            assert_eq!(link.start_job("G21\n".into()), Err(LinkError::Firing));
            // Off goes out ahead of a jog queued right after the release.
            link.fire_off().unwrap();
            link.jog(5.0, 0.0, 1000.0).unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M3 S10", "M5 S0", "$J=G91 G21 X5 F1000"]);
        }
    }

    /// What a scripted GRBL controller has been sent, and the answers it will give (one per read).
    #[derive(Default)]
    struct Wire {
        written: Vec<u8>,
        replies: VecDeque<String>,
    }

    struct Scripted(Arc<Mutex<Wire>>);

    impl Transport for Scripted {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.lock().unwrap().written.extend_from_slice(bytes);
            Ok(())
        }

        fn read_line(&mut self, _: Duration) -> io::Result<Option<String>> {
            Ok(self.0.lock().unwrap().replies.pop_front())
        }
    }

    /// A GRBL with `$30=1000`, `$32=laser_mode` and a first status report in `state`, with nothing sent to it yet.
    fn ready(laser_mode: &str, state: &str) -> (Link, Arc<Mutex<Wire>>) {
        let wire = Arc::new(Mutex::new(Wire::default()));
        let link = Link::connect_inline_with(Box::new(Scripted(Arc::clone(&wire))), Protocol::Grbl);
        reply(&wire, &format!("<{state}|MPos:0.000,0.000,0.000|FS:0,0>"));
        reply(&wire, "$30=1000");
        reply(&wire, &format!("$32={laser_mode}"));
        settle(&link);
        wire.lock().unwrap().written.clear();
        (link, wire)
    }

    fn reply(wire: &Arc<Mutex<Wire>>, line: &str) {
        wire.lock().unwrap().replies.push_back(line.into());
    }

    /// The complete lines written so far, without the real-time bytes (status queries, resets) mixed in.
    fn sent(wire: &Arc<Mutex<Wire>>) -> Vec<String> {
        let w = wire.lock().unwrap();
        let text: String = String::from_utf8_lossy(&w.written).chars().filter(|&c| c != '?' && c != '\u{18}').collect();
        text.split('\n').filter(|l| !l.is_empty()).map(String::from).collect()
    }

    fn settle(link: &Link) {
        for _ in 0..30 {
            link.tick();
        }
    }

    fn set_limit(link: &Link, limit: Duration) {
        let inline = link.inline.as_ref().unwrap();
        inline.lock().unwrap().worker.fire_limit = limit;
    }
}
