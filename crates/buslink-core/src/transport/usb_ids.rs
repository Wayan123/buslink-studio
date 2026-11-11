//! USB-serial adapter identification by VID:PID only (PRD R1, tech.md).
//! Friendly names are never used: Windows and Android report them differently
//! and clones copy them.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UsbChip {
    /// Short chip name shown in the UI, e.g. "CH343".
    pub chip: &'static str,
    pub vendor: &'static str,
    /// The driver class the chip uses (CDC-ACM chips need no vendor driver on Windows 10+).
    pub driver: &'static str,
}

pub const WCH_VID: u16 = 0x1A86;

const KNOWN: [(u16, u16, UsbChip); 4] = [
    (
        WCH_VID,
        0x55D3,
        UsbChip {
            chip: "CH343",
            vendor: "WCH",
            driver: "CDC-ACM",
        },
    ),
    (
        WCH_VID,
        0x7523,
        UsbChip {
            chip: "CH340",
            vendor: "WCH",
            driver: "CH34x",
        },
    ),
    (
        WCH_VID,
        0x5523,
        UsbChip {
            chip: "CH341",
            vendor: "WCH",
            driver: "CH34x",
        },
    ),
    (
        WCH_VID,
        0x55D4,
        UsbChip {
            chip: "CH9102",
            vendor: "WCH",
            driver: "CDC-ACM",
        },
    ),
];

/// The chip for a VID:PID pair, if it is one BusLink knows.
pub fn classify(vid: u16, pid: u16) -> Option<UsbChip> {
    KNOWN
        .iter()
        .find(|(v, p, _)| *v == vid && *p == pid)
        .map(|(_, _, c)| *c)
}

/// `1A86:55D3` style label.
pub fn vid_pid_label(vid: u16, pid: u16) -> String {
    format!("{vid:04X}:{pid:04X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wch_ids() {
        assert_eq!(classify(0x1A86, 0x55D3).map(|c| c.chip), Some("CH343"));
        assert_eq!(classify(0x1A86, 0x7523).map(|c| c.chip), Some("CH340"));
        assert_eq!(classify(0x1A86, 0x55D4).map(|c| c.chip), Some("CH9102"));
        assert_eq!(classify(0x1A86, 0x55D3).map(|c| c.driver), Some("CDC-ACM"));
    }

    #[test]
    fn unknown_ids_are_unlabeled() {
        assert_eq!(classify(0x0403, 0x6001), None, "FTDI is not claimed as WCH");
        assert_eq!(classify(0x1A86, 0x0000), None);
        assert_eq!(classify(0x55D3, 0x1A86), None, "VID and PID are not interchangeable");
        assert_eq!(vid_pid_label(0x1A86, 0x55D3), "1A86:55D3");
    }
}
