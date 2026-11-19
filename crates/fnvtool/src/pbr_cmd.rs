//! `fnvtool pbr ...`: authored PBR material maps under `textures\pbr\` (#2981,
//! docs/spec-notes/pbr-materials.md). `list` mounts the VFS as the engine does and names
//! every set it finds and, per member, the archive or loose folder that supplies it.

use anyhow::Context;
use clap::Subcommand;
use fnv_pbr::{member_path, PbrIndex, PBR_DIR};
use fnv_vfs::Vfs;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum PbrCmd {
    /// The sets under `textures\pbr\` a load order supplies: per set its diffuse, state and
    /// members, the source that wins for each member, then totals and what was left out
    /// (fnv.pbr.listing/1).
    List {
        #[command(flatten)]
        mount: MountArgs,
    },
}

/// The VFS as the engine mounts it: `--data` with the plugins' archives, and either
/// NVForever's mods folder (`--mods`, the home `mods`) or a Mod Organizer 2 profile
/// (`--mo2`, `--profile`) over it.
#[derive(clap::Args)]
pub struct MountArgs {
    /// The game's Data folder.
    #[arg(long)]
    data: PathBuf,
    /// Plugins in load order (default: the base game and its add-on masters; with `--mo2`,
    /// the profile's active plugins).
    plugins: Vec<String>,
    /// NVForever's mods folder: its loose files over Data's and the archives beside its
    /// plugins, as the engine mounts its home `mods` folder.
    #[arg(long, conflicts_with = "mo2")]
    mods: Option<PathBuf>,
    /// A Mod Organizer 2 instance folder: the profile's enabled mods layered over Data, its
    /// archive list and plugins, as `nvforever --mo2` mounts them (docs/design/mod-folders.md).
    #[arg(long)]
    mo2: Option<PathBuf>,
    /// The Mod Organizer 2 profile (default: the instance's selected profile).
    #[arg(long, requires = "mo2")]
    profile: Option<String>,
    /// A base archive list in place of the shipped `SArchiveList` (comma-separated, lowest
    /// precedence first), as NVForever.ini's `[Archive] SArchiveList` sets it.
    #[arg(long)]
    archive_list: Option<String>,
}

pub fn run(cmd: PbrCmd) -> anyhow::Result<Value> {
    match cmd {
        PbrCmd::List { mount } => list(&mount, &std::env::current_dir().context("the current folder")?),
    }
}

/// `pbr list`, relative folders taken from `cwd`. The engine refuses to start with a
/// damaged archive (engine `main.rs`, `damaged_archives`), and a listing without its files
/// would mislead, so a damaged archive fails the command, as `fnvtool bsa check` does.
fn list(a: &MountArgs, cwd: &Path) -> anyhow::Result<Value> {
    let m = mount_vfs(a, cwd)?;
    let broken: Vec<String> = m.vfs.broken_archives().iter().map(|(n, why)| format!("{n}: {why}")).collect();
    anyhow::ensure!(broken.is_empty(), "damaged archive(s), which the engine refuses to start with: {}", broken.join("; "));
    Ok(listing(&m))
}

/// A path as given, made absolute against `cwd`: a relative mods root would otherwise
/// reach `Vfs::open` as an archive name relative to Data, and its archives would be lost.
fn absolute(cwd: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// The VFS mounted, and the exact inputs it was mounted from.
struct Mount {
    vfs: Vfs,
    /// Where the plugin list came from.
    plugins_from: &'static str,
    plugins_asked: Vec<String>,
    /// The plugins opened and loadable, as the VFS was given them (file names).
    plugins: Vec<String>,
    /// (plugin, why) for each left out.
    dropped: Vec<(String, String)>,
    /// The base archive list set for the run and where it came from; `None`: the shipped one.
    archive_list: Option<(Vec<String>, String)>,
}

/// Mount the VFS as the engine does at a developer start (`--data`), with the root crates'
/// own steps (engine `main.rs` and `world::open_plugins`): plugins looked up through the
/// mods folders (`fnv_install::plugins::plugin_path_in`), opened, and those unreadable or
/// with an unmet master left out (`fnv_esm::loadorder::drop_unloadable`), then
/// `Vfs::open_for_plugins_layered` under `--mo2`, else `Vfs::open_for_plugins_in` with the
/// mods folder. Relative folders are taken from `cwd`.
fn mount_vfs(a: &MountArgs, cwd: &Path) -> anyhow::Result<Mount> {
    let data = absolute(cwd, &a.data);
    anyhow::ensure!(data.is_dir(), "{}: not a folder", data.display());
    let mut archive_list = None;
    if let Some(l) = &a.archive_list {
        let list = fnv_vfs::parse_archive_list(l);
        fnv_vfs::set_archive_list(list.clone()).map_err(|_| anyhow::anyhow!("--archive-list: no archive named"))?;
        archive_list = Some((list, "--archive-list".to_string()));
    }
    let masters = || fnv_vfs::DEFAULT_PLUGINS.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    // The VFS's layers over Data, lowest first (the plugin lookup takes them highest first).
    let (layout, layers): (Option<fnv_install::mo2::Layout>, Vec<PathBuf>) = match &a.mo2 {
        Some(instance) => {
            let layout = fnv_install::mo2::Layout::open(&absolute(cwd, instance), a.profile.as_deref()).map_err(|e| anyhow::anyhow!(e))?;
            let roots = layout.mod_roots().iter().map(|r| absolute(cwd, r)).collect();
            (Some(layout), roots)
        }
        None => (None, a.mods.iter().map(|m| absolute(cwd, m)).collect()),
    };
    let (plugins_from, asked) = match (&layout, a.plugins.is_empty()) {
        (_, false) => ("arguments", a.plugins.clone()),
        (Some(l), true) => match l.profile_plugins().map_err(|e| anyhow::anyhow!(e))? {
            p if p.is_empty() => {
                eprintln!("warning: the Mod Organizer 2 profile has no active plugins; loading the masters only");
                ("the base game and add-on masters (the profile has no active plugins)", masters())
            }
            p => ("the Mod Organizer 2 profile", p),
        },
        // Without plugins the engine takes the masters that are in Data (the add-ons are
        // optional, engine `main.rs`).
        (None, true) => {
            let present = masters().into_iter().enumerate().filter(|(i, n)| *i == 0 || fnv_install::plugins::plugin_path(&data, None, n).is_file()).map(|(_, n)| n).collect();
            ("the base game and the add-on masters in Data", present)
        }
    };
    if let (Some(l), None) = (&layout, &archive_list) {
        if let Some((list, from)) = l.profile_archives() {
            fnv_vfs::set_archive_list(list.clone()).map_err(|_| anyhow::anyhow!("the profile's archive list names no archive"))?;
            archive_list = Some((list, from.display().to_string()));
        }
    }
    let lookup: Vec<PathBuf> = layers.iter().rev().cloned().collect();
    let mut dropped = Vec::new();
    let mut opened = Vec::new();
    for n in &asked {
        // A full path (`--mo2-profile`) is opened as is; the VFS is given its file name, the
        // name the engine's load order holds.
        let name = Path::new(n).file_name().map_or_else(|| n.clone(), |f| f.to_string_lossy().into_owned());
        let path = if Path::new(n).is_absolute() { PathBuf::from(n) } else { fnv_install::plugins::plugin_path_in(&data, &lookup, n) };
        match fnv_esm::Plugin::open(&path) {
            Ok(mut p) => {
                p.name = name;
                opened.push(p);
            }
            Err(e) => dropped.push((name, format!("cannot be read: {e}"))),
        }
    }
    let (kept, refused) = fnv_esm::loadorder::drop_unloadable(opened);
    dropped.extend(refused.into_iter().map(|d| (d.plugin, d.reason.to_string())));
    for (p, why) in &dropped {
        eprintln!("warning: plugin {p} left out: {why}");
    }
    let plugins: Vec<String> = kept.into_iter().map(|p| p.name).collect();
    let vfs = match &layout {
        Some(_) => Vfs::open_for_plugins_layered(&data, &layers, &plugins),
        None => Vfs::open_for_plugins_in(&data, layers.first().map(PathBuf::as_path), &plugins),
    }
    .with_context(|| data.display().to_string())?;
    Ok(Mount { vfs, plugins_from, plugins_asked: asked, plugins, dropped, archive_list })
}

/// The loose folder that indexed `file` under `key`: the root whose path to the file,
/// normalised, is the key (a prefix alone names the wrong root when roots nest).
fn loose_root<'a>(vfs: &'a Vfs, key: &str, file: &Path) -> Option<&'a PathBuf> {
    vfs.loose_roots().iter().rev().find(|r| file.strip_prefix(r).is_ok_and(|rel| fnv_vfs::normalize(&rel.to_string_lossy()) == key))
}

/// Where the file `path` that wins in `vfs` comes from: an archive (its name and file) or a
/// loose folder (the root that indexed it and the file).
fn source(vfs: &Vfs, path: &str) -> Value {
    match vfs.source_of(path) {
        Some("loose") => {
            let file = vfs.loose_path(path);
            let key = fnv_vfs::normalize(path);
            let root = file.as_ref().and_then(|f| loose_root(vfs, &key, f));
            json!({
                "kind": "loose",
                "root": root.map(|r| r.display().to_string()),
                "file": file.map(|f| f.display().to_string()),
            })
        }
        Some(archive) => json!({
            "kind": "archive",
            "archive": archive,
            "file": vfs.archive_path(archive).map(|f| f.display().to_string()),
        }),
        None => Value::Null,
    }
}

fn listing(m: &Mount) -> Value {
    let vfs = &m.vfs;
    let index = PbrIndex::build(vfs.paths(|k| k.starts_with(PBR_DIR)));
    let sets: Vec<Value> = index
        .sets()
        .map(|(stem, files)| {
            let mut members = serde_json::Map::new();
            for m in files.members() {
                let path = member_path(stem, m);
                members.insert(m.name().to_string(), json!({"path": path, "source": source(vfs, &path)}));
            }
            json!({
                "stem": stem,
                "diffuse": fnv_pbr::diffuse_stem(stem),
                "state": files.state().name(),
                "members": members,
            })
        })
        .collect();
    let c = index.counts();
    json!({
        "schema": "fnv.pbr.listing/1",
        "mount": {
            "data": vfs.data_dir().display().to_string(),
            "loose_roots": vfs.loose_roots().iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
            "plugins_from": m.plugins_from,
            "plugins_asked": m.plugins_asked,
            "plugins": m.plugins,
            "plugins_dropped": m.dropped.iter().map(|(p, why)| json!({"plugin": p, "why": why})).collect::<Vec<_>>(),
            "archive_list": m.archive_list.as_ref().map(|(names, from)| json!({"names": names, "from": from})),
            "archive_order_env": std::env::var(fnv_vfs::ARCHIVE_ORDER_ENV).ok().filter(|v| !v.trim().is_empty()),
            "archives": vfs.archives().map(|n| json!({"name": n, "file": vfs.archive_path(n).map(|f| f.display().to_string())})).collect::<Vec<_>>(),
        },
        "sets": sets,
        "unmatched": index.unmatched(),
        "totals": {
            "files": c.files,
            "sets": c.sets,
            "active": c.active(),
            "authored": c.authored,
            "height_only": c.height_only,
            "incomplete": c.incomplete,
            "unmatched": c.unmatched,
            "members": {"orm": c.orm, "normal": c.normal, "height": c.height, "scalars": c.scalars, "albedo": c.albedo},
        },
        "summary": index.summary(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(data: &str, plugins: &[&str], mods: Option<&str>) -> MountArgs {
        MountArgs {
            data: PathBuf::from(data),
            plugins: plugins.iter().map(|p| p.to_string()).collect(),
            mods: mods.map(PathBuf::from),
            mo2: None,
            profile: None,
            archive_list: None,
        }
    }

    fn file(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, rel).unwrap();
    }

    /// A plugin with only its TES4 header, needing `masters`.
    fn plugin(dir: &Path, name: &str, masters: &[&str]) {
        let sub = |sig: &[u8; 4], data: &[u8]| [&sig[..], &(data.len() as u16).to_le_bytes(), data].concat();
        let mut payload = sub(b"HEDR", &[1.34f32.to_le_bytes(), 0i32.to_le_bytes(), 0x800u32.to_le_bytes()].concat());
        for m in masters {
            payload.extend(sub(b"MAST", &[m.as_bytes(), &[0]].concat()));
            payload.extend(sub(b"DATA", &[0; 8]));
        }
        let record = [&b"TES4"[..], &(payload.len() as u32).to_le_bytes(), &[0; 12], &15u16.to_le_bytes(), &[0; 2], &payload].concat();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), record).unwrap();
    }

    /// An archive holding one file `folder\file`.
    fn archive(dir: &Path, name: &str, folder: &str, file: &str) {
        use fnv_bsa::{encode_block, flags, write_archive, WriteFile, WriteFolder};
        let files = vec![WriteFile { name: file.as_bytes().to_vec(), size_flag_bits: 0, data: encode_block(b"x", false, None) }];
        let mut bytes = Vec::new();
        write_archive(&mut bytes, flags::DIR_NAMES | flags::FILE_NAMES, 0x1, 0, &[WriteFolder { name: folder.as_bytes().to_vec(), files }]).unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    fn fresh(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nvf-fnvtool-pbr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn list_names_sets_states_and_the_source_of_each_member() {
        let root = fresh("list");
        let (data, mods) = (root.join("Data"), root.join("mods"));
        file(&data, "textures/pbr/walls/brick07_orm.dds");
        file(&data, "textures/pbr/walls/brick07_n.dds");
        file(&mods, "textures/pbr/walls/brick07_orm.dds");
        file(&mods, "textures/pbr/walls/brick07.toml");
        file(&mods, "textures/pbr/props/crate_p.dds");
        file(&mods, "textures/pbr/props/lamp_n.dds");
        file(&mods, "textures/pbr/props/lamp.dds");
        file(&data, "textures/walls/brick07.dds");

        let v = list(&args(&data.to_string_lossy(), &[], Some(&mods.to_string_lossy())), &root).unwrap();
        assert_eq!(v["schema"], "fnv.pbr.listing/1");
        // No plugin in this Data: the base game's master is asked for and left out.
        assert_eq!(v["mount"]["plugins_asked"], json!(["FalloutNV.esm"]));
        assert_eq!(v["mount"]["plugins"], json!([]));
        assert_eq!(v["mount"]["plugins_dropped"][0]["plugin"], "FalloutNV.esm");
        let sets = v["sets"].as_array().unwrap();
        let stems: Vec<&str> = sets.iter().map(|s| s["stem"].as_str().unwrap()).collect();
        assert_eq!(stems, [r"textures\pbr\props\crate", r"textures\pbr\props\lamp", r"textures\pbr\walls\brick07"]);
        assert_eq!(sets[0]["state"], "height-only");
        assert_eq!(sets[1]["state"], "incomplete");
        let brick = &sets[2];
        assert_eq!(brick["state"], "authored");
        assert_eq!(brick["diffuse"], r"textures\walls\brick07");
        let root_of = |m: &str| brick["members"][m]["source"]["root"].as_str().map(PathBuf::from);
        // Each member resolves on its own: the mod's ORM and scalars, Data's normal.
        assert_eq!(root_of("orm"), Some(mods.clone()));
        assert_eq!(root_of("scalars"), Some(mods.clone()));
        assert_eq!(root_of("normal"), Some(data.clone()));
        assert_eq!(brick["members"]["orm"]["source"]["kind"], "loose");
        assert_eq!(brick["members"]["orm"]["path"], r"textures\pbr\walls\brick07_orm.dds");
        assert!(brick["members"].get("height").is_none());
        assert_eq!(v["unmatched"], json!([r"textures\pbr\props\lamp.dds"]));
        let t = &v["totals"];
        assert_eq!((t["files"].as_u64(), t["sets"].as_u64(), t["active"].as_u64()), (Some(6), Some(3), Some(2)));
        assert_eq!((t["authored"].as_u64(), t["height_only"].as_u64(), t["incomplete"].as_u64(), t["unmatched"].as_u64()), (Some(1), Some(1), Some(1), Some(1)));
        assert_eq!(t["members"], json!({"orm": 1, "normal": 2, "height": 1, "scalars": 1, "albedo": 0}));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Sibling `Data` and `mods` given as relative folders: the mods folder's plugin-owned
    /// archive is mounted (a relative root once reached `Vfs::open` joined onto Data and was
    /// lost), beside its loose files.
    #[test]
    fn a_relative_mods_root_keeps_its_archives() {
        let root = fresh("relative");
        let (data, mods) = (root.join("Data"), root.join("mods"));
        plugin(&data, "Example.esp", &[]);
        archive(&mods, "Example.bsa", r"textures\pbr\walls", "brick_orm.dds");
        file(&mods, "textures/pbr/walls/brick_n.dds");

        let v = list(&args("Data", &["Example.esp"], Some("mods")), &root).unwrap();
        assert_eq!(v["mount"]["plugins"], json!(["Example.esp"]));
        assert_eq!(v["mount"]["data"], data.display().to_string());
        let brick = &v["sets"][0];
        assert_eq!(brick["stem"], r"textures\pbr\walls\brick");
        assert_eq!(brick["state"], "authored");
        let orm = &brick["members"]["orm"]["source"];
        assert_eq!((orm["kind"].as_str(), orm["archive"].as_str()), (Some("archive"), Some(mods.join("Example.bsa").to_string_lossy().as_ref())));
        assert_eq!(orm["file"].as_str().map(PathBuf::from), Some(mods.join("Example.bsa")));
        assert_eq!(brick["members"]["normal"]["source"]["root"].as_str().map(PathBuf::from), Some(mods.clone()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Plugins are opened first, as the engine does: one missing, one whose master is
    /// missing, are left out with why, and the second's archive is not mounted.
    #[test]
    fn unloadable_plugins_are_dropped_and_named() {
        let root = fresh("dropped");
        let data = root.join("Data");
        plugin(&data, "Base.esm", &[]);
        plugin(&data, "Needs.esp", &["Missing.esm"]);
        archive(&data, "Needs.bsa", r"textures\pbr\walls", "lost_orm.dds");
        archive(&data, "Base.bsa", r"textures\pbr\walls", "kept_orm.dds");

        let v = list(&args("Data", &["Base.esm", "Gone.esp", "Needs.esp"], None), &root).unwrap();
        let m = &v["mount"];
        assert_eq!(m["plugins_asked"], json!(["Base.esm", "Gone.esp", "Needs.esp"]));
        assert_eq!(m["plugins"], json!(["Base.esm"]));
        let dropped: Vec<&str> = m["plugins_dropped"].as_array().unwrap().iter().map(|d| d["plugin"].as_str().unwrap()).collect();
        assert_eq!(dropped, ["Gone.esp", "Needs.esp"]);
        assert!(m["plugins_dropped"][0]["why"].as_str().unwrap().starts_with("cannot be read"));
        let archives: Vec<&str> = m["archives"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
        assert_eq!(archives, ["Base.bsa"]);
        let stems: Vec<&str> = v["sets"].as_array().unwrap().iter().map(|s| s["stem"].as_str().unwrap()).collect();
        assert_eq!(stems, [r"textures\pbr\walls\kept"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A damaged archive fails the listing, as it stops the engine.
    #[test]
    fn a_damaged_archive_fails_the_listing() {
        let root = fresh("broken");
        let data = root.join("Data");
        plugin(&data, "Example.esp", &[]);
        std::fs::write(data.join("Example.bsa"), b"not an archive").unwrap();
        let e = list(&args("Data", &["Example.esp"], None), &root).unwrap_err().to_string();
        assert!(e.contains("damaged archive") && e.contains("Example.bsa"), "{e}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The loose root named is the one that indexed the file under its key, also when a
    /// mods folder lies inside Data (a path prefix alone would name the mods folder for
    /// Data's file).
    #[test]
    fn nested_roots_name_the_root_that_indexed_the_file() {
        let root = fresh("nested");
        let data = root.join("Data");
        let mods = data.join("textures");
        file(&data, "textures/pbr/walls/brick_orm.dds");
        let v = list(&args("Data", &[], Some("Data/textures")), &root).unwrap();
        let orm = &v["sets"][0]["members"]["orm"]["source"];
        assert_eq!(v["sets"][0]["stem"], r"textures\pbr\walls\brick");
        assert_eq!(orm["root"].as_str().map(PathBuf::from), Some(data.clone()));
        // The mods folder indexed the same file under `pbr\…`, outside textures\pbr\.
        assert!(mods.join("pbr").is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
