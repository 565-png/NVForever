# Drawn script model attachments

Implementation context: at pinned public main
`63db0209cc77ab7ce45eed44a90fecf925e47dd1`, `nif_blocks::call` records
AttachModel specifications but `nvforever::nif_mirror` does not draw them. Modes
and runtime lists are documented in [ext-jippp.md](ext-jippp.md); this change
implements the reference attachment consumer, not a native plugin ABI.

## Consumer and ownership

[hypothesis: engineering policy] A reference attachment is a registered structural
intent plus one owned instance in the selected concrete model. Mode 3 can create
it in the current model, waiting for an initially absent parent. Mode 1 waits for
the next model if one has already been published. Mode 0 stops future creation
but retains an already drawn instance. Mode 2 also removes that exact instance. The existing
script return values and runtime-list API remain unchanged. Legacy/headless edit
drains also retire unconsumed structural side requests; the drawing producer takes
them before its mirror drains ordinary edits. The omitted player
view retains the public mirror's third-person default; this adds no selector.

[hypothesis: engineering policy] A first available model pins a cold request.
An observed or synchronous removal of its Transform, or of a captured parent,
retires its instance. A different parent with the same name does not inherit it
within that model. A pending parent request invalidated before its first creation
does not gain authority in a replacement model; a fresh request is required. A
new model can replay structural intent that already produced an instance; old
translation/rotation/scale holds retain the existing concrete-entity lifetime
and are not replayed; attachment material values belong to each produced entity.
A fresh explicit registration permits a parent-lifetime retry. Model preparation
retains the existing cache and its remembered failures. Repeated registration of
a live instance does not duplicate geometry. Session replacement retires all
old owned instances. Exact ownership, not a display label, controls mode 2.

Implementation context: `ModelCtx` owns model preparation and
its cache and sweeper. Mirror-only hosts may not yet have the asset resources;
the producer still captures queued recipients and processes session/lifetime
retirement and explicit removal. Only model preparation waits for those resources,
so asset arrival does not revive an expired recipient. A separate attachment variant retains the source graph;
ordinary and streamed models do not allocate it. Attachment node metadata and
drawable indices are captured from the same decoded file during preparation; there is no second file read joined by
name. Duplicate and absent names therefore do not collapse geometry, materials,
local transforms or roots. Static models retain every source root. Animated
models use the existing Rig spawn path for embedded idle animation and material
tracks; an incomplete rig-to-source correspondence is refused before any entity is created. Existing
model, texture-role, material and animation preparation remains authoritative.
The host's GroupRig, actor PlayGroup routing and collision-pose publication retain
their sole reference ownership. Attached named-group dispatch is not enrolled as
a second unqualified GroupRig for the same FormId.

[hypothesis: engineering policy] The producer validates the complete cached graph
before issuing Commands. Removal retires only entities bearing this instance's
ownership marker, preserves foreign children, and never searches by names.
Dependent owned attachments retire in the same producer pass rather than drawing
one frame as unparented objects. The
producer preserves registration order for duplicate-name publication. Its bounded
ancestry checks accept at most 255 parent links and refuse deeper attachments before
spawning; checked instance IDs never wrap. The mirror publishes real resulting
entities and attachment-local material/property metadata only after the Commands barrier. Newly created node metadata is not
invented before the producer runs. The existing AttachLight path is independent.

## Bounded coverage

Authored controls use compiled AttachModel/NIF queries, synthetic NIF bytes,
actual ModelCtx/mesh/material assets and the production producer/mirror. They
cover missing files/parents, replacement and same-entity Transform removal,
rename/namesake isolation, duplicate/unnamed shapes, multiple static roots,
mode 0/1/2/3, cache source consistency and neighboring owners. No game assets,
DLL implementation or native addresses are used. Qualification is tied to the
exact source and execution receipt; source alone establishes no rendering result.

Base-form-wide replay, CopyRuntimeNodes/ClearRuntimeNodes ownership semantics,
particle simulation, attached named-group dispatch, legacy alpha/stencil override
lifetime, native parity and
full B42 mod acceptance are outside this reference-consumer leaf. No capability/version spoofing or silent original-model
fallback is introduced.
