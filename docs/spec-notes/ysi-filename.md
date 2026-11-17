# ySI filename configuration subset

The original Sorting Icons configuration's author documentation describes `tags` and
`icons` sections, additional JSON files contributing those sections, highest-priority
matching tag rules, form/property conditions and explicit mod/form/formlist selectors.
Icon records associate a tag with its template and filename [documented: Sorting
Icons 1.4b package, `menus/ySI/ySI.json` author documentation; original configuration
is not redistributed]. Original JAM item UDFs request the `filename` trait with an
explicit item, while sort UDFs request `string` using both explicit items and chained
receivers [observed: original mod script workload, not a runtime observation].

## Source mapping

The bounded `ySIGetTagTrait "filename" item` / `item.ySIGetTagTrait "filename"`
mapping evaluates the documented configuration fields through existing record and
inventory consumers [hypothesis/design]:

- Explicit `mod` + hexadecimal local `form`, including a list of form strings, and
  `formType` scalar/list predicates are evaluated. The existing form-type resolver
  supplies the record type; placed item references resolve to their base, and an
  existing temporary item base resolves through its original.
- The package's `questItem` condition reads the existing World quest-item predicate.
  Integer 0/1 mean false/true in this bounded source mapping; other representations
  remain unknown. The predicate includes the authored quest-item flag and saved
  SetQuestObject overrides, including an explicit clear of an authored flag. Rules
  are cached, while this predicate reads current World state on each query
  [hypothesis/design: native classification timing after a flag edit is unobserved].
- Other predicates, including `formlist` and weapon/aid/property conditions, are
  **unknown**, rather than false. A certainly false supported predicate rejects a
  rule even if it also has unknown predicates. A possible unknown rule at or above
  the winning priority refuses the query; an unknown lower rule cannot win.
- Multiple loose configuration files contribute rules. Equal-priority distinct tags
  refuse instead of guessing native file order. Identical winning tag outputs agree.
  Winning icons must have one unambiguous highest-priority filename. Missing tags,
  missing filenames and conflicting icon ties refuse; no native default is invented.
- The main `menus/ySI/ySI.json` is required. The existing Data reader selects each
  loose file/engine overlay, with the existing host reader as fallback for the main
  file. Additional archive-only JSON enumeration is not supported. A private local
  reader accepts comments and mapped trailing commas, preserving quoted content.
  Generic JSON functions and their strict failure behavior are unchanged.
- Configuration is a snapshot at the first query in each Game session, including a
  load failure. Restore creates a fresh cache. This mapped timing avoids repeated
  filesystem/rule parsing during inventory comparisons; it does not establish native
  hot-reload timing or recover after late file attachment in the same session.
- Accepted configuration is capped at 32 files, 4 MiB total parsed input and 20,000
  rules [hypothesis/design resource bounds, not native limits]. The existing reader
  obtains a file before its accepted-size check; this is not a pre-read allocation cap.

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
icon rule, so it cannot pass by parsing only an initial subset. These two new
controls, compilation and the full eleven-case successor target are **UNRUN**;
the nine-case hosted result above remains scoped to its exact earlier source.
Production, registry and Cargo files are unchanged.

The fixture-identical pre-quest comparator `76e26d14` remains **UNRUN**, as do the
earlier queued `253`/`5e3` inputs; no baseline/candidate outcome is inferred. Native/provider/default/order/timing behavior, GPU texture loading
and complete installed ySI/JAM acceptance remain unqualified.
