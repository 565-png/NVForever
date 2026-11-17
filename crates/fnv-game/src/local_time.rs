//! Host-local time for the four script APIs; policy: host-local-time.md.
use jiff::{civil::DateTime, tz::TimeZone, Timestamp};
use std::time::SystemTime;

pub(crate) fn now() -> DateTime {
    at(SystemTime::now())
}

pub(crate) fn at(instant: SystemTime) -> DateTime {
    // Keep conversion failure distinct from missing file metadata. Jiff's
    // unknown-zone fallback must not silently turn local time into UTC.
    in_zone(instant, TimeZone::try_system()).expect("host local-time conversion failed")
}

fn in_zone(
    instant: SystemTime,
    zone: Result<TimeZone, jiff::Error>,
) -> Result<DateTime, jiff::Error> {
    let timestamp = Timestamp::try_from(instant)?;
    Ok(zone?.to_datetime(timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn instant(text: &str) -> SystemTime {
        SystemTime::try_from(text.parse::<Timestamp>().unwrap()).unwrap()
    }

    #[test]
    fn fractions_before_epoch_and_sunday_numbering() {
        // One FILETIME tick is also exactly representable by Unix SystemTime.
        let time = in_zone(UNIX_EPOCH - Duration::from_nanos(100), Ok(TimeZone::UTC)).unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (1969, 12, 31));
        assert_eq!(
            (
                time.hour(),
                time.minute(),
                time.second(),
                time.millisecond()
            ),
            (23, 59, 59, 999)
        );
        let sunday = in_zone(instant("2024-03-10T12:00:00Z"), Ok(TimeZone::UTC)).unwrap();
        assert_eq!(sunday.weekday().to_sunday_zero_offset(), 0);
    }

    #[test]
    fn dst_gap_and_fold_use_the_offset_at_each_instant() {
        let zone = TimeZone::posix("EST5EDT,M3.2.0/2,M11.1.0/2").unwrap();
        for (utc, expected) in [
            ("2024-03-10T06:59:59.999Z", (1, 59, 59, 999)),
            ("2024-03-10T07:00:00Z", (3, 0, 0, 0)),
            ("2024-11-03T05:59:59Z", (1, 59, 59, 0)),
            ("2024-11-03T06:00:00Z", (1, 0, 0, 0)),
        ] {
            let local = in_zone(instant(utc), Ok(zone.clone())).unwrap();
            assert_eq!(
                (
                    local.hour(),
                    local.minute(),
                    local.second(),
                    local.millisecond()
                ),
                expected,
                "{utc}"
            );
        }
    }

    #[test]
    fn jiff_range_and_lookup_failures_are_not_successful_dates() {
        // Windows SystemTime cannot represent Jiff's BCE endpoint. Exercise
        // the upper endpoint and its refusal through the actual adapter.
        let value = SystemTime::try_from(Timestamp::MAX).unwrap();
        assert!(in_zone(value, Ok(TimeZone::UTC)).is_ok());
        let outside = SystemTime::try_from(Timestamp::MAX).unwrap() + Duration::from_secs(1);
        assert!(in_zone(outside, Ok(TimeZone::UTC)).is_err());
        assert!(in_zone(UNIX_EPOCH, TimeZone::posix("invalid")).is_err());
    }

    #[test]
    fn real_system_zone_converts_the_epoch() {
        let time = at(UNIX_EPOCH);
        assert!((1969..=1970).contains(&time.year()));
        assert!((0..=999).contains(&now().millisecond()));
    }

    #[test]
    #[should_panic(expected = "host local-time conversion failed")]
    fn public_conversion_refuses_out_of_range_time() {
        let outside = SystemTime::try_from(Timestamp::MAX).unwrap() + Duration::from_secs(1);
        at(outside);
    }
}
