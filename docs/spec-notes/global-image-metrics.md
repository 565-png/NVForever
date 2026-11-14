# Loaded global-image metrics

[documented: GECK Wiki, Tutorial: Understanding XML, sections 3.2(b) and
3.2(d), https://geckwiki.com/index.php?title=Tutorial%3A_Understanding_XML,
read 2026-11-14] An image filename is relative to Data/Textures. Numeric zoom
is a percentage; zero hides the image and -1 stretches it to its element.
The engine supplies read-only filewidth/fileheight after zoom, with zero for
stretch zoom. The visibility property controls display and input. This source
does not establish when hidden global-image metadata is evaluated or readable.

[observed: private installed XML metadata inspection, no mod text transcribed]
A loaded globals subtree contains a hidden own-image calibration. A global
expression divides a fixed logical metric by its fileheight, and menu image
zooms depend on that result. This is a consumer observation, not a native
rendering measurement or proof that a full Pip-Boy figure is fixed.

[documented: NVForever public source at
576d39630287845c598914d17ddffd71eebbef28,
engine/nvf-play/src/tiles/mod.rs and its page-specific image-metric writers]
The existing writers visit their open menus rather than the loaded globals
image. The UI expression evaluator retains the numerator on division by zero
(crates/fnv-ui/src/tree.rs::apply, OpKind::Div). In that source path the missing metric can therefore
produce a much larger dependent zoom. The hand-built CND density consumer in
PR #2475 is separate.

[hypothesis: engineering policy] Before the existing post-script expression
update and late page settle, write own-image metrics throughout the loaded
globals subtree, including hidden images. Hidden metadata remains usable for
dependent expressions under this proposed policy; native timing is unmeasured.
Reuse the actual FontKit/Pictures/VFS own-image decoder without guessing a mod's
desired zoom or replacing its authored expressions. A missing, empty or unsafe
new source has zero metrics instead of another source's stale dimensions.
Nonempty-atlas-tagged and non-global images are unchanged. Empty/whitespace atlas
values select an own file, matching the public painter. Unchanged numeric metrics
make no write. This does not impose a new general atlas-density rule.

[hypothesis: engineering safety and cache policy] Bound the new work to 4096
visited global nodes and 64 image children. Admit the complete traversal before
image reads or metadata writes. Log coarse resolution/unavailable/limit changes.
Cache only currently referenced filenames within that bounded tree; a rebuilt
globals TileId clears cached dimensions. Same-name external asset replacement
without UI reload and native rendering parity are outside this cache contract.
Symbolic/nonpositive/nonfinite zoom produces zero dimensions; finite products
are required. These bounds are engineering choices, not measured native limits.

[hypothesis: resource admission policy] Without GameData there is no metric read,
write or negative cache entry. Later insertion is admitted normally; a changed
GameData resource clears cached dimensions before resolution. The scheduled
fixture first has UI without data, then inserts actual authored VFS data under
the same UI identity. This differs from a decoded missing image.

[observed: authored CPU controls, qualified Windows normal26e host plus six
retained UI source leaves; result SHA256 prefix
011a0bf6, independent audit SHA256 prefix 0fcc47c7] All six selected controls passed,
each with zero failures/ignored cases and 1726 other tests filtered. They cover
actual global expressions and child Piece/placed geometry, filename invalidation
and recovery, live zoom/invalid products, UI replacement/no-op writes, bounded
admission/untouched neighbors, and scheduled FontKit/VFS resolution before
expression settle. The prior fixture String/&str compile failure executed zero
tests and is retained separately. No game assets, installed XML or original
game text are included in these authored controls.

[observed: current public-port review state] This port's Rust build, strict
Clippy, canonical CI and opt-in game smoke are unrun. The qualified CPU host is
not whole public main, a pristine-main baseline pair or native/GPU acceptance.
The 4096-node boundary has no dedicated case; the resource fixture tests initial
absence then insertion, not replacement after an already cached decoded miss.
A full Pip-Boy schedule, complete mod support and native rendering parity remain
unproved.
