# Callable identity in the source runtime

[documented: `crates/fnv-esm/src/loadorder.rs`, `crates/fnv-sim/src/form.rs`] The accepted
load order admits 255 plugins, whose indices include FE. A loaded script record
can therefore have a number in the ranges also used for runtime callables.
`GameData::script` is the record identity resolver; a numeric threshold alone
does not identify a lambda or a text-compiled function.

[hypothesis: engine design] Dispatch and arity use the loaded script resolver or an exact owned
runtime-handle entry. Function-script local strings/arrays are disposed according
to that resolved kind. Lambdas keep the existing maker-instance/captured-local
ownership, receiver and return-value rules. Event filters are unchanged.

[documented: `crates/fnv-script/src/vm.rs`, `Value::as_form` and `Vm::host_function`]
An omitted receiver is distinct from an explicit empty receiver. The existing
extended-call guard stops an explicit empty call before argument evaluation.
[hypothesis: engine design] Apply that same guard to `Call` and direct function
admission. Omitted calls inherit the caller; nonempty UDF receivers retain their
existing forwarding policy without a new actor/base-form restriction.

[hypothesis: engine design] Newly allocated lambda and text-compiled-function handles skip loaded
forms, live travel-package overrides and owned callables, and stay within their
existing internal regions. Travel packages use the same exclusions in either
allocation order. A handle already returned
to a script is not rewritten. Lambda snapshots carry an optional explicit handle;
older snapshots retain the previous base-plus-position interpretation. A snapshot
with duplicate or record-colliding handles is ambiguous and cannot be silently
retargeted; this is an engine validation limit, not an original-game observation.

[documented: `crates/fnv-game/src/game.rs`, snapshot/restore; `nvse_jip13.rs`]
Generated travel-package definitions are transient and are not serialized by the
accepted snapshot format. This change does not add their persistence; a newly made
package after restore must still exclude the restored owned lambda handles.

[hypothesis: engine design] Existing session eligibility is preserved for older low-valued callback
references, and loaded script records are eligible regardless of numeric prefix.
Owned lambda callbacks remain scoped to their existing game lifetime. Authored, unexecuted
regressions target registration through the compiler, the real Game hit recorder
and callback execution, ordinary UDFs, captured lambdas, filters, receivers,
snapshot identity and deferred calls. Further authored controls exercise source
and source-less loaded bytecode, same-maker nested captures, live package/lambda
allocation, array returns and string/array disposal after success and error.
No native callable representation or parity
is established by these engine tests.

[documented: `GameConfig::default`, `Vm::eval` and extended-statement dispatch]
Default runs recover an ordinary host-expression error and record it as skipped;
strict runs return that error. Allocator refusal controls exercise both existing
contracts without changing them. [documented: `Exec::make_lambda`, `lambda_key`]
An existing maker/instance identity is looked up before requesting a new handle.
Capacity controls therefore retain cached reuse while refusing a distinct new
lambda, with no new slot, owner mapping or allocator-cursor change on refusal.

[documented: `fnv_script::unit::read_record`, `fnv_game::source_compile`]
An owned SCPT fixture places record metadata before its contiguous SCHR script
unit, so its SCTX belongs to that unit. Source presence alone does not establish
source execution: the receiver fixture's typed local selects the existing Auto
route, and its admission checks require a successful source-compile entry.
The paired source-less units require decoded SCDA with no source-compile entry.
Both paths retain the same receiver, side-effect and no-mutation oracles.
