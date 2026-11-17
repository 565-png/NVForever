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

## Validation limits

Two ordinary core controls characterize supported and malformed typed inputs.
One opt-in engine control uses dedicated process-local authored Data/settings
roots, actual Runtime initialization, Restart/NewGame/Load runner files,
StewVal calls, real critical chance/damage calculation, settings-file loading and
restoration. Run its exact selector alone with NVF_STEWIE_PROFILE_HOME pointing to
a dedicated fixture directory and --test-threads=1; ordinary parallel CI skips it.
It retains the actual version guard's rejection and queued message. Skipped is not
passed. Earlier compile attempts failed before selected runtime; corrected
compilation and all five runtime controls are currently **UNRUN**. The added
ordinary core control covers both aliases, current-field updates, unbound and
unrepresented keys, malformed input, save lifetime and rebinding. An additional
opt-in engine control retains identical conflicting-profile assertions on the
baseline and candidate, comparing loaded source values and startup-file results
with actual critical-hit rules without rewriting either input file. The frozen
baseline mismatch is source-traced, not an executed failure.

There is no provider-version increase, popup suppression, auto-dismissal or mod
disable. The historical941 report is not validated by this bridge. The960/970
startup guards remain blocked until their actual required services and compatible
reporting contract are established. GPU/native/installed-mod acceptance and full
Stewie compatibility are not claimed.

The hosted engine and engine-Clippy jobs both stopped at E0599 in the shared
damage fixture: `Rules` has no `Default` constructor. The fixture now calls the
existing `Rules::defaults`, which uses the documented settings without a load
order. Its critical-setting assignments, damage inputs and every expected result
are unchanged. Engine-Clippy did not establish a separate lint failure; neither
job executed an engine control. The corrected source remains uncompiled/unrun.
