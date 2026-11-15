use crate::{IoConfig, MemConfig};

/// Share of a period at least one task was stalled, per resource (0.001 = 0.1%).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pressure {
    pub(crate) memory: f64,
    pub(crate) io: f64,
}

/// Bytes to reclaim this period (TMO, §3.3):
/// `current × ratio × max(0, 1 − pressure / threshold)`.
pub(crate) fn tmo(mem: &MemConfig, pressure: Pressure, current: u64) -> u64 {
    let room = (1.0 - pressure.memory / mem.psi_threshold).max(0.0);
    (current as f64 * mem.reclaim_ratio * room) as u64
}

/// IO pressure is below its threshold. Refaults can slow the device without showing up as
/// memory stalls, so reclaim waits while IO is under pressure.
pub(crate) fn io_ok(io: &IoConfig, pressure: Pressure) -> bool {
    pressure.io < io.psi_threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmo_scales_with_pressure() {
        let mem = MemConfig {
            reclaim_ratio: 0.01,
            psi_threshold: 0.001,
            interval: 6,
        };
        let tmo = |memory: f64| tmo(&mem, Pressure { memory, io: 0.0 }, 1 << 30);
        // No pressure: the full ratio. Halfway to the threshold: half of it.
        assert_eq!(tmo(0.0), (1 << 30) / 100);
        assert_eq!(tmo(0.0005), (1 << 30) / 200);
        // At or above the threshold: nothing.
        assert_eq!(tmo(0.001), 0);
        assert_eq!(tmo(0.5), 0);
    }

    #[test]
    fn io_ok_below_threshold() {
        let io = IoConfig {
            psi_threshold: 0.001,
        };
        let io_ok = |io_pressure: f64| {
            io_ok(
                &io,
                Pressure {
                    memory: 0.0,
                    io: io_pressure,
                },
            )
        };
        assert!(io_ok(0.0009));
        assert!(!io_ok(0.001));
    }
}
