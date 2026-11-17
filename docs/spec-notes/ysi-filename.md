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

## Authored controls

Seven source controls are authored in the dedicated `ysi_filename_source` test target,
isolated because the existing file roots are process-wide: source calls with an
inventory item, array-element receiver and placed item reach the real UI filename
trait through ordinary SetUIString; multiple files and highest-priority/tie policy;
unknown higher/lower/decisively-false rules; unsupported traits/invalid receivers and
missing icons; actual Game save/restore cache lifetime with retained inventory;
comment preservation versus the existing strict JSON reader on the same path; and
mapped file-bound/malformed-input refusal. The fixtures contain only independently
authored configuration, items and menu XML. All compilation/runtime controls remain
**UNRUN**; no GPU texture loading/render, native or installed-mod success is claimed.

Two additional controls exercise the quest-item predicate through compiled source
and the existing UI filename recipient, authored versus script-set/cleared flags,
Game save/restore and retained inventory. A certainly false quest predicate can
disqualify an otherwise unknown higher-priority rule; an unknown possible winner
or unsupported predicate value still refuses. These controls are also **UNRUN**.
