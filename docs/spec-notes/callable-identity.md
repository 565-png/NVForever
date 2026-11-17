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

[hypothesis: engine design] Newly allocated lambda and text-compiled-function handles skip loaded
forms and stay within their existing internal regions. A handle already returned
to a script is not rewritten. Lambda snapshots carry an optional explicit handle;
older snapshots retain the previous base-plus-position interpretation. A snapshot
with duplicate or record-colliding handles is ambiguous and cannot be silently
retargeted; this is an engine validation limit, not an original-game observation.

[hypothesis: engine design] Existing session eligibility is preserved for older low-valued callback
references, and loaded script records are eligible regardless of numeric prefix.
Owned lambda callbacks remain scoped to their existing game lifetime. Authored, unexecuted
regressions target registration through the compiler, the real Game hit recorder
and callback execution, ordinary UDFs, captured lambdas, filters, receivers,
snapshot identity and deferred calls. No native callable representation or parity
is established by these engine tests.
