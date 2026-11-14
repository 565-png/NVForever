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

[observed: separately retained authored CPU receipts, 2026-11-14] Historical
CPU qualification ran 18 DDS controls separately and, in a qualified
c28-based dependency graph, 173 asset controls (165 existing and 8 BC5). The
13 shared DDS baseline controls give 11 passes and 2 format-acceptance assertion
failures; the candidate passes all 18. Real play/app source metadata was also
checked without linking a normal client. Those contexts do not qualify this
current-main port. Current whole-head build, strict lint, CI and rendered checks
are pending; no GPU, visible repair or device-loss cause is claimed.
