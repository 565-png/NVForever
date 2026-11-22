//! Synthetic clean-room round-trip & edge tests for `.fos` -> `.nvfsave` conversion.
use crate::common::*;
use fnv_esm::{LoadOrder, Plugin, Schema};
use fnv_game::fos_convert::{convert_fos, inspect_fos, ConvertOptions};
use fnv_game::game::{Game, GameConfig, GameSnapshot, InstKey};
use fnv_game::GameData;
use fnv_save::fos::{
    Fos, FORM_DELETED, FORM_DISABLED, FORM_TALKED_TO_PC, PLAYER_BASE, PLAYER_CLASS, PLAYER_REF,
    TYPE_ACHR, TYPE_CHAL, TYPE_CLAS, TYPE_FACT, TYPE_FLST, TYPE_INFO, TYPE_LVLI, TYPE_NOTE,
    TYPE_NPC, TYPE_QUST, TYPE_REFR, TYPE_REPU,
};
use fnv_save::{PluginEntry, SaveBody, SaveHeader, SaveKind, BODY_FORMAT, ENGINE_FORMAT};
use fnv_script::vm::Value;
use fnv_script::FunctionTable;
use fnv_sim::{FormId, FormStore, RefTable, World};

const BAR: u8 = 0x7C;
const GLOB_HOUR: u32 = 0x0000_0038;
const GLOB_DAYS: u32 = 0x0000_0039;
const GLOB_CUSTOM: u32 = 0x0000_0801;
const QUEST_ID: u32 = 0x0000_0810;
const SCRIPT_ID: u32 = 0x0000_0811;
const ITEM_ID: u32 = 0x0000_0820;
const ITEM_WORN_ID: u32 = 0x0000_0821;
const ROOM_ID: u32 = 0x0000_0830;
const PROP_REF: u32 = 0x0000_0831;
const NPC_BASE: u32 = 0x0000_0832;
const NPC_REF: u32 = 0x0000_0833;
const FACT_A: u32 = 0x0000_0840;
const FACT_B: u32 = 0x0000_0841;
const FLST_ID: u32 = 0x0000_0850;
const LVLI_ID: u32 = 0x0000_0851;
const REPU_ID: u32 = 0x0000_0860;
const CHAL_ID: u32 = 0x0000_0861;
const NOTE_ID: u32 = 0x0000_0870;
const DIAL_ID: u32 = 0x0000_0880;
const INFO_ID: u32 = 0x0000_0881;
const WTHR_ID: u32 = 0x0000_0890;

struct Fixture {
    data: GameData,
    forms: FormStore,
    refs: RefTable,
}

fn fixture() -> Fixture {
    let glob = |id: u32, edid: &str, val: f32| {
        record(
            b"GLOB",
            0,
            id,
            &[
                sub(b"EDID", z(edid)),
                sub(b"FNAM", vec![b'f']),
                sub(b"FLTV", val.to_le_bytes().to_vec()),
            ],
        )
    };
    let globals = [
        glob(0x0000_0035, "GameYear", 2281.0),
        glob(0x0000_0036, "GameMonth", 9.0),
        glob(0x0000_0037, "GameDay", 19.0),
        glob(GLOB_HOUR, "GameHour", 8.0),
        glob(GLOB_DAYS, "GameDaysPassed", 1.0),
        glob(GLOB_CUSTOM, "OwnTestGlobal", 0.0),
    ]
    .concat();
    let mut scpt_subs = vec![sub(b"EDID", z("OwnQuestScript"))];
    scpt_subs.extend(unit(
        1,
        &script(&[(0, None, Vec::new())]),
        &[(1, "fCount", 0), (2, "rTarget", 0)],
        &[],
    ));
    let script = record(b"SCPT", 0, SCRIPT_ID, &scpt_subs);
    let quest = record(
        b"QUST",
        0,
        QUEST_ID,
        &[
            sub(b"EDID", z("OwnTestQuest")),
            sub(b"SCRI", SCRIPT_ID.to_le_bytes().to_vec()),
        ],
    );
    let item = record(
        b"MISC",
        0,
        ITEM_ID,
        &[
            sub(b"EDID", z("OwnTestMisc")),
            sub(b"DATA", [5i32.to_le_bytes(), 1.0f32.to_le_bytes()].concat()),
        ],
    );
    // Synthetic armor with health = 100 and biped slot upper body so Equip works cleanly.
    let armor = record(
        b"ARMO",
        0,
        ITEM_WORN_ID,
        &[
            sub(b"EDID", z("OwnTestSuit")),
            sub(b"BMDT", [4u32.to_le_bytes().as_slice(), &[0, 0, 0, 0]].concat()),
            sub(
                b"DATA",
                [
                    50i32.to_le_bytes().as_slice(),
                    &100i32.to_le_bytes(),
                    &2.0f32.to_le_bytes(),
                ]
                .concat(),
            ),
        ],
    );
    let npc_player = record(b"NPC_", 0, PLAYER_BASE, &[sub(b"EDID", z("Player"))]);
    let npc_other = record(b"NPC_", 0, NPC_BASE, &[sub(b"EDID", z("OwnNpcBase"))]);
    let fact_a = record(b"FACT", 0, FACT_A, &[sub(b"EDID", z("OwnFactA"))]);
    let fact_b = record(b"FACT", 0, FACT_B, &[sub(b"EDID", z("OwnFactB"))]);
    let flst = record(b"FLST", 0, FLST_ID, &[sub(b"EDID", z("OwnFormList"))]);
    let lvli = record(b"LVLI", 0, LVLI_ID, &[sub(b"EDID", z("OwnLeveledList"))]);
    let repu = record(b"REPU", 0, REPU_ID, &[sub(b"EDID", z("OwnReputation"))]);
    let chal = record(
        b"CHAL",
        0,
        CHAL_ID,
        &[
            sub(b"EDID", z("OwnChallenge")),
            sub(
                b"DATA",
                [
                    13u32.to_le_bytes().as_slice(), // type: scripted
                    &5u32.to_le_bytes(),            // threshold: 5
                    &0u32.to_le_bytes(),            // flags
                    &1u32.to_le_bytes(),            // interval
                    &0u32.to_le_bytes(),
                    &0u32.to_le_bytes(),
                ]
                .concat(),
            ),
        ],
    );
    let note = record(b"NOTE", 0, NOTE_ID, &[sub(b"EDID", z("OwnNote"))]);
    let dial = record(b"DIAL", 0, DIAL_ID, &[sub(b"EDID", z("OwnTopic"))]);
    let info = record(
        b"INFO",
        0,
        INFO_ID,
        &[sub(b"DATA", vec![0, 0, 0, 0x04])], // say_once flag
    );
    let dial_grp = [dial, group(DIAL_ID.to_le_bytes(), 7, &info)].concat();
    let wthr = record(b"WTHR", 0, WTHR_ID, &[sub(b"EDID", z("OwnClearWeather"))]);

    let prop_ref = record(
        b"REFR",
        0,
        PROP_REF,
        &[
            sub(b"EDID", z("OwnPropRef")),
            sub(b"NAME", ITEM_ID.to_le_bytes().to_vec()),
            sub(b"DATA", vec![0; 24]),
        ],
    );
    let npc_ref = record(
        b"ACHR",
        0,
        NPC_REF,
        &[
            sub(b"EDID", z("OwnNpcRef")),
            sub(b"NAME", NPC_BASE.to_le_bytes().to_vec()),
            sub(b"DATA", vec![0; 24]),
        ],
    );
    let player_ref = record(
        b"ACHR",
        0,
        PLAYER_REF,
        &[
            sub(b"EDID", z("PlayerRef")),
            sub(b"NAME", PLAYER_BASE.to_le_bytes().to_vec()),
            sub(b"DATA", vec![0; 24]),
        ],
    );
    let room = record(
        b"CELL",
        0,
        ROOM_ID,
        &[sub(b"EDID", z("OwnTestRoom")), sub(b"DATA", vec![1])],
    );
    let children = group(
        ROOM_ID.to_le_bytes(),
        6,
        &group(
            ROOM_ID.to_le_bytes(),
            9,
            &[prop_ref, npc_ref, player_ref].concat(),
        ),
    );
    let bytes = [
        tes4(),
        top(b"GLOB", &globals),
        top(b"SCPT", &script),
        top(b"QUST", &quest),
        top(b"MISC", &item),
        top(b"ARMO", &armor),
        top(b"NPC_", &[npc_player, npc_other].concat()),
        top(b"FACT", &[fact_a, fact_b].concat()),
        top(b"FLST", &flst),
        top(b"LVLI", &lvli),
        top(b"REPU", &repu),
        top(b"CHAL", &chal),
        top(b"NOTE", &note),
        top(b"DIAL", &dial_grp),
        top(b"WTHR", &wthr),
        top(
            b"CELL",
            &group(
                0i32.to_le_bytes(),
                2,
                &group(0i32.to_le_bytes(), 3, &[room, children].concat()),
            ),
        ),
    ]
    .concat();
    let plugins = vec![Plugin::parse(&bytes, "FalloutNV.esm".into()).unwrap()];
    let order = LoadOrder::new(&plugins).unwrap();
    let schema =
        Schema::load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/fnv")).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/script/functions.json");
    let catalogue = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let forms = FormStore::build(&order, &schema);
    let refs = RefTable::build(&order, &schema);
    let data = GameData::build(
        &order,
        &schema,
        FunctionTable::load(&path).unwrap(),
        &catalogue,
        &refs,
    );
    Fixture { data, forms, refs }
}

struct Build {
    b: Vec<u8>,
}

impl Build {
    fn bar(&mut self) -> &mut Self {
        self.b.push(BAR);
        self
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.b.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.b.extend(v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.b.extend(v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.b.extend(v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.b.extend(v.to_le_bytes());
        self
    }
    fn f64(&mut self, v: f64) -> &mut Self {
        self.b.extend(v.to_le_bytes());
        self
    }
    fn s_field(&mut self, v: &str, trailing_bar: bool) -> &mut Self {
        self.u16(v.len() as u16).bar();
        self.b.extend(v.as_bytes());
        if trailing_bar && !v.is_empty() {
            self.bar();
        }
        self
    }
    fn r(&mut self, idx_1based: u32) -> &mut Self {
        self.b.extend(&idx_1based.to_be_bytes()[1..]);
        self
    }
    fn vs(&mut self, n: u32) -> &mut Self {
        assert!(n < 64);
        self.u8((n << 2) as u8)
    }
}

fn push_record(out: &mut Vec<u8>, r: u32, flags: u32, kind: u8, data: &[u8]) {
    out.extend(&r.to_be_bytes()[1..]);
    out.extend(flags.to_le_bytes());
    out.push(kind | 0x40);
    out.push(27);
    out.extend((data.len() as u16).to_le_bytes());
    out.extend(data);
}

fn synthetic_fos_bytes() -> Vec<u8> {
    // 1-based form ID array (index 1..=20)
    let fids = [
        GLOB_HOUR,           // 1
        GLOB_DAYS,           // 2
        GLOB_CUSTOM,         // 3
        QUEST_ID,            // 4
        ITEM_ID,             // 5
        ITEM_WORN_ID,        // 6
        ROOM_ID,             // 7
        PROP_REF,            // 8
        NPC_REF,             // 9
        FACT_A,              // 10
        FACT_B,              // 11
        FLST_ID,             // 12
        LVLI_ID,             // 13
        REPU_ID,             // 14
        CHAL_ID,             // 15
        NOTE_ID,             // 16
        INFO_ID,             // 17
        WTHR_ID,             // 18
        PLAYER_REF,          // 19
        PLAYER_BASE,         // 20
        PLAYER_CLASS,        // 21
        0x0100_0999,         // 22: form from an unloaded plugin
        0xFF00_0042,         // 23: created 0xFF form
    ];

    let mut h = Build { b: Vec::new() };
    h.u32(0x30).bar();
    h.b.extend([0u8; 64]);
    h.bar()
        .u32(2)
        .bar()
        .u32(2)
        .bar()
        .u32(7)
        .bar()
        .s_field("Synthetic", true)
        .s_field("Wanderer", true)
        .u32(4)
        .bar()
        .s_field("OwnTestRoom", true)
        .s_field("002.15.30", false);

    let mut b = b"FO3SAVEGAME".to_vec();
    b.extend((h.b.len() as u32).to_le_bytes());
    b.extend(&h.b);
    // 2x2 RGB screenshot (12 bytes)
    b.extend([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);

    let mut pl = Build { b: Vec::new() };
    pl.u8(2).bar().s_field("FalloutNV.esm", true).s_field("MissingMod.esp", true);
    b.push(27);
    b.extend((pl.b.len() as u32).to_le_bytes());
    b.extend(&pl.b);

    let table_at = b.len();
    b.extend([0u8; 32 + 78]);

    let g1 = b.len();
    // Global type 0: misc stats (index 0 = 3, index 2 = 11)
    let mut t0 = Build { b: Vec::new() };
    t0.u32(3).bar().u32(3).bar().u32(0).bar().u32(11).bar();
    b.extend(0u32.to_le_bytes());
    b.extend((t0.b.len() as u32).to_le_bytes());
    b.extend(&t0.b);

    // Global type 1: location & active quest (QUEST_ID = r(4))
    let mut t1 = Build { b: Vec::new() };
    t1.u32(0xFF00_0050)
        .bar()
        .r(0)
        .bar()
        .i32(0)
        .bar()
        .i32(0)
        .bar()
        .r(7)
        .bar()
        .f32(10.0)
        .f32(20.0)
        .f32(30.0)
        .bar()
        .r(4)
        .bar();
    b.extend(1u32.to_le_bytes());
    b.extend((t1.b.len() as u32).to_le_bytes());
    b.extend(&t1.b);

    // Global type 3: 5 globals (3 valid, 1 unloaded plugin, 1 created 0xFF)
    let mut t3 = Build { b: Vec::new() };
    t3.vs(5)
        .bar()
        .r(1)
        .bar()
        .f32(14.5)
        .bar()
        .r(2)
        .bar()
        .f32(9.25)
        .bar()
        .r(3)
        .bar()
        .f32(42.0)
        .bar()
        .r(22)
        .bar()
        .f32(1.0)
        .bar()
        .r(23)
        .bar()
        .f32(2.0)
        .bar();
    b.extend(3u32.to_le_bytes());
    b.extend((t3.b.len() as u32).to_le_bytes());
    b.extend(&t3.b);

    // Global type 8: weather (WTHR_ID = r(18))
    let mut t8 = Build { b: Vec::new() };
    t8.r(18).bar().r(0).bar();
    b.extend(8u32.to_le_bytes());
    b.extend((t8.b.len() as u32).to_le_bytes());
    b.extend(&t8.b);

    // Global type 6: unmapped global table entry (should be counted in skipped)
    b.extend(6u32.to_le_bytes());
    b.extend(4u32.to_le_bytes());
    b.extend([1, 2, 3, 4]);

    let cf = b.len();
    // 1. QUST (QUEST_ID = r(4)): running, delay=2.5, stage 20 done, vars (1=7.5, 2=NPC_REF), obj 10 displayed+completed
    let mut q = Build { b: Vec::new() };
    q.u8(0x01).bar().f32(2.5).bar();
    q.vs(1).bar().u8(20).bar().u8(1).bar().vs(0).bar();
    q.vs(2)
        .bar()
        .u32(1)
        .bar()
        .f64(7.5)
        .bar()
        .u32(0x8000_0002)
        .bar()
        .r(9)
        .bar()
        .vs(0)
        .bar()
        .vs(0)
        .bar();
    q.vs(1).bar().u32(10).bar().u32(3).bar();
    push_record(&mut b, 4, (1 << 1) | (1 << 2) | (1 << 31) | (1 << 30) | (1 << 29), TYPE_QUST, &q.b);

    // 2. NPC_ player base (r(20)): SPECIAL
    let mut npc = Build { b: Vec::new() };
    npc.b.extend([6, 7, 5, 4, 8, 9, 10]);
    npc.bar();
    push_record(&mut b, 20, 1 << 2, TYPE_NPC, &npc.b);

    // 3. CLAS PlayerClass (r(21)): tag skills 38, 43, 44
    let mut clas = Build { b: Vec::new() };
    clas.u32(38).bar().u32(43).bar().u32(44).bar().u32(u32::MAX).bar();
    push_record(&mut b, 21, 1 << 1, TYPE_CLAS, &clas.b);

    // 4. Player ACHR (r(19)): place, 232 AV floats (AV2: karma=250.0, xp=1200.0), 2 items (1 worn with condition 50/100)
    let mut pl_rec = Build { b: Vec::new() };
    pl_rec.r(7).f32(12.0).f32(34.0).f32(56.0).f32(0.1).f32(0.0).f32(1.5).bar();
    for i in 0..232 {
        let v = if i == 77 + 23 {
            250.0
        } else if i == 77 + 24 {
            1200.0
        } else {
            0.0
        };
        pl_rec.f32(v).bar();
    }
    pl_rec
        .u8(0)
        .bar()
        .f32(1.0)
        .bar()
        .vs(0)
        .bar()
        .u8(0x5E)
        .bar()
        .vs(0)
        .bar()
        .u8(0x18)
        .bar()
        .r(0)
        .bar()
        .f32(0.0)
        .f32(0.0)
        .f32(0.0)
        .bar()
        .f32(0.0)
        .bar();
    pl_rec.vs(2).bar();
    pl_rec.r(5).bar().i32(3).bar().vs(0).bar();
    pl_rec
        .r(6)
        .bar()
        .i32(1)
        .bar()
        .vs(1)
        .bar()
        .vs(2)
        .bar()
        .u8(0x16)
        .bar()
        .u8(0x25)
        .bar()
        .f32(50.0)
        .bar();
    push_record(&mut b, 19, (1 << 1) | (1 << 4) | (1 << 5), TYPE_ACHR, &pl_rec.b);

    // 5. REFR PROP_REF (r(8)): disabled + deleted
    let mut rf = Build { b: Vec::new() };
    rf.u32(FORM_DISABLED | FORM_DELETED).bar();
    push_record(&mut b, 8, 1 << 0, TYPE_REFR, &rf.b);

    // 6. ACHR NPC_REF (r(9)): moved + talked_to_pc
    let mut nr = Build { b: Vec::new() };
    nr.r(7).f32(100.0).f32(200.0).f32(5.0).f32(0.0).f32(0.0).f32(0.5).bar();
    nr.u8(0).bar().u32(FORM_TALKED_TO_PC).bar();
    push_record(&mut b, 9, 0b11, TYPE_ACHR, &nr.b);

    // 7. FACT FACT_A (r(10)): reaction to FACT_B (r(11)) = 2 (ally)
    let mut fc = Build { b: Vec::new() };
    fc.vs(1).bar().r(11).bar().i32(50).bar().u32(2).bar();
    push_record(&mut b, 10, 1 << 2, TYPE_FACT, &fc.b);

    // 8. FLST (r(12)): u32 count=1 | ITEM_ID
    let mut fl = Build { b: Vec::new() };
    fl.u32(1).bar().r(5).bar();
    push_record(&mut b, 12, 1 << 31, TYPE_FLST, &fl.b);

    // 9. LVLI (r(13)): 1 entry (ITEM_ID, level 2, count 4, cond 1.0)
    let mut lv = Build { b: Vec::new() };
    lv.vs(1).bar().r(5).bar().u16(2).bar().u16(4).bar().f32(1.0).bar();
    push_record(&mut b, 13, 1 << 31, TYPE_LVLI, &lv.b);

    // 10. REPU (r(14)): fame 15.0, infamy 3.0
    let mut rp = Build { b: Vec::new() };
    rp.f32(15.0).bar().f32(3.0).bar();
    push_record(&mut b, 14, 1 << 1, TYPE_REPU, &rp.b);

    // 11. CHAL (r(15)): progress 5 (>= threshold 5 -> completed)
    let mut ch = Build { b: Vec::new() };
    ch.u32(5).bar().u32(4).bar();
    push_record(&mut b, 15, 1 << 1, TYPE_CHAL, &ch.b);

    // 12. NOTE (r(16)) & INFO (r(17))
    push_record(&mut b, 16, 1 << 31, TYPE_NOTE, &[]);
    push_record(&mut b, 17, 1 << 31, TYPE_INFO, &[]);

    // 13. Created 0xFF REFR (r(23)) — must be gracefully skipped & counted
    push_record(&mut b, 23, 1 << 0, TYPE_REFR, &[0, 0, 0, 0, BAR]);

    let g2 = b.len();
    b.extend(1000u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());

    let fid = b.len();
    b.extend((fids.len() as u32).to_le_bytes());
    for f in fids {
        b.extend(f.to_le_bytes());
    }
    b.extend(0u32.to_le_bytes());
    let tail = b.len();
    b.extend([5, 0, 0, 0, 0, 0, 0, 0, BAR]);

    let table = [fid, tail, g1, cf, g2, 5, 1, 14].map(|v| v as u32);
    for (i, v) in table.iter().enumerate() {
        b[table_at + 4 * i..table_at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

#[test]
fn fos_converts_to_valid_nvfsave_and_restores_all_sections() {
    let f = fixture();
    let raw = synthetic_fos_bytes();
    let fos = Fos::parse(&raw).unwrap();

    let inspect = inspect_fos(&fos, true);
    assert_eq!(inspect.schema, "fnv.save.fos_inspect/1");
    assert!(inspect.cosave_present);
    assert_eq!(inspect.source.player_name, "Synthetic");
    assert_eq!(inspect.decoded_counts.get("quests"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("form_lists"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("leveled_lists"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("reputations"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("challenges"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("notes"), Some(&1));
    assert_eq!(inspect.decoded_counts.get("said_infos"), Some(&1));

    let opts = ConvertOptions {
        kind: SaveKind::Manual,
        plugins: vec![PluginEntry { name: "FalloutNV.esm".into(), size: 1234 }],
        include_thumbnail: true,
        cosave_present: true,
    };
    let converted = convert_fos(&fos, &f.data, &f.forms, &f.refs, &opts).unwrap();
    assert_eq!(converted.report.missing_plugins, ["MissingMod.esp"]);
    assert!(converted.report.cosave_ignored);
    assert!(converted.report.warnings.iter().any(|w| w.contains(".nvse")));
    assert_eq!(converted.report.converted.get("globals"), Some(&3));
    assert_eq!(converted.report.converted.get("quests"), Some(&1));
    assert_eq!(converted.report.converted.get("form_lists"), Some(&1));
    assert_eq!(converted.report.converted.get("leveled_lists"), Some(&1));
    assert_eq!(converted.report.converted.get("reputations"), Some(&1));
    assert_eq!(converted.report.converted.get("challenges"), Some(&1));
    assert_eq!(converted.report.converted.get("notes"), Some(&1));
    assert_eq!(converted.report.converted.get("said_infos"), Some(&1));
    assert_eq!(converted.report.converted.get("weather"), Some(&1));
    assert_eq!(converted.report.converted.get("thumbnail_encoded"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("player_xp (AV2 code 24) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("player_karma (AV2 code 23) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("reputations (REPU bit 1, fame/infamy) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("active_quest (global type 1) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("form_lists (FLST bit 31, u32 count) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("leveled_lists (LVLI bit 31) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("challenges (CHAL bit 1, progress/flags) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("notes (NOTE bit 31) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("said_infos (INFO bit 31) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("player_item_conditions (extra 0x25) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("player_equipped (extra 0x16) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("reference_form_flags (REFR/ACHR/ACRE bit 0) [hypothesis]"), Some(&2));
    assert_eq!(converted.report.hypothesis_fields.get("faction_relations (FACT bit 2) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("player_special (NPC_ bit 2) [hypothesis]"), Some(&7));
    assert_eq!(converted.report.hypothesis_fields.get("player_tag_skills (CLAS bit 1) [hypothesis]"), Some(&3));
    assert_eq!(converted.report.hypothesis_fields.get("misc_stats (global type 0) [hypothesis]"), Some(&2));
    assert_eq!(converted.report.hypothesis_fields.get("weather (global type 8) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("quest_flags_delay_objectives (QUST) [hypothesis]"), Some(&1));
    assert_eq!(converted.report.hypothesis_fields.get("thumbnail_rgb_order (header screenshot) [hypothesis]"), Some(&1));
    assert!(converted.report.warnings.iter().any(|w| w.contains("[hypothesis]")));
    assert!(converted.report.skipped.contains_key("global: form of a plugin not loaded"));
    assert!(converted.report.skipped.contains_key("global: created form (0xFF)"));
    assert!(converted.report.skipped.contains_key("global_table: type 6 layout unmapped"));
    assert!(converted.report.skipped.contains_key("changed_form: created REFR (0xFF) unsupported"));

    // Write real .nvfsave file and verify via fnv_save::read_header + read_body + GameSnapshot::check + Game::restore.
    let dir = std::env::temp_dir().join(format!("nvf-fos-convert-test-{}", std::process::id()));
    let out = dir.join("converted.nvfsave");
    let body_bytes = serde_json::to_vec(&converted.body).unwrap();
    fnv_save::write(&out, &converted.header, &converted.shot, &body_bytes, false).unwrap();

    let head_only = fnv_save::read_header(&out).unwrap();
    assert_eq!(head_only.header.player_name, "Synthetic");
    assert_eq!(head_only.header.level, 4);
    assert_eq!(head_only.header.location, "OwnTestRoom");
    assert_eq!(head_only.header.play_seconds, 2.0 * 3600.0 + 15.0 * 60.0 + 30.0);
    assert_eq!(head_only.header.game_year, 2281);
    assert_eq!(head_only.header.game_month, 9);
    assert_eq!(head_only.header.game_day, 19);
    assert_eq!(head_only.header.game_hour, 14.5);
    assert_eq!(head_only.header.game_days_passed, Some(9.25));
    assert_eq!(head_only.header.weather.as_deref(), Some("OwnClearWeather"));
    assert_eq!(head_only.header.place.interior, Some(ROOM_ID));
    assert_eq!(head_only.header.place.feet, [12.0, 34.0, 56.0]);
    assert!(!head_only.shot.is_empty());
    assert_eq!(&head_only.shot[..8], b"\x89PNG\r\n\x1a\n");

    let (_h, _shot, body): (SaveHeader, Vec<u8>, SaveBody<GameSnapshot>) = fnv_save::read_body(&out).unwrap();
    assert_eq!(body.format, BODY_FORMAT);
    assert_eq!(body.engine.format, ENGINE_FORMAT);
    assert_eq!(body.engine.active_quest, Some(QUEST_ID));
    body.game.check().unwrap();

    let restored_world = World::restore(&f.forms, &f.refs, &body.game.sim).unwrap();
    let mut restored = Game::restore(&f.data, restored_world, GameConfig::default(), &body.game).unwrap();
    assert_eq!(restored.world.global(FormId(GLOB_CUSTOM)), Ok(42.0));
    assert_eq!(restored.player.name.as_deref(), Some("Synthetic"));
    assert_eq!(restored.player_level(), 4.0);
    assert_eq!(restored.player.xp, 1200.0);
    assert_eq!(restored.extra.get(&FormId::PLAYER_REF).unwrap().actor_values.get(&23), Some(&250.0));
    assert_eq!(restored.extra.get(&FormId::PLAYER_REF).unwrap().actor_values.get(&5), Some(&6.0));
    assert_eq!(restored.extra.get(&FormId::PLAYER_REF).unwrap().actor_values.get(&11), Some(&10.0));
    assert!(restored.player.tag_skills.contains(&38));
    assert_eq!(restored.player.misc_stats.get(&0), Some(&3.0));
    assert_eq!(restored.player.misc_stats.get(&2), Some(&11.0));
    assert_eq!(restored.world.item_count(FormId::PLAYER_REF, FormId(ITEM_ID)).unwrap(), 3);
    let inv = restored.world.peek(FormId::PLAYER_REF).unwrap().inventory.as_ref().unwrap();
    assert!(inv.is_equipped(FormId(ITEM_WORN_ID)));
    let cond = inv
        .groups()
        .find(|(id, _, _)| *id == FormId(ITEM_WORN_ID))
        .and_then(|(_, e, _)| e.condition);
    assert_eq!(cond, Some(0.5));
    let qs = restored.quests.get(&FormId(QUEST_ID)).unwrap();
    assert!(qs.running && !qs.completed);
    assert_eq!(qs.delay, Some(2.5));
    assert!(qs.stages_done.contains(&20));
    assert!(qs.objectives[&10].displayed && qs.objectives[&10].completed);
    let q_inst = restored.instances.get(&InstKey::Quest { quest: FormId(QUEST_ID) }).unwrap();
    assert_eq!(q_inst.vars.get(&1), Some(&Value::Num(7.5)));
    assert_eq!(q_inst.vars.get(&2), Some(&Value::Ref(NPC_REF)));
    assert!(!restored.world.peek(FormId(PROP_REF)).unwrap().enabled());
    assert!(restored.world.peek(FormId(PROP_REF)).unwrap().deleted());
    assert!(restored.extra.get(&FormId(NPC_REF)).unwrap().talked_to_pc);
    assert_eq!(restored.world.peek(FormId(NPC_REF)).unwrap().pos, [100.0, 200.0, 5.0]);
    assert_eq!(restored.forms.faction_reactions[&FormId(FACT_A)][&FormId(FACT_B)], 2);
    assert_eq!(restored.form_lists[&FormId(FLST_ID)], [FormId(ITEM_ID)]);
    assert_eq!(restored.world.leveled_added(FormId(LVLI_ID)).len(), 1);
    assert_eq!(restored.player.reputation[&FormId(REPU_ID)], [3.0, 15.0]);
    assert!(restored.player.challenges[&FormId(CHAL_ID)].completed);
    assert_eq!(restored.player.challenges[&FormId(CHAL_ID)].progress, 5);
    assert!(restored.player.notes.contains(&FormId(NOTE_ID)));
    assert!(restored.said_once(FormId(INFO_ID)));
    assert_eq!(restored.weather.shown, Some(FormId(WTHR_ID)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_truncated_and_unknown_extra_data_are_handled_without_panicking() {
    let f = fixture();
    assert!(Fos::parse(b"NOTAFOS").is_err());
    let valid = synthetic_fos_bytes();
    assert!(Fos::parse(&valid[..valid.len() / 2]).is_err());

    // Mutate the player's inventory extra type from 0x25 to an unknown type 0x0D:
    // conversion must succeed, keep the rest of the save, and count inventory as skipped.
    let mut fos = Fos::parse(&valid).unwrap();
    let player_rec = fos.changed.iter_mut().find(|c| c.form == PLAYER_REF).unwrap();
    let last_extra_pos = player_rec.data.len() - 7;
    assert_eq!(player_rec.data[last_extra_pos], 0x25);
    player_rec.data[last_extra_pos] = 0x0D;
    let opts = ConvertOptions {
        plugins: vec![PluginEntry { name: "FalloutNV.esm".into(), size: 1234 }],
        include_thumbnail: false,
        ..ConvertOptions::default()
    };
    let converted = convert_fos(&fos, &f.data, &f.forms, &f.refs, &opts).unwrap();
    assert!(converted.shot.is_empty());
    assert!(converted.report.skipped.contains_key("player_inventory: not read (missing player record or unmapped layout)"));
    assert!(!converted.report.hypothesis_fields.contains_key("player_item_conditions (extra 0x25) [hypothesis]"));
    assert!(!converted.report.hypothesis_fields.contains_key("player_equipped (extra 0x16) [hypothesis]"));
    assert!(!converted.report.hypothesis_fields.contains_key("thumbnail_rgb_order (header screenshot) [hypothesis]"));
    assert_eq!(converted.report.converted.get("quests"), Some(&1));
    converted.body.game.check().unwrap();

    // Regression: i32::MIN inventory delta must skip cleanly without negation overflow,
    // and non-empty TYPE_INFO/TYPE_NOTE flag-31 records must be reported as skipped.
    let mut edge_fos = Fos::parse(&valid).unwrap();
    let edge_player = edge_fos.changed.iter_mut().find(|c| c.form == PLAYER_REF).unwrap();
    // First item in synthetic_fos_bytes() has count = 3i32 right before its 0-lists vsval.
    let item1_count_pos = edge_player.data.windows(5).position(|w| w == [3, 0, 0, 0, BAR]).unwrap();
    edge_player.data[item1_count_pos..item1_count_pos + 4].copy_from_slice(&i32::MIN.to_le_bytes());
    let info_rec = edge_fos.changed.iter_mut().find(|c| c.kind == TYPE_INFO).unwrap();
    info_rec.data.push(0xAA);
    let note_rec = edge_fos.changed.iter_mut().find(|c| c.kind == TYPE_NOTE).unwrap();
    note_rec.data.push(0xBB);
    let edge_converted = convert_fos(&edge_fos, &f.data, &f.forms, &f.refs, &opts).unwrap();
    assert!(edge_converted.report.skipped.contains_key("player_item: negative item count overflow"));
    assert!(edge_converted.report.skipped.contains_key("changed_form: INFO (kind 8) unmapped"));
    assert!(edge_converted.report.skipped.contains_key("changed_form: NOTE (kind 31) unmapped"));
    assert!(!edge_converted.report.hypothesis_fields.contains_key("notes (NOTE bit 31) [hypothesis]"));
    assert!(!edge_converted.report.hypothesis_fields.contains_key("said_infos (INFO bit 31) [hypothesis]"));
    edge_converted.body.game.check().unwrap();
}
