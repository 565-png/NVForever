# Host-local time with Jiff

[documented: NVForever `nvse_data.rs`, `nvse_lutana.rs`, `nvse_sup5.rs` before this
change] The four existing host-time APIs expose local calendar fields, not game
time. GetUserTime returns Year (negative years become zero), Month, Sunday-based
DayOfWeek, Day, Hour, Minute, Second and truncated Millisecond. GetTimeStamp
returns `HH-MM-SS`. GetFileTime returns year through second; GetFileTimeSTR uses
`DD/MM/YY HH:MM`, or `MM/DD/YY HH:MM` when the integer-cast format selector is one.
The integer-cast file selector chooses creation (0), access (1) or modification
(2). Rooted file selection and empty results for unavailable metadata stay with
the existing file consumer. These are repository contracts, not new original-game
observations.

[documented: Jiff 1.0, https://docs.rs/jiff/1.0.0/jiff/ and its
`_documentation/platform` module] Jiff owns calendar conversion, system-zone
discovery, timezone caching and timezone data. The private adapter only converts
SystemTime to a Jiff timestamp and local civil datetime. Default Cargo features
include the timezone database on Windows; Windows zone names are mapped to IANA
names and IANA rules replace direct Windows yearly-rule evaluation. Unix uses
Jiff's installed-zoneinfo selection; environment overrides, cache refresh and
fallback behavior follow Jiff rather than preserving Chrono's implementation.
There is no NVForever timezone parser, transition evaluator or additional cache.

[documented: https://docs.rs/jiff/1.0.0/jiff/civil/struct.Date.html#method.new and
https://docs.rs/jiff/1.0.0/jiff/struct.Timestamp.html] Civil years are limited to
-9999 through 9999; timestamps have slightly narrower endpoints to allow all
valid timezone offsets. This intentionally narrows Chrono's former range. The
conversion boundary continues to panic on an unrepresentable timestamp. System
timezone detection uses `TimeZone::try_system`: failure also panics instead of
silently presenting UTC as local time. In particular, the old Unix final UTC
fallback is no longer promised. This migration is a policy change, not exact
Chrono compatibility. A missing file or unsupported metadata clock still returns
the existing empty result, before time conversion.

[documented: Jiff's `TimeZone::try_system` API] Targets without supported
system-zone discovery also fail at this boundary instead of retaining the former
zero-offset backend. Jiff's default unknown-zone/UTC fallback is not used.

[hypothesis] The existing field and format contracts remain sufficient for mods
using contemporary host dates. Original-game parity, historical Windows rule
equivalence, engine execution and installed-mod acceptance require separate
qualification. Tests exercise fixed instants, pre-epoch fractions, DST boundaries,
Jiff range/failure boundaries and compiled source/file consumers. Validation
results are recorded in the pull request; authored tests alone are not passes.

[documented: Windows CPU checks for this implementation, Rust/Cargo 1.97.1]
The release library binary passed 379 tests with one existing ignored test,
including all five adapter controls. All six SUP5 tests passed; the added consumer
test executed in separate New York and Kathmandu processes with scoped TZ
overrides. Focused library/SUP5 Clippy with `--no-deps -D warnings`, both workspace
dependency-policy checks and all 12 release-tool tests passed. Broader Clippy
stopped in unchanged `fnv-script/src/vm.rs:78` on `manual_range_contains`.
Full workspace/engine validation and original-game/mod acceptance remain pending.
