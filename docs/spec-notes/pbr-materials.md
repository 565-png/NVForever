# Authored PBR material maps (`textures\pbr\`)

NVForever's own file format for texture packs that give a surface physically based material
maps: occlusion, roughness and metallic, a normal map and a height map, beside the game's
diffuse texture (#2981). It is **NVForever's own format**: it describes no behaviour of the
original game, and no file the base game or its add-ons ship is known to lie under
`textures\pbr\` [hypothesis: not checked against the archives]. Every rule below is our
choice, marked **[design]**; facts about the texture formats cite their source
**[documented]**; what the engine does today cites its code.

Version 1. Pack authors may rely on the layout, member names, channels and scalar keys
below; a later version adds to them without changing their meaning.

## What the engine implements at this stage

| Part | State |
|---|---|
| Path rules, member names, the index of sets a load order supplies | **implemented** (`crates/fnv-pbr`; `GameData::pbr_index` in `engine/nvf-assets/src/data.rs` builds it on first use and logs one line of counts; nothing in the engine asks for it yet) |
| `fnvtool pbr list` (which sets a load order has, and which archive or folder supplies each file) | **implemented** (`crates/fnvtool/src/pbr_cmd.rs`, docs/cli-contract.md) |
| Scalar file reading (`<stem>.toml`) | **implemented** as a parser; no material reads it yet |
| Reading members' pixels | **planned**: `_orm` and `_n` in stage 3, `_p` in stage 4; `_albedo` stays reserved. This stage finds files by name only |
| Reading BC7 and BC4 files (DDS with the DX10 header, or legacy `BC4U` / `BC4S`) | stage 1 of #2981 (`crates/fnv-dds`) |
| Drawing a surface with its set (ORM, normal) | **planned** (stage 3): no material uses a set yet, so installing a pack changes no picture today |
| Height maps (`_p`, parallax) | **planned** (stage 4, with #2979) |
| Metallic | **planned**: read with the ORM but **held at 0** (stage 3) until the engine has a reflection source (stage 5) |
| A pack validator (`fnvtool pbr check`, `fnvtool pbr selftest`) | **implemented** (`crates/fnvtool/src/pbr_check.rs`, docs/cli-contract.md `fnv.pbr.check/1`) |

Sections marked "planned" describe how the engine will use a set once those stages land;
until then a set is only found by its files' names, counted and listed. Wherever this note
says a member "will be read" or "will be accepted", that is the planned material path.

## Layout

[design] Every file of a pack lives under `textures\pbr\`, in the same folders as the game
diffuse it belongs to. The set's **stem** is the diffuse's path with its extension removed
(whatever it is), placed under `textures\pbr\`:

| Diffuse the game draws | Stem |
|---|---|
| `textures\architecture\urban\sidewalkquad01.dds` | `textures\pbr\architecture\urban\sidewalkquad01` |
| `textures\clutter\food\tincan.tga` | `textures\pbr\clutter\food\tincan` |

A set's members are the stem plus one suffix:

| File | Role | Required |
|---|---|---|
| `<stem>_orm.dds` | red: ambient occlusion; green: perceptual roughness; blue: metallic | makes the set **authored** |
| `<stem>_n.dds` | tangent-space normal, two channels (X in red, Y in green), green pointing down like the game's normal maps [observed: docs/spec-notes/nif.md] | optional |
| `<stem>_p.dds` | height, brighter is higher | optional; alone it makes a **height-only** set |
| `<stem>.toml` | scalars (below) | optional |
| `<stem>_albedo.dds` | **reserved** for a later version: found and counted, never activates a set, will not be read in version 1 | — |

- [design] A set with `_orm` is authored. A set with `_p` and no `_orm` is valid: the game's
  own material with height only (when height maps land). A set with only `_n`, `.toml` or
  `_albedo` does nothing; the engine counts it as incomplete.
- [design] There is **no plain-name file**: `<stem>.dds` is not a member. In version 1 the
  base colour is always the game's own diffuse (or whatever diffuse replacer wins in the
  load order).
- [design] Paths are matched without regard to slash direction or ASCII case, as every
  game path is (`fnv_vfs::normalize`): only `A`-`Z` fold to `a`-`z`; other letters (accented
  or non-Latin) must match exactly. A diffuse path a mesh writes with a leading `data\` or an
  absolute folder is cut at `textures\` first, as for the diffuse itself
  (`fnv_vfs::texture_path`).
- [design] The extension is not part of the stem, so `foo.dds` and `foo.tga` in one folder
  share one set. That is accepted: a pack cannot give the two different sets.
- [design] The key is the diffuse **as the engine resolves it**: after record texture swaps,
  after the first-person twin is chosen, and after a console path (`\xbox\`) is replaced by
  the PC copy, which happens only when the PC diffuse exists (`resolved_path`,
  `engine/nvf-assets/src/texture.rs`). A set under `\pc\` therefore applies only where the PC
  diffuse is drawn.
- [design] **First person**: when the diffuse's file name starts with `1stperson` and there is
  no active set for it, the set of the same name without the prefix is used. One set
  serves a weapon in both views; a pack can still give the first-person texture its own.

### Reading a file name back: one suffix, once

[design] A file under `textures\pbr\` is assigned to a set by removing **exactly one**
suffix from the end of its file name (the last path component; folder names are never
peeled): `_orm.dds`, `_n.dds`, `_p.dds`, `_albedo.dds` or `.toml`, compared after ASCII case
folding. What is left must be a non-empty name; it is the stem. The suffix is never
removed twice:

| File under `textures\pbr\walls\` | Set (diffuse) | Member |
|---|---|---|
| `foo_n.dds` | `foo` | normal |
| `foo_n_orm.dds` | `foo_n` (a diffuse named `foo_n.dds`) | ORM |
| `foo_n_n.dds` | `foo_n` | normal |
| `foo.dds` | none: **unmatched** | — |
| `foo_orm.png`, `readme.txt` | none: unmatched | — |

Unmatched files are ignored and counted (`fnvtool pbr list` names them).

## Channels and colour spaces

[design] The colour space follows the member's **role**, not the DDS header: every member
of version 1 (`_orm`, `_n`, `_p`) will be read as linear data, even when a DX10 header names
an `_SRGB` format. Authors should still export them as linear (non-sRGB) so other tools show
them correctly.

- `_orm`: red occlusion (1 = unoccluded), green perceptual roughness (0 smooth, 1 rough),
  blue metallic (0 dielectric, 1 metal). The alpha channel will not be read.
- `_n`: X in red, Y in green, each 0..1 standing for -1..1; Z is rebuilt from them. Green
  follows the game's convention, +G pointing down the image [observed: docs/spec-notes/nif.md,
  "Normal maps", from the game's normal and height map pairs], which the engine follows by
  flipping Y for every NIF normal map (`flip_normal_map_y` in
  `engine/nvf-assets/src/nif_spawn.rs`), so a pack can start from the game's normal maps.
  A three-channel normal map (BC7, BC1, BC3) will be accepted and read as X, Y, Z
  (stage 3); its alpha will not be read.
- `_p`: one channel, red; brighter is higher.

## Compression and sizes

- Recommended formats: `_orm` **BC7** (or BC1 when its quality is enough), `_n` **BC5**
  (unsigned), `_p` **BC4** (unsigned). BC1 and BC3 will be accepted for `_orm` and `_n`
  from stage 3, and for `_p` when height maps land (stage 4); `_albedo` stays reserved.
  [documented: Microsoft, "Texture Block Compression in Direct3D 11": BC4 stores one channel
  and BC5 two in 4x4-texel blocks of 8 and 16 bytes; BC7 stores RGB or RGBA in 16-byte
  blocks]
  https://learn.microsoft.com/en-us/windows/win32/direct3d11/texture-block-compression-in-direct3d-11
- BC7 files carry the DDS DX10 extension header [documented: Microsoft,
  "DDS_HEADER_DXT10"]. BC4 and BC5 files may carry it too, or a legacy header with the
  FourCC `BC4U` / `BC4S` (BC4) or `ATI2` / `BC5S` (BC5) [documented: Microsoft,
  "Programming Guide for DDS"; docs/spec-notes/dds.md, docs/spec-notes/dds-bc5.md]. The
  engine reads the DX10 header and the BC4 FourCCs from stage 1 of #2981 on.
- [design] The **top level's width and height must be multiples of 4**. The engine has no CPU
  decoder for BC7, so a file it cannot upload whole will be refused rather than decoded.
- [design] Ship **full mip chains**, down to 1x1. Smaller mip levels need not be multiples of 4.
- [design] Each member's size is its own: an ORM at 2048 beside a 1024 diffuse and a 4096
  normal map is fine. Nothing ties the sizes together.
- Before shipping, run `fnvtool pbr check <folder>` on the pack (the mod folder, its
  `textures` folder or the `pbr` folder). It reads each file's DDS headers and size and the
  scalar files: formats per member, sides that are multiples of 4, mip chains, cut-off or
  padded files, `_SRGB` codes, incomplete sets and unmatched files, and it estimates the
  pack's GPU memory; it exits non-zero on an error. It does not decode or judge the maps'
  content (whether the roughness or normals look right): view them in the engine.

## Scalars: `<stem>.toml`

A TOML file of plain keys, all optional:

| Key | Meaning | Range | Default |
|---|---|---|---|
| `parallax_depth` | depth of the height map's darkest point below the surface, in UV units (planned, stage 4) | 0 to 0.25 | the engine's own depth |
| `roughness_scale` | multiplies the ORM's roughness; the product is then clamped to 0.089..1 when drawn (below) | 0 to 2 | 1.0 |
| `reflectance` | the highlight strength of non-metals; F0 = 0.16 x reflectance² (0 to 16 %) | 0 to 1 | 0.5 (F0 4 %) |

```toml
# textures\pbr\walls\brick07.toml
roughness_scale = 0.9
reflectance = 0.35
```

[design] Reading is lenient (`fnv_pbr::parse_scalars`): an integer counts as a number; an
unknown key, a table, or a value that is not a finite number is ignored with a warning; a
number outside its range is clamped into it with a warning; a file that is not UTF-8 or
not TOML is ignored whole with a warning. A UTF-8 byte order mark is accepted.
`normal_strength` is not a key: bake the strength into the normal map.

## What a set changes on a surface (planned, stage 3)

[design] When the engine draws a surface whose diffuse has an authored set:

- **Switched off**: the shape's glossiness, the normal map's alpha as a highlight mask, and the
  shape's Specular flag. Roughness comes from the ORM (times `roughness_scale`), the highlight
  strength from `reflectance`. A surface the game drew without highlights therefore gets the
  default 4 % F0 once authored; `reflectance = 0` keeps it matte.
- **Replaced**: the normal map, when the set has `_n`; otherwise the shape's own normal map
  stays, without its alpha mask.
- **Kept**: the base colour (the diffuse), the glow map, alpha blending and alpha testing (from
  the diffuse's alpha, as before), the decal flag, and environment maps once the engine draws
  them (it does not yet: `engine/nvf-assets/src/nif_material.wesl` header).
- **Not covered in version 1**: actor skin, hair and FaceGen heads; terrain layers; distant
  LOD; effects (unlit, additive, refraction, particles); the Pip-Boy and interface textures.

## How the engine lights a set (planned use, today's lighting)

Maps tuned for other renderers need these notes; the lighting itself is the engine's today,
the use of the maps is planned:

- The diffuse light follows the engine's model of the original's: lights are summed in
  display values and the diffuse is multiplied by that sum raised to 2.2, not by a linear
  light sum (`engine/nvforever/src/lights/display_shader.rs`). That the original lights
  surfaces this way is itself a [hypothesis, fitted to the original's frames]
  (docs/spec-notes/world.md, "Light on surfaces in display values").
- Ambient occlusion (ORM red) darkens only the ambient part of that sum, not the sun or
  lamps (`display_shader.rs`, `diffuse_occlusion`).
- The highlight is GGX with the Fresnel factor held at F0 in world views (no brightening at
  grazing angles; `FRESNEL_EDIT`, `engine/nvforever/src/lights.rs`).
- Roughness is perceptual: the ORM's green is multiplied by `roughness_scale`, and that
  product is clamped to 0.089..1 and then squared (Bevy's `perceptualRoughnessToRoughness`,
  `engine/vendor/bevy_pbr/src/render/pbr_lighting.wesl`). A product below 0.089 draws as
  0.089; a raw green below 0.089 can still end above the floor with a scale above 1.
- **Metallic** will be read but **held at 0** in version 1: with no reflection source in the
  engine, a metal would draw black except for lamp highlights. Until a later version adds
  one, a metal surface draws as a dielectric of its diffuse colour with the authored
  roughness. Author the blue channel correctly anyway; it will be used.

## Load order: per file, so ship complete sets

[design] Each file resolves on its own through the virtual file system, like every other
texture: plugin archives in load order, then `Data`'s loose files, then NVForever's mods
folder or a Mod Organizer 2 profile's mods (docs/design/mod-folders.md). Two packs can
therefore mix, one's `_orm` with another's `_n`. **Ship complete sets**, and use
`fnvtool pbr list` to see which pack supplies each file.

What is true of archives today (`Vfs::archive_order_in`, `Vfs::archive_files_layered`,
`crates/fnv-vfs/src/lib.rs`):

- Loose files always load, from `Data`, the mods folder or a Mod Organizer 2 mod.
- A pack in a `.bsa` is mounted only when the archive belongs to an **active plugin** by
  name (`<plugin>.bsa`, `<plugin> - <anything>.bsa`, or its numbered pieces), lying in
  `Data`, in NVForever's mods folder or in a Mod Organizer 2 mod; when the archive list
  names it (`[Archive] SArchiveList` in NVForever.ini, for an archive in `Data`; or a Mod
  Organizer 2 profile's ticked archives, in any of its layers); or when it is `Update.bsa`,
  which is mounted after the base list (from `Data`, or under Mod Organizer 2 from the
  highest layer that has it) unless the base list names it, which keeps it at that place,
  or the `NVF_ARCHIVE_ORDER` override sets the whole order without it. A plugin that cannot
  be read or lacks a master is left out, and the archives it owns by name with it; an
  archive list that names one of those archives still mounts it. A lone `.bsa` that none
  of these covers is not mounted.

## Log line and listing

The first time the index is asked for, the engine logs one line, e.g.
`authored materials: 12 sets under textures\pbr\ (11 with _orm, 1 height only; files: 11 _orm,
9 _n, 3 _p, 2 .toml, 0 _albedo reserved), 1 incomplete sets and 2 unmatched files ignored`.
`fnvtool pbr list --data <Data> [plugins…] [--mods <folder> | --mo2 <instance>]` prints
every set with its state and, per member, the archive or loose folder that wins
(`fnv.pbr.listing/1`, docs/cli-contract.md).
