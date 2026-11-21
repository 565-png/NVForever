# ySI filename configuration subset

The original Sorting Icons configuration's author documentation describes `tags` and
`icons` sections, additional JSON files contributing those sections, highest-priority
matching tag rules, form/property conditions and explicit mod/form/formlist selectors.
Icon records associate a tag with its template and filename [documented: Sorting
Icons 1.4b package, `menus/ySI/ySI.json` author documentation; original configuration
is not redistributed].

`yUI` (version 140) registers `ySIGetTrait` (`0x21D0`) and `ySISetTrait` (`0x21D1`)
[documented: GECK wiki `YSIGetTrait`/`YSISetTrait`; observed: original FNV 1.4.0.525
with xNVSE 6.4.9 and yUI 140]. In that original-game runtime, `ySIGetTagTrait` is not
registered by any loaded plugin, and calling `ySIGetTrait "filename"` on an item
configured with a tag and icon filename (`FalloutNV.esm` `00000F`) returns the
configured icon filename string while passing a string tag instead of a form fails
script compilation [observed: original FNV 1.4.0.525 with xNVSE 6.4.9 and yUI 140].
Accordingly, `ySIGetTagTrait` is left out of NVForever, and the bounded configuration
reader below is wired to `ySIGetTrait "filename"` while preserving the existing
in-memory `ySIGetTrait` / `ySISetTrait` `"tag"` and `"icon"` behavior.

## Source mapping

The bounded `ySIGetTrait "filename" item` / `item.ySIGetTrait "filename"` mapping
evaluates the documented configuration fields through existing record and inventory
consumers. The explicit-item `"filename"` return is [observed: original FNV 1.4.0.525
with xNVSE 6.4.9 and yUI 140, single configured item]. Placed-item receiver forms,
multi-rule precedence/ties, and bad-item/no-rule outputs on `ySIGetTrait "filename"`
remain [hypothesis] until an interactive-desktop multi-rule capture runs.

- Explicit `mod` + hexadecimal local `form`, including a list of form strings, and
  `formType` scalar/list predicates are evaluated [documented: field names, package
  author documentation above]. [hypothesis] `form` is the hexadecimal local FormID in
  the named plugin, and `formType` uses the same numeric codes as the existing
  form-type resolver. The existing form-type resolver supplies the record type;
  placed item references resolve to their base, and an existing temporary item base
  resolves through its original [engine design choice].
- The package's `questItem` condition reads the existing World quest-item predicate.
  [hypothesis] Integer 0/1 mean false/true; other representations remain unknown. The
  predicate includes the authored quest-item flag and saved SetQuestObject overrides,
  including an explicit clear of an authored flag. [engine design choice] Rules are
  cached, while this predicate reads current World state on each query; native
  classification timing after a flag edit is unobserved.
- [engine design choice] Other predicates, including `formlist` and
  weapon/aid/property conditions, are **unknown**, rather than false. A certainly
  false supported predicate rejects a rule even if it also has unknown predicates. A
  possible unknown rule at or above the winning priority refuses the query; an
  unknown lower rule cannot win.
- Multiple loose configuration files contribute rules [documented: package author
  documentation above]. [engine design choice] Equal-priority distinct tags refuse
  instead of guessing native file order. Identical winning tag outputs agree. Winning
  icons must have one unambiguous highest-priority filename. Missing tags, missing
  filenames and conflicting icon ties refuse; no native default is invented.
- [engine design choice] The main `menus/ySI/ySI.json` is required. The existing Data
  reader selects each loose file/engine overlay, with the existing host reader as
  fallback for the main file. Additional archive-only JSON enumeration is not
  supported. A private local reader accepts comments and trailing commas, preserving
  quoted content [hypothesis: that the original reader accepts them too]. Generic JSON
  functions and their strict failure behavior are unchanged.
- [engine design choice] Configuration is a snapshot at the first query in each Game
  session, including a load failure. Restore creates a fresh cache. This timing avoids
  repeated filesystem/rule parsing during inventory comparisons; it does not establish
  native hot-reload timing or recover after late file attachment in the same session.
- [engine design choice] Accepted configuration is capped at 32 files, 4 MiB total
  parsed input and 20,000 rules (resource bounds, not native limits). The existing
  reader obtains a file before its accepted-size check; this is not a pre-read
  allocation cap.

The literal `string` trait, default/missing outputs, native predicate behavior outside
this subset, case normalization, provider version and native encoding remain
unqualified. They explicitly refuse. No provider presence/version is advertised.
This does **not** repair JAM sorting or establish full ySI/JAM compatibility. A
configuration with unresolved potentially winning predicates cannot claim a filename
merely because a lower supported rule exists. No original JSON, icons, scripts or
private paths are included.

## Source string arguments

[documented] Old string-parameter functions require explicit string conversion or
an inline expression when given a string variable
([String Variable](https://geckwiki.com/index.php?title=String_Variable),
[Inline Expression](https://geckwiki.com/index.php?title=Inline_Expression)).
The source-to-UI controls use `SetUIString "path" (icon)`; the additional
`SetUIStringAlt ("path") $icon` route mirrors the original JAM icon writer's
argument shape with independently authored paths and values [observed: original
JAM4.6 script workload only, not original-game execution]. A bare variable token
on the legacy call retains its existing literal-text meaning. Compiler, VM and UI
string-argument behavior are unchanged by the fixture correction.

## Authored controls

Nine source controls are in the dedicated `ysi_filename_source` test target,
isolated because the existing file roots are process-wide. They cover inventory,
array-element and placed receivers; source-to-existing UI filename assignment;
multiple files and priority/ties; unknown higher/lower/decisively-false rules;
unsupported traits/invalid receivers/missing icons; Game save/restore cache lifetime
with retained inventory; comments versus generic strict JSON; resource bounds and
malformed input; and authored/script-set/cleared quest flags through source and save.
The fixtures contain independently authored configuration, items and menu XML.

Historical public source `6edb715d` executed all nine controls once: seven passed and
two failed at their first UI filename reread. Both failing callers passed bare
`icon` to legacy SetUIString and stored that literal token. Later assertions in
those tests were not reached. The corrected callers use `(icon)`, preserving all
expected filenames, receiver, inventory and save assertions. The first consumer
control also retains the bare-token behavior and exercises explicit Alt/string
conversion. Public source `eaff6072`, executed as merge `353657d7` with the same
tree, then passed all nine controls once. Hosted core tests passed 3,463 with five
ignored; engine tests passed 3,409 with three ignored; both strict Clippy checks
and all seven required checks passed. These results qualify that exact source
and the existing UI trait consumer, not texture loading or rendering.

Two additional source-to-UI controls exercise the aggregate 4 MiB accepted-byte
bound and the aggregate 20,000 valid-rule bound. Each accepts the exact limit across
two valid JSON files, refuses one additional byte or valid rule before UI/world
changes, and checks retained success/error session snapshots versus a fresh Game.
The rule control selects the final matching rule, including a separately counted
icon rule, so it cannot pass by parsing only an initial subset. Public source `fe5cd2f0`, executed as merge `3286e5c8` with the same tree,
passed all eleven controls exactly once, including both aggregate-cap cases.
Core tests passed 3,465 with five ignored; engine tests passed 3,409 with three
ignored. Both strict Clippy checks and all seven required checks passed. The
earlier nine-case result remains scoped to its own revision. This later
documentation-only successor changes no Rust, fixture, registry or Cargo bytes
and has no additional runtime result. These are source/UI-trait consumer checks,
not texture decoding, GPU, original native or installed-mod acceptance.

The fixture-identical pre-quest comparator `76e26d14` remains **UNRUN**, as do the
earlier queued `253`/`5e3` inputs; no baseline/candidate outcome is inferred. Native/provider/default/order/timing behavior, GPU texture loading
and complete installed ySI/JAM acceptance remain unqualified.
