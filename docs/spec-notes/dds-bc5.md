# Legacy BC5 data and normal-map consumption

[documented] Microsoft's DDS Programming Guide maps legacy FourCC `BC5S` to
DXGI_FORMAT_BC5_SNORM and `ATI2` to BC5_UNORM. A `DX10` FourCC instead introduces
an additional 20-byte header. This leaf supports the two documented legacy
codes, not DX10 or an unverified `BC5U` FourCC alias. Source (2026-11-14):
https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide

[documented] BC5 has two independent 8-byte channels, red then green, over the
same 4x4 texels. Each channel has two 8-bit endpoints and sixteen three-bit
row-major indices. Signed endpoints use SNORM8 (both -128 and -127 map to -1).
Raw signed endpoint comparison chooses six interpolated values when the first
is greater; otherwise four interpolated values plus -1/+1. Unsigned uses 0..1
and extra values 0/+1. Interpolate floating-point decoded endpoints, not the
integer-rounded DXT5 alpha palette. Microsoft's reference keeps the raw signed
comparison even for -127 versus -128. Sources (2026-11-14): Direct3D 11.3
Functional Specification sections 3.2.3.3 and 19.5.9-19.5.12, and Block Compression:
https://microsoft.github.io/DirectX-Specs/d3d/archive/D3D11_3_FunctionalSpec.htm
https://learn.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-block-compression

[hypothesis/engineering policy] The core decoder exposes normalized two-channel
floats without a normal role, Z reconstruction, green flip or sRGB. Preserve
existing header, mip/face bounds and trailing-byte policy. Generic linear GPU
images use native BC5 when supported and block-aligned; CPU fallback uses
RG8_SNORM / RG8_UNORM with rounded quantization. A color/sRGB request refuses
BC5 explicitly instead of inventing a gamma mapping. Existing formats retain
their existing paths.

[documented: pinned public source] At 7201ca6d57d3bb8a435bffc3fc7a5ecedf79dea3, NIF normal slot 1 calls texture(false), as do water
height/noise callers. The old cache keys only (path, sRGB). Vendored Bevy treats
unsigned RG formats as XY normals, reads XY*2-1, and computes unclamped
sqrt(1-x*x-y*y). NIF refraction also reads RG*2-1. Raw signed BC5 therefore
cannot be substituted for that normal encoding. Sources: nvf-assets texture.rs,
nif_spawn.rs, model.rs; nvforever water.rs; vendor/bevy_pbr pbr_material.rs and
render/pbr_functions.wesl at that commit:
https://github.com/LootJunkieAzza/NVForever/tree/7201ca6d57d3bb8a435bffc3fc7a5ecedf79dea3/engine

[hypothesis/engineering policy] Introduce Color, Linear and Normal texture roles
in every cache, worker, failure, snapshot, gate and release key. Preserve the
old bool API as Color/Linear. Only NIF's actual normal slot selects Normal in
this leaf; water stays Linear. For BC5 Normal, decode signed XY (or unsigned
XY*2-1), derive positive Z with sqrt(max(0, 1-x*x-y*y)), then bias and round XYZ
to RGBA8_UNORM. This uses the existing three-component normal path, avoiding a
vendor shader change and its unclamped two-component square root. Existing
NIF green flipping remains in the material. The synthesized alpha is storage,
not a specular mask: carry original no-alpha metadata with the loaded handle
through worker/snapshot/main adoption and material construction. Non-BC5 normal
formats keep their previous bytes, format and alpha classification. This is
an engine ingestion policy, not a claim about native filtering or GPU parity.

[documented: authored controls in `crates/fnv-dds/src/lib.rs` and
`engine/nvf-assets/src/{texture,model,nif_spawn}.rs`] Authored controls cover independent literal
endpoint/index palettes, signed ordering, partial block edges, mip/truncation
bounds, unsigned neighbors, role isolation, same-role worker deduplication,
release, original alpha metadata, and actual NIF material slot consumption.
No installed texture bytes or asset-specific substitutions enter this source.

Process qualification (separately retained authored CPU receipts, 2026-11-14): Historical
CPU qualification ran 18 DDS controls separately and, in a qualified
c28-based dependency graph, 173 asset controls (165 existing and 8 BC5). The
13 shared DDS baseline controls give 11 passes and 2 format-acceptance assertion
failures; the candidate passes all 18. Real play/app source metadata was also
checked without linking a normal client. Those historical contexts are separate
from the hosted qualification below.

Process qualification (hosted run, 2026-11-15): [CI run 43665568999](https://github.com/LootJunkieAzza/NVForever/actions/runs/43665568999)
executed checkout `e03841213cf49fe3e1fd5b33b04398a6d78678b8`, tree
`46546106bb8ec2a17a66afd753b39e4c81e3c524`, the tested merge of main
`78408bcba623baaabbab4bb77a0e19d3a5e4fd72` and BC5 head
`5c967f149199d0bd929c8d0f1f2bcf26ae8ffab0`. Core reported 3,024 passed and
4 ignored; engine reported 2,916 passed, with no test failures. All 13 authored
BC5 controls passed exactly once. Both strict lint checks and dependency audits
passed. These results identify that checkout and run, not later revisions.
Later revisions need their own source-bound qualification. The authored rendered
field and its separately pinned Windows result are described below; native
pixel/filtering parity and device-loss causes remain unqualified.

## Authored rendered material field

[documented: source consumers] `nif_spawn::shape_material` selects the Normal
role through `StoreTextures` and generates tangents from mesh positions/UVs.
The normal map reaches the existing `NifMaterial` shader. The default `Props`
has no specular mask, so the reference image's alpha and BC5's original-no-alpha
metadata do not change this field's lighting. `fnv_vfs::Vfs::open` mounts loose
files under `textures/`; files at the root do not enter its asset namespace.

[hypothesis: engineering policy] `NVF_PLAY_TEST_BC5_FIELD=1` installs the field
in the existing normal material plugin. It requires an explicit QA
`NVFOREVER_HOME`, creates a new `bc5-field-output` directory, and refuses to
reuse it. Source literals generate four authored 8x8 DDS files: signed BC5S
endpoint blocks (64,-64), (-64,64), (96,0), (0,-96); independent RGBA reference
(192,63,217,255), (63,192,217,255), (224,128,211,255), (128,31,211,255);
flat (128,128,255,255); and white. Each signed block selects endpoint zero.
The reference tuples are fixed inputs, never production-decoder output. The
local VFS must return each exact payload before any field entity is created.
It has no installed archive, global texture-cache or missing-file fallback.

[hypothesis: engineering policy] Three copies of an authored two-triangle quad
use actual `shape_material` and `MeshMaterial3d<NifMaterial>` with white in
slot 0 and signed/reference/flat in slot 1. Their camera covers 6x2 world units
onto a 384x128 RGBA8_UNORM target. Render layer 31 is isolated, apart from the
normal plugin's existing zero-area transmission anchor. A foreign layer user
observed during any active update or at readback completion fails the field,
even when the received pixels otherwise pass. Only owned entities are retired.
Ambient is zero only
for this camera; exposure EV 0, directional illuminance 2 and light direction
normalize(0.25,0.35,1) are fixed. No shadows, tonemapping, dither, MSAA or
postprocess components are used. Normal world cameras and lights are unchanged.

[hypothesis: engineering policy] After 5 seconds and 10 render-schedule
completions with the extracted field camera and target, capture actual
`ReadbackComplete` bytes. Capture again at least 10 such submitted frames after
the first completion. This clock runs after render-graph submission, not on
main-loop ticks. The 30-second aid deadline fails incomplete captures. Four
24x24 regions start at panel-local (16,16), (72,8), (8,72), (88,88), away from
texture and triangle boundaries. Every signed/reference RGB difference must
be <= 1; reference/flat mean absolute RGB difference must be >= 8 in at least two
regions; every region's mean luminance must be strictly between 8 and 247;
all target alpha must be 255 and the readback exactly 196,608 bytes. Retain both
full raw files, PPM views and reports, including failed comparisons. These
predeclared thresholds are not adjusted to make a run pass.

Process: the `bc5-signed-normal-owned-field` normal GAME scenario uses an
explicit relative QA home under the smoke output directory and no environment
placeholder. Use a fresh `--out` directory for each run. Stock world bootstrap
remains read-only and separate from the wholly authored field. The scenario
requires two actual capture markers and the comparator result; shader/wgpu
errors fail. Review both complete images and record the reviewer role alongside
raw-file/source/input hashes. Assistant image inspection must not be described
as human inspection. The aid's CPU and launcher controls do not qualify rendered
pixels. No native filtering, arbitrary NIF parser, installed texture repair or
device-loss-cause claim follows from this field.


Process qualification (owned normal Windows field, 2026-11-15): the exact
integration `a0ce72c529be8b480705eb60138f5622e2f045b1`, tree
`8a9acbeb8ef99682835ce0ec84b439605f92d1c8`, built with normal default and
intro-video features. Executable SHA256
`22df96271d00b1e3cdc015e4d8ca5a373514cee347525dc85d1b0766b9a84ce1`
ran the registered scenario through the ordinary launcher on an RTX 3090,
Vulkan driver 621.24, using a read-only stock bootstrap of 10 masters and
21 archives. All field geometry and four DDS inputs were authored and verified
in the separate fresh owned VFS; no installed field assets were substituted.

Both complete 196,608-byte RGBA readbacks passed the fixed comparator at render
counts 130 and 143. Signed/reference maximum RGB differences were zero in all
four regions. Reference/flat mean differences were 1, 73, 19 and 10, exceeding
the fixed threshold in three regions. Every alpha was 255. Both full PPM/PNG
RGB streams matched the raw bytes. Two assistant image inspections confirmed
the matching four-quadrant panels and separate flat panel; human inspection is
not claimed. The normal process exited zero, shader/error guards passed and
all owned descendants ended. The separate actual audit is identified by SHA256
`98d717d69010e3dd667890936d2f1c1f617288c493322c67942c8f51c2fb269f`.

This is the authored material-field result in that exact normal integration,
not execution of a later public head or proof of native filtering, arbitrary
mod support, visible installed-texture repair, device-loss cause or FPS. Source
composition, CI, review and any acceptance requirements for later revisions
remain separate. Earlier failed preparation/admission attempts remain retained.
