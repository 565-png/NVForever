# The Anh NVSE plugin's functions (2026-10-30, #1015)

Anh's script functions as NVForever builds them: the pure ones in the script VM
(`crates/fnv-script/src/vm/anh.rs`), the ones on the game in `crates/fnv-game/src/
nvse_anh.rs`. Sources: the GECK wiki export [documented: the page named] and black-box
script probes in Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10
official base masters (`[observed]`, no DLL or source consulted). Unprobed details remain
`[hypothesis]`, and deliberate crash-avoidance fallbacks are marked `[design]`.

## 1. Arrays, angles and strings

| Function | NVForever |
|---|---|
| Ar_Max, Ar_Min, Ar_RangeMath `array` | the largest, the smallest and their difference of the array's numbers; other elements left out [documented: "*numeric*" values; observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probe anh_01_ar_max_min_range_empty (Ar_Max/Ar_Min/Ar_RangeMath): [6,2,10,7] -> 10.00, 2.00, 8.00; empty or string-only arrays crash AnhNVSE 131 with 0xC0000005; no DLL or source consulted]; NVForever safely returns 0 when the array has no numbers [design] |
| DegToRad, RadToDeg `angle` | [documented] |
| GetAngleQuadrant `degrees` | 1-4 for an angle in **degrees**, truncated to integer degrees modulo 360 (`[0, 90) -> 1`, `[90, 180) -> 2`, `[180, 270) -> 3`, `[270, 360) -> 4`) [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probe anh_02_angle_quadrant_edges (GetAngleQuadrant): 0, 1.5708, 3.1416, 4.7124, -0.5, 45 -> 1; 90, 135 -> 2; 180, 225 -> 3; 270, 315, -45 -> 4; no DLL or source consulted] |
| Sv_Capitalize `text` | the first character upper case [documented: "pepeglare snig" gives "P…"] |
| Sv_PadStart, Sv_PadEnd `text length pad` | the pad repeated, cut to fit, before or after the text until it is `length` long; a text already as long, or an empty pad, unchanged [documented] |
| ScancodeToChar `code` | uppercase letters and DirectInput key names (`1="Escape"`, `18="E"`, `30="A"`, `59="F1"`) [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probe anh_03_scancode_to_char_layout (ScancodeToChar): 1=[Escape], 59=[F1], 18=[E], 30=[A]; no DLL or source consulted]; other keyboard scan codes `2..=220` reuse `keys::KEY_NAMES` and unmapped codes return `""` [hypothesis] |

## 2. Vectors and quaternions (arrays)

Vector3s are three-number arrays and quaternions `[w, x, y, z]` [documented: QNormalizeAlt;
observed: anh_09_q_axis_angle_zero_order, anh_05_q_axis_angle_and_mult_vec3].
Results are new arrays.

| Function | NVForever |
|---|---|
| V3Mult, V3MultByScalar | element by element; by the number [documented: (1,2,3)·(4,5,6) = (4,10,18); (1,2,3)×2 = (2,4,6)] |
| V3Dotproduct | a number [documented: "the dot product"; the page's return type says array] |
| V3NormalizeAlt, V3CrossproductAlt | unit length; A×B [documented: "the crossproduct of vectorA and vectorB"] |
| V3Lerp `a b t` | a + (b − a)·t on two 3-element arrays without clamping `t` [documented: "linearly interpolates between two points"; observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probes anh_04_v3lerp_arrays ([0,0,0]..[10,20,30] at 0.5 -> 3, 5.00, 10.00, 15.00) and anh_10_v3lerp_extrapolate (t=2 -> 20.00, t=-1 -> -10.00); no DLL or source consulted] |
| GenerateBezierPoint `p0 p1 p2 t` | the quadratic Bézier point (1−t)²p0 + 2(1−t)t·p1 + t²p2 [documented: "a Bezier curve created by 3 points"] |
| QNormalizeAlt | unit length [documented] |
| QMultQuatQuatAlt `a b` | the Hamilton product A·B [documented: "the product of quatA and quatB"] |
| QMultQuatVector3Alt `q v` | returns a 3-element vector3 [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probe anh_05_q_axis_angle_and_mult_vec3 (QMultQuatVector3Alt): size 3; note probe also observed rot=(1.000,0.000,0.000) for q=(0.707,0,0,0.707) on [1,0,0]; no DLL or source consulted]; v turned by q (q·v·q⁻¹) remains [hypothesis] |
| QFromAxisAngleAlt `axis angle` | the angle in **degrees**, returning `[w, x, y, z]` [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probes anh_09_q_axis_angle_zero_order (0 -> 1.000,0.000,0.000,0.000) and anh_05_q_axis_angle_and_mult_vec3 (1.570796 -> w=1.000,z=0.014; 90 around [0,0,1] -> 0.707,0.000,0.000,0.707); no DLL or source consulted] |
| QInterpolateAlt `a b ratio [spherical]` | normalised linear by default, spherical with the fourth argument, the shorter way round [documented: the two methods; hypothesis: the shorter way] |

xNVSE's own forms (QFromAxisAngle …) are different functions with observed conventions
(B·A, the conjugate turn, degrees; `vm/quat.rs`); Anh's are not assumed to share them.

## 3. On the game

| Function | NVForever |
|---|---|
| GetArmorArmorRatingAlt `[armor]` (GetArmorARAlt), SetArmorArmorRatingAlt `dr [armor]` (SetArmorARAlt) | as GetArmorArmorRating and SetArmorArmorRating: the argument, else the calling reference's base [documented] |
| GetQuestCompletedAlt `quest` | 1 when completed and not failed [documented] |
| GetZoneLevel `zone` | the level the zone took when its first cell loaded (encounter-zones.md); 0 before [documented: "the current level"; observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probe anh_07_zone_level_min_owner (GetZoneLevel/SetZoneMinLevel/SetZoneOwner): 0,6,12; no DLL or source consulted] |
| GetZoneMinLevel, GetZoneMatchLevel, GetZoneOwner `zone` | the ECZN record's minimum level, "Match PC Below Minimum Level" flag and owner [documented] |
| SetZoneMinLevel `zone level`, SetZoneOwner `zone owner` | round-trips through `GetZoneMinLevel` / `GetZoneOwner` [documented: the pages' examples; observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + AnhNVSE 131 with the 10 official base masters, clean-room script probes anh_07_zone_level_min_owner (0,6,12) and anh_11_zone_owner_roundtrip (ok); no DLL or source consulted]; session-only lifetime (carried across same-run loads via `WorldSession` / `Game::Session`, not saved) and keeping an already-entered zone's level remain [hypothesis] |

## 4. The last three (2026-11-01)

ArrayToString (the VM), GetCameraFOV and UpdateTubeMesh (`nvse_small.rs`): ext-small.md
§1.
