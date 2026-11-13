//! Which already-granted serial port a reconnect reuses, kept free of browser types so it can be tested natively.
//!
//! `navigator.serial.getPorts()` lists the ports the person already allowed; reusing one skips the picker.
//! See <https://developer.mozilla.org/en-US/docs/Web/API/Serial/getPorts>. The browser calls are in `lib.rs`.

/// What `SerialPort.getInfo()` reports; both are absent for ports that are not USB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortId {
    pub vendor: Option<u16>,
    pub product: Option<u16>,
}

/// The index of the granted port to reopen, or `None` when the picker must be shown.
///
/// The port used last time wins when exactly one granted port matches it. Otherwise a single granted port is
/// reused, since there is nothing to choose between. Several candidates are never guessed at: with two boards
/// attached, opening the wrong one would send them the wrong job.
pub fn choose(granted: &[PortId], last: Option<PortId>) -> Option<usize> {
    if let Some(last) = last {
        let mut matches = granted.iter().enumerate().filter(|(_, id)| **id == last).map(|(i, _)| i);
        if let (Some(first), None) = (matches.next(), matches.next()) {
            return Some(first);
        }
    }
    if granted.len() == 1 { Some(0) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_granted_port_shows_the_picker() {
        assert_eq!(choose(&[], None), None);
        assert_eq!(choose(&[], Some(id(1, 2))), None);
    }

    #[test]
    fn a_single_granted_port_is_reused() {
        assert_eq!(choose(&[id(1, 2)], None), Some(0));
        assert_eq!(choose(&[id(1, 2)], Some(id(9, 9))), Some(0));
    }

    #[test]
    fn the_port_used_last_wins_among_several() {
        assert_eq!(choose(&[id(1, 2), id(3, 4)], Some(id(3, 4))), Some(1));
    }

    #[test]
    fn several_ports_without_a_unique_match_show_the_picker() {
        assert_eq!(choose(&[id(1, 2), id(3, 4)], None), None);
        assert_eq!(choose(&[id(1, 2), id(3, 4)], Some(id(9, 9))), None);
        // Two identical boards cannot be told apart.
        assert_eq!(choose(&[id(1, 2), id(1, 2)], Some(id(1, 2))), None);
    }

    #[test]
    fn ports_without_usb_ids_are_still_matched() {
        let plain = PortId { vendor: None, product: None };
        assert_eq!(choose(&[plain], None), Some(0));
        assert_eq!(choose(&[plain, id(1, 2)], Some(plain)), Some(0));
    }

    fn id(vendor: u16, product: u16) -> PortId {
        PortId { vendor: Some(vendor), product: Some(product) }
    }
}
