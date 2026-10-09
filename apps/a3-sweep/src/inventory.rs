//! The scenario inventory: every mission folder of the install, what kind of scenario it is and
//! which world it runs on.
//!
//! Missions ship inside addon PBOs (`a3\missions_f_*`), plus loose mission PBOs in the game's
//! `Missions` and `MPMissions` folders. A folder is a scenario when it holds a `mission.sqm`; its
//! kind comes from where `CfgMissions` lists it:
//!
//! - `CfgMissions >> Missions | Showcases | Challenges | MPMissions | Cutscenes | Tutorial`:
//!   each listed class (at any depth) names its folder with `directory`;
//! - `CfgMissions >> Campaigns >> <campaign> >> directory` is the campaign folder. Its
//!   `class Campaign` (in config, or in the campaign's `description.ext`) names each mission
//!   with `template`, a folder under `<campaign>\missions`. A campaign whose `directory` is a
//!   mission folder itself (Old Man) is that one mission.
//!
//! Folders no listing names are [`Kind::Unlisted`], except building blocks that are not
//! scenarios on their own ([`Kind::Fragment`]): the Contact `sites` and the unlisted siblings of
//! campaign missions (Old Man's per-area layers).

use std::collections::BTreeMap;
use std::path::Path;

use a3_config::{ConfigRef, ConfigTree};
use a3_vfs::{Vfs, VfsPath};
use serde::{Deserialize, Serialize};

/// What kind of scenario a mission folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A mission of a campaign (Bootcamp, East Wind, Apex, Old Man, ...).
    Campaign,
    /// `CfgMissions >> Missions`: single-player scenarios.
    Scenario,
    /// `CfgMissions >> Showcases`.
    Showcase,
    /// `CfgMissions >> Challenges`: firing drills, time trials, VR challenges.
    Challenge,
    /// `CfgMissions >> Tutorial`.
    Tutorial,
    /// `CfgMissions >> MPMissions`, and the loose `MPMissions` folder.
    Multiplayer,
    /// `CfgMissions >> Cutscenes` and the main-menu background scenes.
    Cutscene,
    /// A mission folder no listing names.
    Unlisted,
    /// A building block that is not a scenario on its own; skipped unless asked for.
    Fragment,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Campaign,
        Kind::Scenario,
        Kind::Showcase,
        Kind::Challenge,
        Kind::Tutorial,
        Kind::Multiplayer,
        Kind::Cutscene,
        Kind::Unlisted,
        Kind::Fragment,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Campaign => "campaign",
            Kind::Scenario => "scenario",
            Kind::Showcase => "showcase",
            Kind::Challenge => "challenge",
            Kind::Tutorial => "tutorial",
            Kind::Multiplayer => "multiplayer",
            Kind::Cutscene => "cutscene",
            Kind::Unlisted => "unlisted",
            Kind::Fragment => "fragment",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| {
            k.as_str().eq_ignore_ascii_case(text) || (text == "mp" && *k == Kind::Multiplayer)
        })
    }

    /// The `CfgMissions` category class that lists this kind.
    fn from_category(category: &str) -> Option<Kind> {
        Some(match category.to_ascii_lowercase().as_str() {
            "missions" => Kind::Scenario,
            "showcases" => Kind::Showcase,
            "challenges" => Kind::Challenge,
            "tutorial" => Kind::Tutorial,
            "mpmissions" => Kind::Multiplayer,
            "cutscenes" => Kind::Cutscene,
            _ => return None,
        })
    }
}

/// One mission folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// The virtual folder, lower case, `\`-separated, e.g.
    /// `a3\missions_f_bootcamp\campaign\missions\boot_m02.altis`.
    pub folder: String,
    pub kind: Kind,
    /// The world the folder's extension names, lower case (`altis`).
    pub world: String,
    /// What ships it: the addon folder after `a3\` (`missions_f_bootcamp`), or `Missions` /
    /// `MPMissions` for loose PBOs.
    pub package: String,
    /// The `CfgMissions` class that lists it, or the campaign's class for a campaign mission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_as: Option<String>,
    /// The campaign folder of a campaign mission (its `description.ext` is
    /// `campaignConfigFile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign: Option<String>,
    /// The scenario ships with an optional DLC (Contact, creator DLC), so it runs with the
    /// optional DLC loaded; every other scenario runs on the base game and its default DLC.
    #[serde(default)]
    pub optional_mods: bool,
}

impl Scenario {
    /// The folder's last component (`boot_m02.altis`).
    pub fn name(&self) -> &str {
        self.folder.rsplit('\\').next().unwrap_or(&self.folder)
    }

    #[cfg(test)]
    pub fn for_test(folder: &str) -> Scenario {
        Scenario {
            folder: normalize(folder),
            kind: Kind::Unlisted,
            world: world_of(folder),
            package: package_of(folder),
            listed_as: None,
            campaign: None,
            optional_mods: false,
        }
    }
}

/// A virtual folder as the inventory keys it: lower case, `\`-separated, no separators at either
/// end.
pub fn normalize(folder: &str) -> String {
    folder
        .trim()
        .replace('/', "\\")
        .trim_matches('\\')
        .to_ascii_lowercase()
}

/// The world a mission folder names through its extension (`boot_m02.altis` → `altis`).
pub fn world_of(folder: &str) -> String {
    let name = folder
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("");
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The addon folder that ships `folder` (`a3\missions_f_epa\...` → `missions_f_epa`).
pub fn package_of(folder: &str) -> String {
    let folder = normalize(folder);
    let mut parts = folder.split('\\');
    match (parts.next(), parts.next()) {
        (Some("a3"), Some(addon)) => addon.to_owned(),
        (Some(first), _) => first.to_owned(),
        _ => String::new(),
    }
}

/// What `CfgMissions` (and the campaigns' descriptions) say about mission folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Normalised folder → (kind, listing class, campaign folder).
    pub folders: BTreeMap<String, (Kind, String, Option<String>)>,
    /// Normalised campaign folders.
    pub campaigns: Vec<String>,
}

impl Listing {
    /// Reads `CfgMissions` from the merged config. `campaign_description` returns a campaign's
    /// parsed `description.ext` (as a tree) for campaigns whose `class Campaign` is not in
    /// config.
    pub fn from_config(
        config: &ConfigTree,
        mut campaign_description: impl FnMut(&str) -> Option<ConfigTree>,
    ) -> Listing {
        let mut listing = Listing::default();
        let missions = config.root().get("CfgMissions");
        for category in missions.entries().into_iter().filter(|c| c.is_class()) {
            if category.name().eq_ignore_ascii_case("Campaigns") {
                for campaign in category.entries().into_iter().filter(|c| c.is_class()) {
                    listing.add_campaign(&campaign, &mut campaign_description);
                }
            } else if let Some(kind) = Kind::from_category(category.name()) {
                listing.add_listed(&category, kind);
            }
        }
        listing
    }

    /// Adds what `other` lists and `self` does not. The optional DLC delete entries from
    /// `CfgMissions` (Contact deletes every other campaign), so the inventory merges the
    /// listing of the base game with the one of the game with optional DLC.
    pub fn merge(&mut self, other: Listing) {
        for (folder, entry) in other.folders {
            self.folders.entry(folder).or_insert(entry);
        }
        for campaign in other.campaigns {
            if !self.campaigns.contains(&campaign) {
                self.campaigns.push(campaign);
            }
        }
    }

    fn add_listed(&mut self, class: &ConfigRef<'_>, kind: Kind) {
        for entry in class.entries().into_iter().filter(|c| c.is_class()) {
            let directory = entry.get("directory");
            if directory.is_text() {
                let folder = normalize(&directory.text());
                self.folders
                    .entry(folder)
                    .or_insert((kind, entry.name().to_owned(), None));
            }
            self.add_listed(&entry, kind);
        }
    }

    fn add_campaign(
        &mut self,
        campaign: &ConfigRef<'_>,
        campaign_description: &mut impl FnMut(&str) -> Option<ConfigTree>,
    ) {
        let directory = campaign.get("directory");
        if !directory.is_text() {
            return;
        }
        let dir = normalize(&directory.text());
        let name = campaign.name().to_owned();
        self.campaigns.push(dir.clone());
        // The campaign folder itself may be the mission (Old Man).
        self.folders.entry(dir.clone()).or_insert((
            Kind::Campaign,
            name.clone(),
            Some(dir.clone()),
        ));
        let mut templates = Vec::new();
        let in_config = campaign.get("Campaign");
        if in_config.is_class() {
            collect_templates(&in_config, &mut templates);
        } else if let Some(tree) = campaign_description(&dir) {
            let class = tree.root().get("Campaign");
            if class.is_class() {
                collect_templates(&class, &mut templates);
            }
        }
        for template in templates {
            let folder = normalize(&format!("{dir}\\missions\\{template}"));
            self.folders
                .entry(folder)
                .or_insert((Kind::Campaign, name.clone(), Some(dir.clone())));
        }
    }

    /// The scenario for a mission folder found in the VFS.
    pub fn classify(&self, folder: &str) -> Scenario {
        let folder = normalize(folder);
        let (mut kind, listed_as, mut campaign) = match self.folders.get(&folder) {
            Some((kind, class, campaign)) => (*kind, Some(class.clone()), campaign.clone()),
            None => (self.unlisted_kind(&folder), None, None),
        };
        // A listed mission in a campaign's `missions` folder is a campaign mission, also when
        // another category lists it (Apex's co-op campaign is under `MPMissions`).
        if kind != Kind::Campaign
            && kind != Kind::Fragment
            && let Some(dir) = self
                .campaigns
                .iter()
                .find(|c| folder.starts_with(&format!("{c}\\missions\\")))
        {
            kind = Kind::Campaign;
            campaign = Some(dir.clone());
        }
        Scenario {
            world: world_of(&folder),
            package: package_of(&folder),
            folder,
            kind,
            listed_as,
            campaign,
            optional_mods: false,
        }
    }

    fn unlisted_kind(&self, folder: &str) -> Kind {
        let has = |part: &str| folder.split('\\').any(|p| p == part);
        if has("sites") {
            return Kind::Fragment;
        }
        // A sibling of a listed mission in a campaign's `missions` folder that no listing names
        // (Old Man, listed under `Missions`, keeps its per-area layers beside it).
        let parent = folder.rsplit_once('\\').map_or("", |(p, _)| p);
        let beside_campaign_mission = parent.ends_with("\\missions")
            && parent.split('\\').any(|p| p.starts_with("campaign"))
            && self
                .folders
                .keys()
                .any(|listed| listed.rsplit_once('\\').is_some_and(|(p, _)| p == parent));
        let in_campaign_missions = has("missions")
            && self
                .campaigns
                .iter()
                .any(|c| folder.starts_with(&format!("{c}\\")));
        if beside_campaign_mission || in_campaign_missions {
            return Kind::Fragment;
        }
        if has("scenes") || has("cutscenes") {
            return Kind::Cutscene;
        }
        Kind::Unlisted
    }
}

/// Every `template` text below a `class Campaign`.
fn collect_templates(class: &ConfigRef<'_>, out: &mut Vec<String>) {
    for entry in class.entries() {
        if entry.is_class() {
            let template = entry.get("template");
            if template.is_text() {
                let text = template.text();
                if !text.is_empty() && !out.contains(&text) {
                    out.push(text);
                }
            }
            collect_templates(&entry, out);
        }
    }
}

/// Mounts the loose mission PBOs of the game's `Missions` and `MPMissions` folders, each at
/// `missions\<file stem>` or `mpmissions\<file stem>`. Returns every mounted folder and its kind;
/// [`inventory`] keeps the ones that hold a `mission.sqm`, so a PBO that is not a mission is
/// mounted but never reported as one.
pub fn mount_loose_missions(vfs: &Vfs, game_dir: &Path) -> Vec<(String, Kind)> {
    let mut out = Vec::new();
    for (folder, kind) in [
        ("Missions", Kind::Scenario),
        ("MPMissions", Kind::Multiplayer),
    ] {
        let Ok(read) = std::fs::read_dir(game_dir.join(folder)) else {
            continue;
        };
        let mut files: Vec<_> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbo")))
            .collect();
        files.sort();
        for file in files {
            let Ok(pbo) = a3_pbo::Pbo::open(&file) else {
                continue;
            };
            let stem = file
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let prefix = format!("{}\\{stem}", folder.to_ascii_lowercase());
            vfs.mount_pbo(pbo, Some(VfsPath::new(&prefix)));
            out.push((prefix, kind));
        }
    }
    out
}

/// Every mission folder in the VFS (folders holding a `mission.sqm`), classified.
pub fn inventory(vfs: &Vfs, listing: &Listing, loose: &[(String, Kind)]) -> Vec<Scenario> {
    let mut out: Vec<Scenario> = vfs
        .glob("**/mission.sqm")
        .into_iter()
        .filter_map(|path| path.parent())
        .map(|folder| {
            let mut scenario = listing.classify(folder.as_str());
            if let Some((_, kind)) = loose.iter().find(|(f, _)| *f == scenario.folder) {
                scenario.kind = *kind;
                scenario.package = if *kind == Kind::Multiplayer {
                    "MPMissions".to_owned()
                } else {
                    "Missions".to_owned()
                };
            }
            scenario
        })
        .collect();
    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    out.dedup_by(|a, b| a.folder == b.folder);
    mark_nested_fragments(&mut out);
    out
}

/// A mission folder inside another mission folder (East Wind's `c_in1.stratis\ambience`) is a
/// part of that mission, not a scenario: marks it [`Kind::Fragment`].
pub fn mark_nested_fragments(scenarios: &mut [Scenario]) {
    let folders: std::collections::BTreeSet<String> =
        scenarios.iter().map(|s| s.folder.clone()).collect();
    for s in scenarios.iter_mut() {
        let mut parent = s.folder.as_str();
        while let Some((up, _)) = parent.rsplit_once('\\') {
            if folders.contains(up) {
                s.kind = Kind::Fragment;
                break;
            }
            parent = up;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
class CfgMissions {
    class Cutscenes { class Intro { directory = "A3\Map_X\Scenes\Intro1.Altis"; }; };
    class Campaigns {
        class Boot {
            directory = "A3\Missions_F_Bootcamp\Campaign";
            class Campaign {
                class MissionDefault {};
                class Missions {
                    class M01: MissionDefault { template = "boot_m01.Altis"; };
                    class M02: MissionDefault { template = "boot_m02.Altis"; };
                };
            };
        };
        class Apex { directory = "A3\Missions_F_Exp\Campaign"; };
        class Campaign2 { directory = "a3\Missions_F_X\Campaign\Missions\Whole.Tanoa"; };
    };
    class Missions {
        class OldMan { directory = "a3\Missions_F_Oldman\Campaign\Missions\Oldman.Tanoa"; };
    };
    class MPMissions {
        class Coop1 { directory = "A3\missions_f\mpscenarios\MP_COOP_m01.Stratis"; };
        class Apex { class Coop { directory = "A3\missions_f_exp\mpscenarios\Coop.Tanoa"; }; };
        class ApexCoop { directory = "A3\Missions_F_Exp\Campaign\Missions\EXP_m02.Tanoa"; };
    };
    class Challenges { class Drill { directory = "a3\missions_f_beta\challenges\drill.vr"; }; };
    class Showcases { class Arty { directory = "a3\missions_f\showcases\arty.altis"; }; };
};
"#;

    const APEX_DESCRIPTION: &str = r#"
class Campaign { class Chapter { class E01 { template = "exp_m01.Tanoa"; }; }; };
"#;

    fn listing() -> Listing {
        let config = ConfigTree::from_config(&a3_config::parse_text(CONFIG).unwrap());
        Listing::from_config(&config, |dir| {
            (dir == "a3\\missions_f_exp\\campaign")
                .then(|| ConfigTree::from_config(&a3_config::parse_text(APEX_DESCRIPTION).unwrap()))
        })
    }

    #[test]
    fn folders_normalise_and_name_their_world_and_package() {
        assert_eq!(
            normalize("/A3\\Missions_F\\x.Altis\\"),
            "a3\\missions_f\\x.altis"
        );
        assert_eq!(world_of("a3\\m\\boot_m02.Altis"), "altis");
        assert_eq!(world_of("a3\\m\\noext"), "");
        assert_eq!(
            package_of("A3\\Missions_F_EPA\\campaign\\x.altis"),
            "missions_f_epa"
        );
        assert_eq!(package_of("mpmissions\\x.altis"), "mpmissions");
    }

    #[test]
    fn listed_folders_take_their_category_at_any_depth() {
        let l = listing();
        let s = l.classify("a3\\missions_f\\mpscenarios\\mp_coop_m01.stratis");
        assert_eq!(
            (s.kind, s.listed_as.as_deref()),
            (Kind::Multiplayer, Some("Coop1"))
        );
        assert_eq!(s.world, "stratis");
        assert_eq!(
            l.classify("A3\\missions_f_exp\\mpscenarios\\Coop.Tanoa")
                .kind,
            Kind::Multiplayer
        );
        assert_eq!(
            l.classify("a3\\missions_f_beta\\challenges\\drill.vr").kind,
            Kind::Challenge
        );
        assert_eq!(
            l.classify("a3\\missions_f\\showcases\\arty.altis").kind,
            Kind::Showcase
        );
        assert_eq!(
            l.classify("a3\\map_x\\scenes\\intro1.altis").kind,
            Kind::Cutscene
        );
    }

    #[test]
    fn campaign_missions_come_from_templates_in_config_or_the_campaign_description() {
        let l = listing();
        let s = l.classify("a3\\missions_f_bootcamp\\campaign\\missions\\boot_m02.altis");
        assert_eq!(s.kind, Kind::Campaign);
        assert_eq!(s.listed_as.as_deref(), Some("Boot"));
        assert_eq!(
            s.campaign.as_deref(),
            Some("a3\\missions_f_bootcamp\\campaign")
        );
        let s = l.classify("a3\\missions_f_exp\\campaign\\missions\\exp_m01.tanoa");
        assert_eq!(
            (s.kind, s.listed_as.as_deref()),
            (Kind::Campaign, Some("Apex"))
        );
        // Listed elsewhere too (Apex is a co-op campaign under MPMissions): still a campaign.
        let s = l.classify("a3\\missions_f_exp\\campaign\\missions\\exp_m02.tanoa");
        assert_eq!(
            (s.kind, s.listed_as.as_deref()),
            (Kind::Campaign, Some("ApexCoop"))
        );
        assert_eq!(s.campaign.as_deref(), Some("a3\\missions_f_exp\\campaign"));
    }

    #[test]
    fn a_campaign_whose_directory_is_a_mission_is_that_mission_and_its_siblings_are_fragments() {
        let l = listing();
        let s = l.classify("a3\\missions_f_x\\campaign\\missions\\whole.tanoa");
        assert_eq!(s.kind, Kind::Campaign);
        // Old Man is listed as a scenario; the unlisted layers beside it are fragments.
        let s = l.classify("a3\\missions_f_oldman\\campaign\\missions\\oldman.tanoa");
        assert_eq!(s.kind, Kind::Scenario);
        let s = l.classify("a3\\missions_f_oldman\\campaign\\missions\\airsectors.tanoa");
        assert_eq!(s.kind, Kind::Fragment);
        // An unlisted folder of a campaign's missions folder is a fragment too.
        assert_eq!(
            l.classify("a3\\missions_f_bootcamp\\campaign\\missions\\boot_m99.altis")
                .kind,
            Kind::Fragment
        );
    }

    #[test]
    fn unlisted_folders_are_sites_scenes_or_just_unlisted() {
        let l = listing();
        assert_eq!(
            l.classify("a3\\missions_f_contact\\sites\\s01.enoch").kind,
            Kind::Fragment
        );
        assert_eq!(
            l.classify("a3\\map_y\\data\\scenes\\intro01.tanoa").kind,
            Kind::Cutscene
        );
        assert_eq!(
            l.classify("a3\\missions_f_contact\\missions\\c01.enoch")
                .kind,
            Kind::Unlisted
        );
    }

    #[test]
    fn a_merged_listing_keeps_what_a_dlc_deleted() {
        let mut base = listing();
        let with_dlc = Listing::from_config(
            &ConfigTree::from_config(
                &a3_config::parse_text(
                    r#"class CfgMissions { class Campaigns {
                        class C { directory = "a3\missions_f_c";
                            class Campaign { class M { class X { template = "x.enoch"; }; }; }; };
                    }; };"#,
                )
                .unwrap(),
            ),
            |_| None,
        );
        base.merge(with_dlc);
        assert_eq!(
            base.classify("a3\\missions_f_c\\missions\\x.enoch").kind,
            Kind::Campaign
        );
        assert_eq!(
            base.classify("a3\\missions_f_bootcamp\\campaign\\missions\\boot_m01.altis")
                .kind,
            Kind::Campaign
        );
        assert!(base.campaigns.contains(&"a3\\missions_f_c".to_owned()));
    }

    #[test]
    fn a_mission_folder_inside_another_is_a_fragment() {
        let mut scenarios = vec![
            Scenario::for_test("a3\\m\\c_in1.stratis"),
            Scenario::for_test("a3\\m\\c_in1.stratis\\ambience"),
            Scenario::for_test("a3\\m\\c_in2.stratis"),
        ];
        mark_nested_fragments(&mut scenarios);
        let kinds: Vec<Kind> = scenarios.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Unlisted, Kind::Fragment, Kind::Unlisted]);
    }

    #[test]
    fn kinds_parse_from_their_names() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::parse("mp"), Some(Kind::Multiplayer));
        assert_eq!(Kind::parse("nope"), None);
    }

    /// A PBO holding `files`, written where a game install keeps it.
    fn write_pbo(path: &std::path::Path, files: &[(&str, &[u8])]) {
        let mut pbo = a3_pbo::PboWriter::new().property("prefix", r"x\missions");
        for (name, bytes) in files {
            pbo = pbo.file(*name, bytes.to_vec());
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, pbo.to_bytes()).unwrap();
    }

    /// The loose mission PBOs of `Missions` and `MPMissions` are mounted under the PBO's file
    /// stem; only the folders that hold a `mission.sqm` are scenarios. Synthetic install: the test
    /// runs everywhere.
    #[test]
    fn loose_mission_pbos_are_mounted_at_their_stem_and_only_missions_are_scenarios() {
        let dir = tempfile::tempdir().unwrap();
        write_pbo(
            &dir.path().join("Missions").join("coop_x.stratis.pbo"),
            &[("mission.sqm", b"version=12;")],
        );
        write_pbo(
            &dir.path().join("MPMissions").join("mp_y.altis.pbo"),
            &[("mission.sqm", b"version=53;")],
        );
        write_pbo(
            &dir.path().join("Missions").join("readme.pbo"),
            &[("readme.txt", b"not a mission")],
        );

        let vfs = Vfs::new();
        let loose = mount_loose_missions(&vfs, dir.path());
        assert_eq!(
            loose,
            vec![
                (r"missions\coop_x.stratis".to_owned(), Kind::Scenario),
                (r"missions\readme".to_owned(), Kind::Scenario),
                (r"mpmissions\mp_y.altis".to_owned(), Kind::Multiplayer),
            ]
        );
        assert!(vfs.exists(r"missions\readme\readme.txt"));

        let scenarios = inventory(&vfs, &Listing::default(), &loose);
        assert_eq!(scenarios.len(), 2, "{scenarios:#?}");
        assert_eq!(
            (
                scenarios[0].folder.as_str(),
                scenarios[0].kind,
                scenarios[0].package.as_str(),
                scenarios[0].world.as_str(),
            ),
            (
                r"missions\coop_x.stratis",
                Kind::Scenario,
                "Missions",
                "stratis"
            )
        );
        assert_eq!(
            (
                scenarios[1].kind,
                scenarios[1].package.as_str(),
                scenarios[1].world.as_str(),
            ),
            (Kind::Multiplayer, "MPMissions", "altis")
        );
    }
}
