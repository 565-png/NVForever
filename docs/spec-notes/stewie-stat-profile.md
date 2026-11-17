# Original Stewie stat-profile defaults

## Supported behavior and policy

[documented] The original profile names bCritChanceIgnoresFireRate in Tweaks and
bScaleCriticalDamage plus fCriticalDamageMult in Critical Hits. The existing
engine critical-hit rules already implement those choices. StewieGetValue/StewVal
reads a key:section setting (existing catalogue and
[behavior page](https://geckwiki.com/index.php?title=StewieGetValue)). [documented] The behavior page identifies StewVal as an alias and describes a
queried boolean as whether its tweak is enabled; it does not specify a disk-read
or reload contract.

[hypothesis/design] After application settings are handed to the actual combat
consumer, the recognized bCritChanceIgnoresFireRate and fCriticalDamageMult queries
read those same current Game settings. This intentional mapped-policy change keeps
script stat calculations consistent with effective NVForever choices. Unbound
headless games and unrelated keys retain the existing layered-file query. No
second setting-value cache is added. The native scale-enable switch is not inferred
from the engine multiplier: bScaleCriticalDamage remains a raw-profile query and
its enabled-intent/native equivalence is open. Consequently the complete original
ItemCards/PipBoy critical-stat initialization is not yet qualified for every
conflicting scale-switch/profile combination.

[hypothesis/design] An installed original profile now supplies defaults for these
two existing engine options when their NVForever setting keys are absent. This is
an intentional compatibility-profile policy change: without an original profile,
ordinary off/no-change defaults remain. Stored tweak-crit-ignores-fire-rate and
tweak-crit-damage-scale values retain the existing settings reader's precedence,
including explicit off/default-valued keys. Legacy serialized keys cannot be
classified as user intent from their values alone; their precedence is logged,
not silently overridden or guessed.

Only boolean0/1 inputs are imported. Scaling enabled by bScaleCriticalDamage uses
a finite multiplier within the existing engine slider's0..2 range; disabled scaling
selects1. Missing, malformed and unsupported inputs do not enable new behavior.
Other original keys are not imported or advertised as supported by this bridge.

## Startup, restoration and lifetime

The profile is reduced to a typed input by Game; the engine receives no new raw
ModFiles access. Runtime initialization applies the settings before new-game
initialization and before Restart files. After restoring a saved game, current
application settings are handed back before Restart files, preserving live
settings rather than serialized game defaults. Ordinary frame synchronization
continues through the existing Tweaks consumer.

[hypothesis/design] Import occurs once per application profile. Later NVForever
settings changes retain their normal effect. Runtime edits of the original INI
are not reimported over live settings by this startup-only subset. Recognized
loaded queries continue to report the real current consumer fields; unrelated
queries retain their existing file behavior. No new watcher, per-frame disk read, save field or
installation write is introduced. Native import/reload parity remains unmeasured.

The existing two critical-rule assignments are shared with the actual damage
consumer; formulas, ordinary condition penalties and unrelated tuning do not
change. This does not implement menu search, selected-row stat publication or any
other Stewie service.

## Validation and limits

Three ordinary core controls characterize supported and malformed typed inputs,
both aliases, current-field updates, unbound/unrepresented keys, save lifetime and
rebinding. Two opt-in engine controls use authored process-local Data/settings
roots, actual Runtime initialization and source files, critical-hit rules,
application settings and restoration. Their exact selectors require a dedicated
`NVF_STEWIE_PROFILE_HOME`, `--ignored` and `--test-threads=1`; ordinary CI skips
them, and a skipped result is not a pass.

[implementation evidence: head `68a9c978`, 2026-11-17] The
[natural hosted run](https://github.com/LootJunkieAzza/NVForever/actions/runs/44155882635)
used checkout `2a932d9b` with parents `[13011912,68a9c978]`. All seven checks
passed. Six owning non-ignored controls each passed once: these three core
profile controls, two inventory controls and the ordinary Pip-Boy control.
The three ignored profile controls received no credit from that run.

[implementation evidence: exact original `68a9c978`, Windows Cargo 1.99 QA]
The two critical-profile opt-ins and the inventory opt-in subsequently each
passed once in three separately admitted runtime processes. The critical case
checked initialization, source files, damage rules, current choices and restore.
The conflicting-profile case checked current settings against the actual damage
consumer without rewriting either input file. These five critical-profile
controls therefore have bounded results on the explicitly stated graphs;
identical-fixture baseline comparisons remain unrun. The inventory case and its
per-phase sentinel checks are described in [ext-small.md](ext-small.md#live-inventory-settings).

The Windows Cargo build returned zero, but its compile controller failed
descendant cleanup and the child identity remains unknown. That controller and
its overall result remain false. Runtime-only r1 then stopped with `KeyError` on
missing `priorResult` metadata before any Job, discovery or test. Corrected r2 reused
the sealed emitted image without rebuilding and passed all three opt-ins; its
runtime qualification is separate from those failed attempts.

Earlier hosted engine and engine-Clippy jobs stopped at E0599 because the shared
damage fixture called nonexistent `Rules::default()`. The fixture was corrected
to the existing `Rules::defaults()` without changing damage inputs or oracles.
Neither failed job executed an engine control or established a separate lint
failure. Later results above do not relabel those attempts as successful.

Accepted Main `76f69f6f` is now integrated normally. Its shared runtime/build
inputs differ from the historical graphs above; all exact integrated-source
compiler, runtime, strict-lint and hosted checks remain UNRUN. This documentation refresh
changes no implementation or fixture, and transfers no earlier result to that
new composition.

There is no provider-version increase, popup suppression, auto-dismissal or mod
disable. The historical941 report is not validated by this bridge. The960/970
startup guards remain blocked until their actual required services and compatible
reporting contract are established. Native import/reload and scale-enable parity,
paired original-game results, GPU/installed-mod acceptance and complete Stewie
compatibility remain unqualified.
