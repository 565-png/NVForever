//! `fnvtool pbr ...`: authored PBR material maps under `textures\pbr\` (#2981,
//! docs/spec-notes/pbr-materials.md). `list` mounts the VFS as the engine does and names
//! every set it finds and, per member, the archive or loose folder that supplies it.

use anyhow::Context;
use clap::Subcommand;
use fnv_pbr::{member_path, PbrIndex, PBR_DIR};
use fnv_vfs::Vfs;
use serde_json::{json, Value};
use std::path::PathBuf;

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
        PbrCmd::List { mount } => {
            let (vfs, plugins) = mount_vfs(&mount)?;
            Ok(listing(&vfs, &plugins))
        }
    }
}

/// Mount the VFS with the engine's own helpers (engine `main.rs`: `open_for_plugins_layered`
/// under `--mo2`, else `open_for_plugins_in` with the mods folder). Returns it and the
/// plugins it was mounted for.
fn mount_vfs(a: &MountArgs) -> anyhow::Result<(Vfs, Vec<String>)> {
    anyhow::ensure!(a.data.is_dir(), "{}: not a folder", a.data.display());
    if let Some(l) = &a.archive_list {
        fnv_vfs::set_archive_list(fnv_vfs::parse_archive_list(l)).map_err(|_| anyhow::anyhow!("--archive-list: no archive named"))?;
    }
    let default_plugins = || fnv_vfs::DEFAULT_PLUGINS.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    match &a.mo2 {
        Some(instance) => {
            let layout = fnv_install::mo2::Layout::open(instance, a.profile.as_deref()).map_err(|e| anyhow::anyhow!(e))?;
            let plugins = if a.plugins.is_empty() { layout.profile_plugins().map_err(|e| anyhow::anyhow!(e))? } else { a.plugins.clone() };
            if a.archive_list.is_none() {
                if let Some((list, _)) = layout.profile_archives() {
                    fnv_vfs::set_archive_list(list).map_err(|_| anyhow::anyhow!("the profile's archive list names no archive"))?;
                }
            }
            let vfs = Vfs::open_for_plugins_layered(&a.data, &layout.mod_roots(), &plugins).with_context(|| a.data.display().to_string())?;
            Ok((vfs, plugins))
        }
        None => {
            let plugins = if a.plugins.is_empty() { default_plugins() } else { a.plugins.clone() };
            let vfs = Vfs::open_for_plugins_in(&a.data, a.mods.as_deref(), &plugins).with_context(|| a.data.display().to_string())?;
            Ok((vfs, plugins))
        }
    }
}

/// Where the file `path` that wins in `vfs` comes from: an archive (its name and file) or a
/// loose folder (the root it was indexed from and the file).
fn source(vfs: &Vfs, path: &str) -> Value {
    match vfs.source_of(path) {
        Some("loose") => {
            let file = vfs.loose_path(path);
            let root = file.as_ref().and_then(|f| vfs.loose_roots().iter().rev().find(|r| f.starts_with(r)));
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

fn listing(vfs: &Vfs, plugins: &[String]) -> Value {
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
        "data": vfs.data_dir().display().to_string(),
        "plugins": plugins,
        "archives": vfs.archives().collect::<Vec<_>>(),
        "loose_roots": vfs.loose_roots().iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
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
    use std::path::Path;

    /// The listing of a VFS mounted from `data` and an optional mods folder.
    fn list_at(data: &Path, mods: Option<&Path>) -> Value {
        let a = MountArgs { data: data.to_path_buf(), plugins: Vec::new(), mods: mods.map(Path::to_path_buf), mo2: None, profile: None, archive_list: None };
        let (vfs, plugins) = mount_vfs(&a).unwrap();
        listing(&vfs, &plugins)
    }

    fn file(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, rel).unwrap();
    }

    #[test]
    fn list_names_sets_states_and_the_source_of_each_member() {
        let root = std::env::temp_dir().join(format!("nvf-fnvtool-pbr-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, mods) = (root.join("Data"), root.join("mods"));
        file(&data, "textures/pbr/walls/brick07_orm.dds");
        file(&data, "textures/pbr/walls/brick07_n.dds");
        file(&mods, "textures/pbr/walls/brick07_orm.dds");
        file(&mods, "textures/pbr/walls/brick07.toml");
        file(&mods, "textures/pbr/props/crate_p.dds");
        file(&mods, "textures/pbr/props/lamp_n.dds");
        file(&mods, "textures/pbr/props/lamp.dds");
        file(&data, "textures/walls/brick07.dds");

        let v = list_at(&data, Some(&mods));
        assert_eq!(v["schema"], "fnv.pbr.listing/1");
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
}
