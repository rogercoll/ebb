use std::time::Duration;

/// Time at least one task was stalled on the resource, from a PSI file (`memory.pressure`,
/// `io.pressure`).
// some avg10=0.00 avg60=0.00 avg300=0.00 total=106340
// full avg10=0.00 avg60=0.00 avg300=0.00 total=0
pub(crate) fn some_total(data: &str) -> Option<Duration> {
    data.lines()
        .find_map(|line| line.strip_prefix("some "))
        .and_then(|line| line.rsplit_once("total="))
        .and_then(|(_, us)| us.trim().parse().ok())
        .map(Duration::from_micros)
}
