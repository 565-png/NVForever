# Inline arguments inside a vanilla expression

[documented: public compiler implementation at commit `576d39630287845c598914d17ddffd71eebbef28`,
`crates/fnv-script/src/compile/gen.rs`, `gen/x.rs`, and `lex.rs`]
The vanilla expression emitter keeps its own postfix operator stack: `||` binds
more tightly than `&&`. The existing extender argument parser accepts a grouped
argument as one expression. Its outer expression grammar has different logical
precedence. Replacing an entire vanilla condition with that grammar would change
conditions unrelated to the grouped argument.

[observed: selected installed mod SCPT record data inspected privately,
2026-11-14; no source text or compiled payload is included here]
One selected sourced unit has balanced outer conditions and a two-parameter
vanilla function call whose second parameter uses an opaque inline encoding.
The current decoder retains that argument as raw. The source-compile selection
therefore chooses the unit in Auto mode. Its known vanilla call is then emitted
without its grouped second argument, leaving two outer expression values. The
old manual log identifies the same unit and stack error, but does not bind the
historical in-memory bytes or failing instruction offset.

[documented: `crates/fnv-game/src/source_compile.rs`]
Auto source compilation seeds the unit's variables and references, and selects
a sourced unit when its decoded representation contains raw pieces. It must not
silently change a condition's surrounding vanilla arithmetic or logical order.

[hypothesis: bounded source-bridge policy] In source-recompile or extender
contexts, a known fixed-arity vanilla call may have a grouped argument. Parse
only that complete call operand through the existing extender argument parser
and emit the existing internal extended-expression token. Keep every surrounding
vanilla postfix operator and its precedence. Preserve the original text span,
including whitespace, signed arguments, strings and reference spelling; do not
reconstruct the argument by joining stripped tokens.

[hypothesis: preservation] Calls without grouped arguments and ordinary editor
compilation keep the existing emitter. Ordinary grouping around a comparison is
not an inline function argument. The lexer may optionally report character spans
without changing its existing token values. No VM stack relaxation, native
inline-bytecode decoder, function/opcode special case or installed source rewrite
is introduced.

[documented: authored control source to accompany this leaf]
Controls must discriminate a two-parameter grouped argument, vanilla mixed
`&&`/`||` precedence, quoted/signed/reference arguments, ordinary neighbors, and
the real Auto source-selection path using an authored opaque argument plus
authored SCTX. An opaque authored test argument is only a selection trigger, not
a claimed native encoding. The exact baseline and candidate use identical test
source. Qualification belongs to the exact source/fixture revision in the
submission; no native or whole-mod behavior is inferred from these authored cases.

## Parser-bound call extent and receiver preservation

[documented: authored six-control and separate eight-edge CPU receipts,
2026-11-14] The initial implementation passed all six primary controls and five
of eight edge controls. Three edge controls still failed: a bare nested second
argument, a spaced-dot receiver, and a quoted EditorID receiver. The ordinary or
explicit neighbor passed before each grouped-case failure. All eight grouped
edge forms failed on the unchanged baseline; these are limits of the proposed
extension, not evidence that a previously working grouped form regressed.

[documented: public `Gen::function`, `Gen::reference`/`Gen::form`, and
`gen/x.rs::xargs`] The vanilla token already identifies the function and its
receiver, including a joined spaced dot and a quoted EditorID. The existing
argument parser knows how many tokens a nested call consumes. Re-parsing the
receiver as a string or counting a nested function name as its entire argument
loses those existing bindings.

[hypothesis: revision-two policy] Reuse that resolved vanilla function/receiver,
then ask the existing argument parser to consume its arguments from the original
source suffix. Map its consumed token prefix back to the existing vanilla source
spans, and emit one call operand only when that consumed range contains a grouped
argument. Preserve the surrounding vanilla operator stack. A probe that is not a
grouped call restores its temporary name/variable/diagnostic state before the
ordinary emitter runs. Do not alter the general extended receiver grammar.

[documented: unchanged authored control sources] Revision two retains every
assertion and input in the primary six, separate eight edges, span unit, and real
Auto source-selection fixture. The final parser-bound compiler revision passed the six primary controls, eight
edge controls and separate span unit in the bounded public compiler target; this
compiler-only receipt does not by itself qualify a whole source port or the
separate GameData Auto path. Nested receiver combinations, optional-argument ambiguities, malformed
syntax, a full source corpus, and native grammar equivalence remain unqualified.

## Bounded validation and engine-query aid

[documented: authored CPU proof `7c310d1e` and independent review `d13f5572`]
The final compiler production blobs were tested from a common public-API layout
on Rust 1.99, normal Cargo release/default features: primary six and edge eight
passed, plus the one span test (14 other unit tests filtered). Unchanged-base
receipts were adopted rather than rebuilt: primary two passed/four stopped at
first runtime assertions; edge zero passed/eight stopped at grouped-call runtime
assertions after passing ordinary/explicit neighbors. The earlier candidate's
three edge failures remain historical evidence. These are authored compiler,
decoder and VM controls with a controlled Host, not a native grammar certificate.

[documented: `lex.rs::MAX_LINE` and source peer `8f016e91`]
The 512-character threshold reports a diagnostic; it is not an enforced input
cap. Prefix mapping may re-lex multiple original substrings. No general parser
performance bound, complete grammar support or full-source identity/execution
claim is made from these bounded fixtures.

[documented: authored Auto fixture, `source_compile.rs`, and R4 CPU proof
`e4e71c65`]
The latest public576/production087 pair passed strict plugin/SCPT/read_record,
owner/source/SCDA/raw admission, actual selected Auto-unit identity and dispatch
admission. The unchanged baseline then failed its first global-value assertion
(the initial5 remained instead of false-case0); its later true case was unrun.
The candidate's one selected test passed both false/true quest-to-global states,
with 146 other synthetic tests filtered. Both sides used the same authored
fixture, including corrected f32 expectations and EDID before the contiguous
unit group. The first type setup failure and later source-extraction setup
failure remain historical and are not grouped-call behavioral results.

This is a bounded real GameData Auto/Game::run_event/quest getter/global writer
control on the shared public graph. It is not execution of this whole current-main
port, automatic game scheduling, the renderer, a named mod or native bytecode.
The executing-worker reconciliation is sealed; independent final receipt review
is pending at this source proposal.

[hypothesis: optional engine-query smoke]
An authored QueryJob runs an ordinary self-distance neighbor, a grouped receiver
condition, a grouped Set expression and a mixed-logical-operator condition in
actual game query dispatch. The only expected distance is a reference to itself;
all other values come from authored arithmetic/branches. PrintC emits owned
sentinels, and the existing job reports five rows with zero errors. This tests
runtime query compilation/dispatch, not the Auto loader, automatic game scripts,
pixels, native parity or a named mod. The registered scenario is unrun.

[documented: public `Game::compile_file`, `compile_source`, `nvse_rest`, and
`mod_files::udf_data_path` at pinned main
`07878551131e14ed6ed8b771d34adff86138f3ee`]
CompileScript takes a UDF-relative file path, not inline source text. The optional
smoke's fifth query compiles the authored file `vanilla-inline-call-owned.gek`
through compile_mod_runtime and calls the resulting function. Its local starts
at -999, the vanilla Set target executes, and only then does SetFunctionValue
return result+1. Success requires PrintC's owned result1 sentinel; a missing
script, aborted target or default return0 cannot satisfy it. Zero query-job
errors is an independent expectation.

[hypothesis: owned-home preparation contract]
Before running this optional scenario, set NVF_PLAY_TEST_INLINE_SOURCE_HOME to a
new dedicated writable home and copy only the authored .gek fixture to
mods/NVSE/user_defined_functions/NVFInlineOperands/owned_self_distance.gek below
that home. The repository fixture remains an input, never the writable HOME.
Do not write into installed Data or reuse a player's config/save directory. Bind
and archive the seeded bytes and generated query output in a future controller.
No source file, scenario or result here authorizes a renderer launch; this smoke
has not been compiled or executed.

[documented: pinned main
`07878551131e14ed6ed8b771d34adff86138f3ee`,
`crates/fnv-install/src/lib.rs::EngineHome::mods` and
`engine/nvf-play/src/script_input.rs::mod_file_roots`]
The engine's loose-file layer is HOME/mods, followed by installed Data. Therefore
the authored UDF must be seeded under HOME/mods/NVSE, not HOME/Data/NVSE. The
previous source-only seed note was wrong; no smoke was executed with it. The
script body, query sources and success expectations are unchanged by this
preparation correction.
