# Loaded global-image metrics

[documented: GECK Wiki, Tutorial: Understanding XML, Rendering and Images]
Image filewidth/fileheight are engine-supplied own-image dimensions after numeric
zoom; stretch zoom has zero file dimensions. Visibility governs drawing, not
whether another expression can read an image's dimensions.

[observed: private installed metadata and current source, no mod text transcribed]
A loaded globals subtree contains a hidden own-image calibration. A global
expression divides a fixed logical metric by its fileheight; menu image zooms
depend on the result. Current page-specific writers traverse their open menus
only. They never visit that global image. Current division-by-zero preserves the
numerator, producing a much larger zoom and sampling only a tiny source corner.
The hand-built condition density correction is a different consumer.

[design] Before the existing post-script expression update and late page settle,
write own-image metrics throughout the loaded globals subtree, including hidden
images. Reuse the actual FontKit/Pictures/VFS own-image decoder; do not guess a
mod's desired zoom or replace its authored expressions. A missing, empty or unsafe
new source has zero metrics rather than retaining another source's dimensions.
Nonempty-atlas-tagged and non-global images are unchanged. Empty/whitespace atlas values select an own file, matching the actual painter. Every unchanged numeric metric
makes no write. This does not impose a new general atlas-density rule.

[design] Bound the new work to 4096 visited global nodes and 64 image children.
Admit the complete traversal before image reads or metadata writes. Log changes
in coarse resolution/unavailable/limit counts, not every frame. Cache only the
currently referenced filenames within that bounded tree; the complete globals
TileId distinguishes a rebuilt UI and clears cached dimensions. The normal
GameData/VFS is immutable for that UI lifetime. Same-name external asset hot
replacement without UI reload, atlas metrics and native timing parity are not
claimed. Symbolic/nonpositive/nonfinite zoom produces zero dimensions; finite
products are required. These are the new writer's explicit safety policies.

Six authored controls cover the real global expression and child Piece/placed
geometry, filename/missing recovery and cache reads, live zoom and invalid
products, UI replacement/no-op writes, bounded admission/untouched neighbors,
and actual scheduled FontKit/VFS decoding before expression settle. All are
uncompiled/unrun until a separately reported CPU execution. No game assets,
installed XML or original text are included in these controls. They do not prove
GPU pixels, a full Pip-Boy schedule, whole mod support or native rendering parity.

[design: resource admission] No GameData resource means no metric read, write or negative cache entry. A later inserted data resource is admitted normally; a changed GameData resource clears cached dimensions before resolution. The scheduled fixture starts with UI but no data, then inserts its real authored VFS resource under the same UI identity. This is separate from a real decoded missing file.
