# Billboard nodes of placed models (#2549)

How NVForever draws a `NiBillboardNode` of a model placed in the world, an effect model
or a model enabled by a script. Implemented in `engine/nvf-assets/src/model.rs`
(`Billboard`, `BakedBillboards`, `Rig::spawn_playing`) and `engine/nvforever/src/world.rs`
(`face_camera`).

## 1. What the file says

- `NiBillboardNode` is an `NiNode` with one field of its own, `Billboard Mode`
  (`BillboardMode`: ALWAYS_FACE_CAMERA 0, ROTATE_ABOUT_UP 1, RIGID_FACE_CAMERA 2,
  ALWAYS_FACE_CENTER 3, RIGID_FACE_CENTER 4, BSROTATE_ABOUT_UP 5, ROTATE_ABOUT_UP2 9)
  [documented: nif.xml NiBillboardNode, BillboardMode].
- A billboard node turns its model-space +z towards the camera ("ALWAYS_FACE_CAMERA …
  model-space z-vector", "RIGID_FACE_CENTER … z-vector to point at the center of the
  camera") [documented: GECK wiki NiBillboardNode, as quoted in issue #2549].
- Which axis a mode 1, 5 or 9 node turns about: its own +y [hypothesis, from
  `shot_fx.rs` before #2549; the flame of the torch below stands along +z, so this is
  not checked against a placed model].

## 2. The torch at Cottonwood Cove

`OasisTorch01` (MSTT 0x000BD477), model `meshes\clutter\oasis\oasistorch01.nif`, placed 5
times in WastelandNV, all at Cottonwood Cove (cells 15,-22; 15,-23; 14,-23 twice; 13,-24)
[observed: `fnvtool world refs --world WastelandNV`]. In the file [observed: `fnvtool nif
decode --schema schemas/nif`]:

- block 6 `GlowMesh`: `NiBillboardNode`, mode 4, translation (-0.09, 0.94, 106.34), no
  rotation. Its one shape `GlowMesh:0` (`NiTriStrips`) has 4 vertices at x ±61.2,
  y ±72.2, z 9.21, all normals +z: one 122 × 144 quad lying in the node's xy plane,
  facing +z. Texture `textures\effects\FXPartTinyGlowy.dds`, `NiAlphaProperty` flags
  4109, its `NiMaterialProperty` (emissive 1.0, 0.77, 0.40) driven by a controller.
- block 18 `FlameMesh`: `NiBillboardNode`, mode 5, at z 100.6, holding the flame shape
  (`FXFireScroll.dds`) and the smoke particle system.
- The file's string table names the exporter's settings `billboardRigidCenter` and
  `billboardBSUp` beside them [observed].

So the glow card is meant to face the viewer; drawn in the file's pose it is a
horizontal sheet 15 units above the flame base, seen nearly edge-on from standing
height: the line of the reports [observed: issue #2549 screenshots].

## 3. What NVForever did and does

Before #2549 only effect models (`shot_fx`: muzzle flashes, impacts, explosions) and
creature parts that play themselves turned their billboard nodes; a placed model drew its
billboard nodes in the file's pose, whether its node hierarchy was spawned (a model with
an idle or controllers of its own, as this torch: render streaming's animated path, a
script-enabled reference's rig) or its shapes were baked (the static path).

Now:

- Every rig node that is a billboard node is spawned with a `Billboard` (its mode and its
  rotation in the file) by `Rig::spawn_playing`, so every rig (placed, script-enabled,
  effect) has them.
- A model drawn with baked shapes puts each shape below a billboard node under an entity
  for that node (`BakedBillboards`), placed at the reference ∘ the node's parent from the
  root, the shape keeping its transform below the node. A billboard node below another
  one is turned alone, the outer one staying in the file's pose [design: no file looked
  at nests them]. A model drawn in a stored ragdoll pose (`XRGD`) keeps its baked shapes.
- `world::face_camera` turns every `Billboard` each frame in `PostUpdate` before
  transforms propagate, from that frame's transforms (the propagated ones are a frame
  old): modes 1, 5 and 9 about the node's y axis, the others with +z straight at the
  camera [documented: the mode names; hypothesis: the axes, §1].
- Render streaming logs `world: N placed billboard nodes turn to the camera (M
  references)` once the first cells are drawn.

Not done: the difference between the "rigid" and the other modes (whether the card also
keeps its up, or rolls freely) is not drawn; every non-up mode takes the shortest turn
from the file's +z [hypothesis].
