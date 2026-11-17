//! Host wall-clock fields for the four existing time APIs (dependencies.md).
//! Calendar/TZif parsing belongs to tz-rs; platform lookup retains NVForever's
//! prior offset, range and failure policies. Native qualification is separate.
//! Platform-policy adaptation: Chrono 0.4.45, MIT OR Apache-2.0;
//! copyright Kang Seonghoon and contributors; Copyright 2012-2014 The Rust Project
//! Developers (local_time_licenses/Chrono-LICENSE.txt).
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

type ClockResult<T> = Result<T, &'static str>;

impl LocalTime {
    pub fn user_fields(self) -> [(&'static str, u32); 8] {
        [("Year", self.year.max(0) as u32), ("Month", self.month),
         ("DayOfWeek", self.weekday), ("Day", self.day), ("Hour", self.hour),
         ("Minute", self.minute), ("Second", self.second), ("Millisecond", self.millisecond)]
    }
    pub fn file_fields(self) -> [f64; 6] {
        [f64::from(self.year), f64::from(self.month), f64::from(self.day),
         f64::from(self.hour), f64::from(self.minute), f64::from(self.second)]
    }
    pub fn stamp(self) -> String {
        format!("{:02}-{:02}-{:02}", self.hour, self.minute, self.second)
    }
    pub fn file_stamp(self, month_first: bool) -> String {
        let (a, b) = if month_first { (self.month, self.day) } else { (self.day, self.month) };
        format!("{a:02}/{b:02}/{:02} {:02}:{:02}", self.year.rem_euclid(100), self.hour, self.minute)
    }
}

pub(crate) fn now() -> LocalTime { at(SystemTime::now()) }

// The former DateTime<Local>::from and Local::now conversions were infallible
// interfaces that panic on conversion failure. Keep that boundary; a failed
// offset lookup must not invent a UTC date or a missing-file result.
pub(crate) fn at(instant: SystemTime) -> LocalTime {
    convert(instant, platform_offset).expect("host local-time conversion failed")
}

fn timespec(instant: SystemTime) -> ClockResult<(i64, u32)> {
    let nanos = match instant.duration_since(UNIX_EPOCH) {
        Ok(d) => i128::from(d.as_secs()) * 1_000_000_000 + i128::from(d.subsec_nanos()),
        Err(e) => -(i128::from(e.duration().as_secs()) * 1_000_000_000 + i128::from(e.duration().subsec_nanos())),
    };
    Ok((nanos.div_euclid(1_000_000_000).try_into().map_err(|_| "timestamp range")?, nanos.rem_euclid(1_000_000_000) as u32))
}

fn convert(instant: SystemTime, offset: impl FnOnce(i64, u32) -> ClockResult<i32>) -> ClockResult<LocalTime> {
    let (seconds, nanos) = timespec(instant)?;
    convert_spec(seconds, nanos, offset)
}

fn convert_spec(seconds: i64, nanos: u32, offset: impl FnOnce(i64, u32) -> ClockResult<i32>) -> ClockResult<LocalTime> {
    let utc = tz::UtcDateTime::from_timespec(seconds, nanos).map_err(|_| "timestamp range")?;
    // Preserve the previous Chrono calendar admission, including dates far past
    // year9999. These are library limits, not an original-game observation.
    if !(-262_143..=262_142).contains(&utc.year()) { return Err("timestamp range"); }
    let offset = offset(seconds, nanos)?;
    if !(-86_399..=86_399).contains(&offset) { return Err("offset range"); }
    let local_type = tz::LocalTimeType::with_ut_offset(offset).map_err(|_| "offset range")?;
    let local = tz::DateTime::from_timespec_and_local(seconds, nanos, local_type).map_err(|_| "local timestamp range")?;
    if !(-262_143..=262_142).contains(&local.year()) { return Err("local timestamp range"); }
    Ok(LocalTime { year: local.year(), month: u32::from(local.month()), day: u32::from(local.month_day()),
        weekday: u32::from(local.week_day()), hour: u32::from(local.hour()), minute: u32::from(local.minute()),
        second: u32::from(local.second()), millisecond: local.nanoseconds() / 1_000_000 })
}

#[cfg(windows)]
fn platform_offset(seconds: i64, nanos: u32) -> ClockResult<i32> { windows::offset(seconds, nanos, windows::native_rules) }
#[cfg(unix)]
fn platform_offset(seconds: i64, _: u32) -> ClockResult<i32> { unix::offset(seconds) }
// The prior dependency also used zero offset on these targets when its
// optional wasmbind backend was disabled, as it was in this project.
#[cfg(not(any(windows, unix)))]
fn platform_offset(_: i64, _: u32) -> ClockResult<i32> { Ok(0) }

#[cfg(any(windows, test))]
mod windows {
    use super::*;

    #[derive(Clone, Copy, Debug, Default)]
    pub(super) struct Change {
        pub year: u16, pub month: u16, pub day: u16, pub weekday: u16,
        pub hour: u16, pub minute: u16, pub second: u16, pub millisecond: u16,
    }
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Rules {
        pub bias: i32, pub standard_bias: i32, pub daylight_bias: i32,
        pub standard: Change, pub daylight: Change,
    }

    // Windows' documented nth-weekday/absolute dates; the selected UTC year's
    // rules and endpoint extrapolation retain the former platform contract.
    fn transition(c: Change, year: i32) -> ClockResult<Option<i128>> {
        if c.year == 0 && c.month == 0 { return Ok(None); }
        let month: u8 = c.month.try_into().map_err(|_| "transition month")?;
        let hour = c.hour.try_into().map_err(|_| "transition hour")?;
        let minute = c.minute.try_into().map_err(|_| "transition minute")?;
        let second = c.second.try_into().map_err(|_| "transition second")?;
        if c.millisecond > 999 { return Err("transition millisecond"); }
        let nanos = u32::from(c.millisecond) * 1_000_000;
        let date_year = if c.year == 0 { year } else { i32::from(c.year) };
        let day = if c.year != 0 { c.day.try_into().map_err(|_| "transition day")? } else {
            if !(1..=5).contains(&c.day) || c.weekday > 6 { return Err("transition rule"); }
            let first = tz::UtcDateTime::new(year, month, 1, 0, 0, 0, 0).map_err(|_| "transition month")?;
            let day = 1 + (i32::from(c.weekday) - i32::from(first.week_day())).rem_euclid(7) as u8 + 7 * (c.day as u8 - 1);
            if c.day == 5 && tz::UtcDateTime::new(year, month, day, 0, 0, 0, 0).is_err() { day - 7 } else { day }
        };
        let date = tz::UtcDateTime::new(date_year, month, day, hour, minute, second, nanos).map_err(|_| "transition date")?;
        Ok(Some(i128::from(date.unix_time()) * 1_000_000_000 + i128::from(nanos)))
    }

    pub(super) fn offset(seconds: i64, nanos: u32, lookup: impl FnOnce(u16) -> ClockResult<Rules>) -> ClockResult<i32> {
        let utc = tz::UtcDateTime::from_timespec(seconds, 0).map_err(|_| "timestamp range")?;
        let year = utc.year();
        let r = lookup(year.clamp(1601, 30827) as u16)?;
        let bias = |extra: i32| r.bias.checked_add(extra).and_then(|v| v.checked_mul(-60)).filter(|v| (-86_399..=86_399).contains(v)).ok_or("offset range");
        let standard = bias(r.standard_bias)?;
        let daylight = bias(r.daylight_bias)?;
        let end = transition(r.standard, year)?.map(|v| v - i128::from(daylight) * 1_000_000_000);
        let start = transition(r.daylight, year)?.map(|v| v - i128::from(standard) * 1_000_000_000);
        let instant = i128::from(seconds) * 1_000_000_000 + i128::from(nanos);
        let is_daylight = match (start, end) {
            (Some(a), Some(b)) if a < b => instant >= a && instant < b,
            (Some(a), Some(b)) => instant >= a || instant < b,
            (Some(a), None) => instant >= a,
            (None, Some(b)) => instant < b,
            (None, None) => false,
        };
        Ok(if is_daylight { daylight } else { standard })
    }

    #[cfg(windows)]
    pub(super) fn native_rules(year: u16) -> ClockResult<Rules> {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::Time::{GetTimeZoneInformationForYear, TIME_ZONE_INFORMATION};
        let mut raw = std::mem::MaybeUninit::<TIME_ZONE_INFORMATION>::uninit();
        // SAFETY: an owned correctly sized output; null selects the current zone.
        // The output is read only after the documented nonzero success result.
        let raw = unsafe {
            if GetTimeZoneInformationForYear(year, std::ptr::null(), raw.as_mut_ptr()) == 0 { return Err("Windows timezone lookup"); }
            raw.assume_init()
        };
        let change = |s: SYSTEMTIME| Change { year: s.wYear, month: s.wMonth, day: s.wDay, weekday: s.wDayOfWeek,
            hour: s.wHour, minute: s.wMinute, second: s.wSecond, millisecond: s.wMilliseconds };
        Ok(Rules { bias: raw.Bias, standard_bias: raw.StandardBias, daylight_bias: raw.DaylightBias,
            standard: change(raw.StandardDate), daylight: change(raw.DaylightDate) })
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{cell::RefCell, fs};
    #[derive(PartialEq, Eq)]
    enum Source { Environment(String), File(Option<SystemTime>) }
    struct Cached { checked: SystemTime, source: Source, zone: tz::TimeZone }
    thread_local! { static ZONE: RefCell<Option<Cached>> = const { RefCell::new(None) }; }

    fn source(value: &Option<String>) -> Source {
        match value {
            Some(value) => Source::Environment(value.clone()),
            None => Source::File(fs::symlink_metadata("/etc/localtime").and_then(|m| m.modified()).ok()),
        }
    }
    fn parse_zone(name: &str, mut read: impl FnMut(&str) -> Option<Result<Vec<u8>, ()>>) -> Option<tz::TimeZone> {
        if name == "localtime" { return tz::TimeZone::from_tz_data(&read("/etc/localtime")?.ok()?).ok(); }
        let path = name.strip_prefix(':').unwrap_or(name);
        if std::path::Path::new(path).is_absolute() {
            if let Some(bytes) = read(path) { return tz::TimeZone::from_tz_data(&bytes.ok()?).ok(); }
        } else {
            for directory in ["/usr/share/zoneinfo", "/share/zoneinfo", "/etc/zoneinfo", "/usr/share/lib/zoneinfo"] {
                if let Some(bytes) = read(&format!("{directory}/{path}")) {
                    // First opened file owns selection, including a read/parse failure.
                    return tz::TimeZone::from_tz_data(&bytes.ok()?).ok();
                }
            }
        }
        if name.starts_with(':') { return None; }
        // The maintained library owns POSIX grammar. File selection above owns
        // precedence, so this parser invocation has no filesystem access.
        tz::TimeZoneSettings::new(&[], |_| Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())).parse_posix_tz(name).ok()
    }
    fn read_opened_file(path: &str) -> Option<Result<Vec<u8>, ()>> {
        use std::io::Read;
        let mut file = fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        Some(file.read_to_end(&mut bytes).map(|_| bytes).map_err(|_| ()))
    }
    fn fallback() -> Option<tz::TimeZone> {
        let name = iana_time_zone::get_timezone().ok()?;
        #[cfg(any(target_os = "android", target_env = "ohos"))]
        let bytes = android::system_zone(&name)?;
        #[cfg(not(any(target_os = "android", target_env = "ohos")))]
        let bytes = {
            #[cfg(target_os = "aix")]
            let root = "/usr/share/lib/zoneinfo";
            #[cfg(not(target_os = "aix"))]
            let root = "/usr/share/zoneinfo";
            fs::read(format!("{root}/{name}")).ok()?
        };
        tz::TimeZone::from_tz_data(&bytes).ok()
    }
    fn load(value: Option<&str>) -> tz::TimeZone {
        choose(value, |name| {
            #[cfg(any(target_os = "android", target_env = "ohos"))]
            { android::system_zone(name) }
            #[cfg(not(any(target_os = "android", target_env = "ohos")))]
            { let _ = name; None }
        }, |v| parse_zone(v, read_opened_file), fallback)
    }
    fn admit(zone: tz::TimeZone) -> Option<tz::TimeZone> {
        let designation = |kind: &tz::LocalTimeType| !matches!(kind.time_zone_designation().len(), 1 | 2);
        let view = zone.as_ref();
        let ordinary = view.local_time_types().iter().all(designation);
        let footer = match view.extra_rule() {
            Some(tz::timezone::TransitionRule::Fixed(kind)) => designation(kind),
            Some(tz::timezone::TransitionRule::Alternate(rule)) => designation(rule.std()) && designation(rule.dst()),
            None => true,
        };
        // Keep the previous 3..=7-byte designation admission; tz-rs also accepts
        // one-letter military names, which would change the existing fallback.
        (ordinary && footer).then_some(zone)
    }
    pub(super) fn choose(value: Option<&str>, record: impl FnOnce(&str) -> Option<Vec<u8>>,
        parse: impl FnOnce(&str) -> Option<tz::TimeZone>, fallback: impl FnOnce() -> Option<tz::TimeZone>) -> tz::TimeZone {
        // The explicit Android record precedes ordinary file/POSIX parsing. An
        // existing malformed record fails selection, as before; it is not skipped.
        if value == Some("") { return tz::TimeZone::utc(); }
        let name = value.unwrap_or("localtime");
        let selected = if name != "localtime" {
            match record(name) { Some(bytes) => tz::TimeZone::from_tz_data(&bytes).ok(), None => parse(name) }
        } else { parse(name) };
        selected.and_then(admit).or_else(|| fallback().and_then(admit)).unwrap_or_else(tz::TimeZone::utc)
    }
    fn zone_offset(zone: &tz::TimeZone, seconds: i64) -> ClockResult<i32> {
        match zone.find_local_time_type(seconds) {
            Ok(kind) => Ok(kind.ut_offset()),
            Err(tz::TzError::NoAvailableLocalTimeType) if zone.as_ref().extra_rule().is_none() => {
                // Retain the previous last-type policy for footerless TZif files.
                let view = zone.as_ref();
                let last = view.transitions().last().ok_or("timezone lookup")?;
                view.local_time_types().get(last.local_time_type_index()).map(|t| t.ut_offset()).ok_or("timezone lookup")
            }
            Err(_) => Err("timezone lookup"),
        }
    }
    fn refresh(slot: &mut Option<Cached>, now: SystemTime,
        source: impl FnOnce() -> (Source, Option<String>),
        load: impl FnOnce(Option<&str>) -> tz::TimeZone) {
        if slot.as_ref().is_none_or(|c| now.duration_since(c.checked).map_or(true, |d| d.as_secs() >= 1)) {
            let (current, value) = source();
            let reload = slot.as_ref().is_none_or(|c| c.source != current || matches!(current, Source::File(None)));
            if reload { *slot = Some(Cached { checked: now, source: current, zone: load(value.as_deref()) }); }
            else if let Some(c) = slot.as_mut() { c.checked = now; }
        }
    }
    pub(super) fn offset(seconds: i64) -> ClockResult<i32> {
        ZONE.with(|slot| {
            let mut slot = slot.borrow_mut();
            refresh(&mut slot, SystemTime::now(), || {
                let value = std::env::var("TZ").ok();
                (source(&value), value)
            }, load);
            zone_offset(&slot.as_ref().ok_or("timezone cache")?.zone, seconds)
        })
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{cell::Cell, time::Duration};
        fn footerless_tzif() -> Vec<u8> {
            let mut bytes = b"TZif\0".to_vec();
            bytes.extend([0; 15]);
            for count in [0u32, 0, 0, 1, 2, 8] { bytes.extend(count.to_be_bytes()); }
            bytes.extend(100i32.to_be_bytes()); bytes.push(1);
            for (offset, name) in [(0i32, 0u8), (3600, 4)] { bytes.extend(offset.to_be_bytes()); bytes.extend([0, name]); }
            bytes.extend(b"OLD\0NEW\0"); bytes
        }
        thread_local! { static DIRECTORY_READS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
        fn secondary_file(path: &str) -> Option<Result<Vec<u8>, ()>> {
            DIRECTORY_READS.with(|r| r.borrow_mut().push(path.to_owned()));
            if path == "/usr/share/lib/zoneinfo/Owned/Secondary" { Some(Ok(footerless_tzif())) } else { None }
        }
        #[test]
        fn actual_zone_selection_searches_every_prior_directory_and_retains_footerless_last_type() {
            DIRECTORY_READS.with(|r| r.borrow_mut().clear());
            let zone = choose(Some("Owned/Secondary"), |_| None,
                |name| parse_zone(name, secondary_file), || panic!("the existing secondary directory must be selected"));
            DIRECTORY_READS.with(|r| assert_eq!(*r.borrow(), ["/usr/share/zoneinfo/Owned/Secondary", "/share/zoneinfo/Owned/Secondary", "/etc/zoneinfo/Owned/Secondary", "/usr/share/lib/zoneinfo/Owned/Secondary"]));
            assert_eq!(zone_offset(&zone, 99), Ok(0));
            assert_eq!(zone_offset(&zone, 100), Ok(3600));
            assert_eq!(zone_offset(&zone, 101), Ok(3600));
            let far = tz::UtcDateTime::new(35000, 1, 1, 0, 0, 0, 0).unwrap().unix_time();
            assert_eq!(zone_offset(&zone, far), Ok(3600));
        }
        #[test]
        fn first_opened_zone_file_failure_does_not_select_a_later_file_or_posix_rule() {
            for failed in [Err(()), Ok(b"bad TZif".to_vec())] {
                let selected = choose(Some("UTC0"), |_| None, |name| {
                    parse_zone(name, |path| {
                        assert_eq!(path, "/usr/share/zoneinfo/UTC0", "later files must not be tried after an opened failure");
                        Some(failed.clone())
                    })
                }, || Some(tz::TimeZone::fixed(20700).unwrap()));
                assert_eq!(zone_offset(&selected, 0), Ok(20700));
            }
        }
        #[test]
        fn selected_android_override_record_precedes_parser_and_os_fallback() {
            use std::io::Cursor;
            let payload = footerless_tzif();
            let mut data = b"tzdata2026a\0".to_vec();
            for n in [24u32, 76, 76 + payload.len() as u32] { data.extend(n.to_be_bytes()); }
            let mut entry = [0; 52]; entry[..12].copy_from_slice(b"Owned/Named\0");
            entry[44..48].copy_from_slice(&(payload.len() as u32).to_be_bytes()); data.extend(entry); data.extend(payload);
            let selected = choose(Some("Owned/Named"), |name| android::record(&mut Cursor::new(data), name.as_bytes(), 52),
                |_| panic!("selected container record precedes ordinary parsing"), || panic!("explicit override precedes OS identifier"));
            assert_eq!(zone_offset(&selected, 101), Ok(3600));
            let bad = choose(Some("Owned/Bad"), |_| Some(b"not TZif".to_vec()),
                |_| panic!("an existing malformed record fails selection"), || Some(tz::TimeZone::fixed(20700).unwrap()));
            assert_eq!(zone_offset(&bad, 0), Ok(20700));
        }
        #[test]
        fn designation_admission_retains_short_name_fallback_and_valid_posix_rules() {
            let parse = |name: &str| parse_zone(name, |_| None);
            for name in ["X0", "XY0", "UTC0X,M3.2.0,M11.1.0"] {
                let selected = choose(Some(name), |_| None, parse, || Some(tz::TimeZone::fixed(20700).unwrap()));
                assert_eq!(zone_offset(&selected, 0), Ok(20700), "{name}");
            }
            let valid = choose(Some("UTC0"), |_| None, parse, || panic!("valid three-letter name must parse"));
            assert_eq!(zone_offset(&valid, 0), Ok(0));
        }
        #[test]
        fn real_cache_consumer_retains_zone_until_recheck_and_reloads_changed_or_missing_sources() {
            let loads = Cell::new(0);
            let mut slot = None;
            let loader = |_: Option<&str>| { loads.set(loads.get() + 1); tz::TimeZone::fixed(20700).unwrap() };
            let environment = || (Source::Environment("owned".into()), Some("owned".into()));
            refresh(&mut slot, UNIX_EPOCH, environment, loader);
            assert_eq!(loads.get(), 1);
            assert_eq!(slot.as_ref().unwrap().zone.find_local_time_type(0).unwrap().ut_offset(), 20700);
            refresh(&mut slot, UNIX_EPOCH + Duration::from_nanos(999_999_999), || panic!("cache must not inspect process state early"), loader);
            refresh(&mut slot, UNIX_EPOCH + Duration::from_secs(1), environment, loader);
            assert_eq!(loads.get(), 1, "unchanged environment retains parsed zone");
            refresh(&mut slot, UNIX_EPOCH + Duration::from_secs(2), || (Source::File(Some(UNIX_EPOCH)), None), loader);
            assert_eq!(loads.get(), 2, "environment to file changes ownership");
            refresh(&mut slot, UNIX_EPOCH, || (Source::File(None), None), loader);
            assert_eq!(loads.get(), 3, "backwards clock rechecks and missing metadata invalidates");
            refresh(&mut slot, UNIX_EPOCH + Duration::from_secs(1), || (Source::File(None), None), loader);
            assert_eq!(loads.get(), 4, "missing metadata must not pin a stale zone");
        }
    }
}

// The Android system tzdata container is a documented OS data format, not a
// game/extender layout. Keep its existing environment-root order and read only
// the selected record; no timezone database is bundled into NVForever.
#[cfg(any(target_os = "android", target_env = "ohos", test))]
mod android {
    use std::io::{Read, Seek, SeekFrom};

    pub(super) fn record(reader: &mut (impl Read + Seek), name: &[u8], entry_len: usize) -> Option<Vec<u8>> {
        let mut header = [0; 24]; reader.read_exact(&mut header).ok()?;
        if &header[..6] != b"tzdata" || header[11] != 0 || !matches!(entry_len, 48 | 52) { return None; }
        let number = |b: &[u8]| b.try_into().ok().map(u32::from_be_bytes);
        let index = number(&header[12..16])?;
        let data = number(&header[16..20])?;
        let end = number(&header[20..24])?;
        let file_len = reader.seek(SeekFrom::End(0)).ok()?;
        if index < 24 || data < index || end < data || u64::from(end) > file_len || !((data - index) as usize).is_multiple_of(entry_len) { return None; }
        reader.seek(SeekFrom::Start(u64::from(index))).ok()?;
        // Only one fixed-size directory entry is retained while scanning.
        let mut entry = vec![0; entry_len];
        for _ in 0..(data - index) as usize / entry_len {
            reader.read_exact(&mut entry).ok()?;
            let Some(nul) = entry[..40].iter().position(|b| *b == 0) else { continue };
            if &entry[..nul] != name { continue; }
            let relative = number(&entry[40..44])?; let length = number(&entry[44..48])?;
            let start = u64::from(data).checked_add(u64::from(relative))?;
            let stop = start.checked_add(u64::from(length))?;
            if stop > u64::from(end) || stop > file_len { return None; }
            reader.seek(SeekFrom::Start(start)).ok()?;
            let mut bytes = vec![0; length.try_into().ok()?]; reader.read_exact(&mut bytes).ok()?;
            return Some(bytes);
        }
        None
    }
    #[cfg(any(target_os = "android", target_env = "ohos"))]
    pub(super) fn system_zone(name: &str) -> Option<Vec<u8>> {
        #[cfg(target_env = "ohos")]
        { return record(&mut std::fs::File::open("/system/etc/zoneinfo/tzdata").ok()?, name.as_bytes(), 48); }
        #[cfg(target_os = "android")]
        {
            for (var, suffix) in [("ANDROID_TZDATA_ROOT", "/etc/tz/tzdata"), ("ANDROID_DATA", "/misc/zoneinfo/current/tzdata"), ("ANDROID_ROOT", "/usr/share/zoneinfo/tzdata")] {
                if let Ok(root) = std::env::var(var) {
                    // Preserve first-open-file selection, even if its selected record is absent.
                    if let Ok(mut file) = std::fs::File::open(format!("{root}{suffix}")) { return record(&mut file, name.as_bytes(), 52); }
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn instant(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8, nanos: u32) -> SystemTime {
        let t = tz::UtcDateTime::new(year, month, day, hour, minute, second, nanos).unwrap();
        if t.unix_time() >= 0 { UNIX_EPOCH + Duration::new(t.unix_time() as u64, nanos) }
        else { UNIX_EPOCH - Duration::from_secs(t.unix_time().unsigned_abs()) + Duration::from_nanos(u64::from(nanos)) }
    }
    fn fixed_spec(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8, nanos: u32) -> LocalTime {
        let t = tz::UtcDateTime::new(year, month, day, hour, minute, second, nanos).unwrap();
        convert_spec(t.unix_time(), nanos, |_, _| Ok(0)).unwrap()
    }
    fn fixed(t: SystemTime, offset: i32) -> LocalTime { convert(t, |_, _| Ok(offset)).unwrap() }

    #[test]
    fn real_conversion_supplies_all_four_api_field_and_format_producers() {
        let t = fixed(instant(2026, 11, 15, 0, 7, 9, 999_999_999), 0);
        assert_eq!(t.user_fields(), [("Year", 2026), ("Month", 11), ("DayOfWeek", 0), ("Day", 15), ("Hour", 0), ("Minute", 7), ("Second", 9), ("Millisecond", 999)]);
        assert_eq!(t.file_fields(), [2026.0, 11.0, 15.0, 0.0, 7.0, 9.0]);
        assert_eq!(t.stamp(), "00-07-09");
        assert_eq!(t.file_stamp(false), "15/11/26 00:07");
        assert_eq!(t.file_stamp(true), "11/15/26 00:07");
    }
    #[test]
    fn negative_epoch_fraction_is_not_truncated_towards_zero() {
        assert_eq!(timespec(UNIX_EPOCH - Duration::from_nanos(1)), Ok((-1, 999_999_999)));
        let t = fixed(UNIX_EPOCH - Duration::from_nanos(1), 0);
        assert_eq!(t.file_fields(), [1969.0, 12.0, 31.0, 23.0, 59.0, 59.0]);
        assert_eq!(t.millisecond, 999);
    }
    #[test]
    fn wide_dates_and_bce_formatting_retain_existing_admission() {
        for year in [10000, 30827, 35000, 262142, -262143] {
            let t = fixed_spec(year, 6, 1, 12, 0, 0, 0);
            assert_eq!(t.year, year);
        }
        let bce = fixed_spec(-1, 1, 2, 3, 4, 5, 0);
        assert_eq!(bce.user_fields()[0], ("Year", 0));
        assert_eq!(bce.file_fields()[0], -1.0);
        assert_eq!(bce.file_stamp(false), "02/01/99 03:04");
        let mut called = false;
        let seconds = tz::UtcDateTime::new(262143, 1, 1, 0, 0, 0, 0).unwrap().unix_time();
        assert_eq!(convert_spec(seconds, 0, |_, _| { called = true; Ok(0) }), Err("timestamp range"));
        assert!(!called, "an inadmissible instant cannot reach platform lookup");
        let upper = tz::UtcDateTime::new(262142, 12, 31, 23, 59, 59, 0).unwrap().unix_time();
        assert_eq!(convert_spec(upper, 0, |_, _| Ok(1)), Err("local timestamp range"));
        let lower = tz::UtcDateTime::new(-262143, 1, 1, 0, 0, 0, 0).unwrap().unix_time();
        assert_eq!(convert_spec(lower, 0, |_, _| Ok(-1)), Err("local timestamp range"));
    }
    #[test]
    fn offset_failure_and_invalid_offset_never_publish_utc_success() {
        assert_eq!(convert(UNIX_EPOCH, |_, _| Err("injected native failure")), Err("injected native failure"));
        assert_eq!(convert(UNIX_EPOCH, |_, _| Ok(86_400)), Err("offset range"));
        let t = fixed(instant(2024, 2, 29, 20, 0, 0, 0), 5 * 3600 + 45 * 60);
        assert_eq!(t.file_fields(), [2024.0, 3.0, 1.0, 1.0, 45.0, 0.0]);
    }
    fn north() -> windows::Rules {
        windows::Rules { bias: 300, standard_bias: 0, daylight_bias: -60,
            daylight: windows::Change { month: 3, day: 2, weekday: 0, hour: 2, ..Default::default() },
            standard: windows::Change { month: 11, day: 1, weekday: 0, hour: 2, ..Default::default() } }
    }
    fn windows_at(t: SystemTime, r: windows::Rules) -> LocalTime { convert(t, |s, n| windows::offset(s, n, |_| Ok(r))).unwrap() }
    #[test]
    fn injected_windows_rule_consumer_uses_the_instant_for_dst_gap_and_fold() {
        let r = north();
        assert_eq!(windows_at(instant(2026, 3, 8, 6, 59, 59, 999_999_999), r).stamp(), "01-59-59");
        assert_eq!(windows_at(instant(2026, 3, 8, 7, 0, 0, 0), r).stamp(), "03-00-00");
        assert_eq!(windows_at(instant(2026, 11, 1, 5, 59, 59, 999_999_999), r).stamp(), "01-59-59");
        assert_eq!(windows_at(instant(2026, 11, 1, 6, 0, 0, 0), r).stamp(), "01-00-00");
    }
    #[test]
    fn injected_windows_lookup_preserves_wide_actual_year_and_historical_request() {
        for (year, requested) in [(1500, 1601), (2026, 2026), (35000, 30827)] {
            let mut seen = None;
            let seconds = tz::UtcDateTime::new(year, 6, 1, 12, 0, 0, 0).unwrap().unix_time();
            let t = convert_spec(seconds, 0, |s, n| windows::offset(s, n, |y| { seen = Some(y); Ok(north()) })).unwrap();
            assert_eq!(seen, Some(requested));
            assert_eq!(t.year, year, "the actual date is never clamped to the OS query year");
            assert_eq!(t.hour, 8);
        }
        assert_eq!(convert(UNIX_EPOCH, |s, n| windows::offset(s, n, |_| Err("native query failed"))), Err("native query failed"));
    }
    #[test]
    fn windows_southern_last_weekday_fixed_and_millisecond_rules_remain_distinct() {
        let mut r = windows::Rules { bias: -600, standard_bias: 0, daylight_bias: -60,
            daylight: windows::Change { month: 10, day: 1, weekday: 0, hour: 2, ..Default::default() },
            standard: windows::Change { month: 4, day: 1, weekday: 0, hour: 3, ..Default::default() } };
        assert_eq!(windows_at(instant(2026, 1, 1, 0, 0, 0, 0), r).hour, 11);
        assert_eq!(windows_at(instant(2026, 7, 1, 0, 0, 0, 0), r).hour, 10);
        r.bias = 0; r.daylight_bias = -60;
        r.daylight = windows::Change { month: 2, day: 5, weekday: 0, millisecond: 500, ..Default::default() };
        r.standard = windows::Change { month: 10, day: 5, weekday: 0, ..Default::default() };
        assert_eq!(windows_at(instant(2026, 2, 22, 0, 0, 0, 499_999_999), r).hour, 0);
        assert_eq!(windows_at(instant(2026, 2, 22, 0, 0, 0, 500_000_000), r).hour, 1);
        r.daylight = Default::default(); r.standard = Default::default();
        assert_eq!(windows_at(UNIX_EPOCH, r).hour, 0);
    }
    #[test]
    fn windows_absolute_and_single_transition_rules_and_invalid_data_keep_their_boundaries() {
        let mut r = north();
        r.daylight = windows::Change { year: 2026, month: 6, day: 2, hour: 0, ..Default::default() };
        r.standard = Default::default();
        assert_eq!(windows_at(instant(2026, 6, 2, 4, 59, 59, 0), r).hour, 23);
        assert_eq!(windows_at(instant(2026, 6, 2, 5, 0, 0, 0), r).hour, 1);
        r.daylight.month = 13;
        assert_eq!(convert(UNIX_EPOCH, |s, n| windows::offset(s, n, |_| Ok(r))), Err("transition date"));
        r.daylight = Default::default();
        r.bias = i32::MAX;
        assert_eq!(convert(UNIX_EPOCH, |s, n| windows::offset(s, n, |_| Ok(r))), Err("offset range"));
    }
    #[cfg(unix)]
    #[test]
    fn unix_selection_keeps_empty_tz_system_fallback_and_final_utc_separate() {
        let zone = unix::choose(Some(""), |_| panic!("empty override must not inspect a record"), |_| panic!("empty override must not read a file"), || panic!("empty override must not use system fallback"));
        assert_eq!(zone.find_local_time_type(0).unwrap().ut_offset(), 0);
        let zone = unix::choose(Some("invalid"), |_| None, |_| None, || Some(tz::TimeZone::fixed(20700).unwrap()));
        assert_eq!(zone.find_local_time_type(0).unwrap().ut_offset(), 20700);
        let zone = unix::choose(None, |_| panic!("localtime does not select an explicit record"), |name| { assert_eq!(name, "localtime"); None }, || None);
        assert_eq!(zone.find_local_time_type(0).unwrap().ut_offset(), 0);
    }
    fn tzdata(entry_len: usize) -> Vec<u8> {
        let start = 24 + entry_len;
        let mut b = b"tzdata2026a\0".to_vec();
        for n in [24u32, start as u32, start as u32 + 4] { b.extend_from_slice(&n.to_be_bytes()); }
        let mut entry = vec![0; entry_len]; entry[..9].copy_from_slice(b"Owned/TZ\0");
        entry[44..48].copy_from_slice(&4u32.to_be_bytes()); b.extend(entry); b.extend(b"TZif"); b
    }
    #[test]
    fn android_selected_record_consumer_checks_directory_and_data_boundaries() {
        use std::io::Cursor;
        for len in [48, 52] { assert_eq!(android::record(&mut Cursor::new(tzdata(len)), b"Owned/TZ", len), Some(b"TZif".to_vec())); }
        assert_eq!(android::record(&mut Cursor::new(tzdata(52)), b"Absent/TZ", 52), None);
        let mut bad = tzdata(52); bad[24 + 44..24 + 48].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(android::record(&mut Cursor::new(bad), b"Owned/TZ", 52), None);
        let mut bad = tzdata(52); bad[12..16].copy_from_slice(&100u32.to_be_bytes());
        assert_eq!(android::record(&mut Cursor::new(bad), b"Owned/TZ", 52), None);
        assert_eq!(android::record(&mut Cursor::new(tzdata(52)), b"Owned/TZ", 1_000_000), None);
    }
}
