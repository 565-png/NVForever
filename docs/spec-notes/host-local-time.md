# Host-local time with Jiff

[documented: NVForever `local_time.rs` and `nvse_sup5.rs`; Jiff 1.0
[`TimeZone`](https://docs.rs/jiff/1.0.0/jiff/tz/struct.TimeZone.html) and
[`Timestamp`](https://docs.rs/jiff/1.0.0/jiff/struct.Timestamp.html)]
GetUserTime, GetTimeStamp, GetFileTime and GetFileTimeSTR use Jiff's system
timezone. If discovery fails, they use UTC and emit one warning per process.

An actual file timestamp outside Jiff's representable range is treated as
unavailable metadata: GetFileTime returns an empty list and GetFileTimeSTR
returns an empty string. Timestamp conversion happens before timezone lookup.

[hypothesis] The Windows-to-IANA mapping can differ from Windows historical or
per-machine rules, including when automatic daylight saving adjustment is off.
Original-game and installed-mod parity are not established.
