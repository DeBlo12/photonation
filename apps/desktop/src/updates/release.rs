// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! What GitHub says the latest release is, and which of its files installs
//! on this computer. Nothing here touches the network: it reads the answer
//! `transfer` fetched. See docs/DESKTOP.md.

use serde::Deserialize;
use std::fmt;

/// Where the releases are: the repository in `Cargo.toml`.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// No installer is anywhere near this; a release claiming more is refused
/// before anything is downloaded.
pub const LARGEST_INSTALLER_BYTES: u64 = 1 << 30;

/// As much of a release's notes as the window shows.
const NOTES_CHARACTERS: usize = 4000;

/// A released version: three numbers, as in `Cargo.toml` and the `v…` tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// `1.2.3`, and nothing else: no `v`, no pre-release suffix, no spaces.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let mut number = || -> Option<u32> {
            let part = parts.next()?;
            let only_digits = !part.is_empty()
                && part.len() <= 9
                && part.bytes().all(|byte| byte.is_ascii_digit());
            only_digits.then(|| part.parse().ok()).flatten()
        };
        let version = Version {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// The version of the running app.
    pub fn running() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version is three numbers")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The kinds of computer an installer is built for (packaging/).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `architecture` as `uname -m` says it, which names the disk image.
    MacOs {
        architecture: &'static str,
    },
    Windows,
    /// No installer: the app was built from source.
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            let architecture = if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            };
            Platform::MacOs { architecture }
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Platform::Windows
        } else {
            Platform::Other
        }
    }

    /// The installer's file name, as the packaging scripts write it.
    pub fn installer_name(self, version: Version) -> Option<String> {
        match self {
            Platform::MacOs { architecture } => {
                Some(format!("photonation-{version}-macos-{architecture}.dmg"))
            }
            Platform::Windows => Some(format!("photonation-{version}-windows-x64-setup.exe")),
            Platform::Other => None,
        }
    }
}

/// The installer for this computer among a release's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub file_name: String,
    /// Where it downloads from. Built from the repository, the version and
    /// the file name, never taken from what GitHub answered.
    pub url: String,
    pub bytes: u64,
    /// The checksum GitHub recorded when the file was uploaded: 64 lower-case
    /// hexadecimal digits.
    pub sha256: String,
}

/// The latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// What changed, as the release's page says it.
    pub notes: String,
    pub page_url: String,
    /// `None` when the release has no installer for this computer.
    pub installer: Option<Installer>,
}

/// The fields of GitHub's "get the latest release" answer that are used.
#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

impl Release {
    /// Reads GitHub's answer. An answer that isn't a published release of
    /// this project with a `v<version>` tag is an error; an installer that
    /// can't be verified is left out, so the release still shows.
    pub fn from_json(json: &str, platform: Platform) -> Result<Release, String> {
        let release: GithubRelease = serde_json::from_str(json)
            .map_err(|error| format!("GitHub's answer wasn't a release: {error}"))?;
        if release.draft || release.prerelease {
            return Err("GitHub's latest release isn't a published one.".into());
        }
        let version = release
            .tag_name
            .strip_prefix('v')
            .and_then(Version::parse)
            .ok_or_else(|| {
                let tag: String = release.tag_name.chars().take(40).collect();
                format!("The latest release's tag isn't a version: {tag}")
            })?;
        let installer = platform.installer_name(version).and_then(|file_name| {
            let asset = release
                .assets
                .iter()
                .find(|asset| asset.name == file_name)?;
            let sha256 = sha256_of(asset.digest.as_deref()?)?;
            let plausible_size = (1..=LARGEST_INSTALLER_BYTES).contains(&asset.size);
            plausible_size.then(|| Installer {
                url: format!("{REPOSITORY}/releases/download/v{version}/{file_name}"),
                file_name,
                bytes: asset.size,
                sha256,
            })
        });
        let releases_url = format!("{REPOSITORY}/releases");
        let page_url = if release.html_url.starts_with(&format!("{releases_url}/")) {
            release.html_url
        } else {
            releases_url
        };
        Ok(Release {
            version,
            notes: shown_notes(release.body.as_deref().unwrap_or_default()),
            page_url,
            installer,
        })
    }
}

/// The checksum in GitHub's `sha256:<64 hexadecimal digits>`.
fn sha256_of(digest: &str) -> Option<String> {
    let hexadecimal = digest.strip_prefix("sha256:")?;
    let well_formed = hexadecimal.len() == 64
        && hexadecimal
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    well_formed.then(|| hexadecimal.to_owned())
}

/// The notes with Unix line ends, cut to what the window shows.
fn shown_notes(body: &str) -> String {
    let notes = body.replace("\r\n", "\n");
    let notes = notes.trim();
    match notes.char_indices().nth(NOTES_CHARACTERS) {
        Some((cut, _)) => format!("{}…", &notes[..cut]),
        None => notes.to_owned(),
    }
}

/// The address that answers with the latest published release.
pub fn latest_release_api_url() -> Option<String> {
    let owner_and_name = REPOSITORY.strip_prefix("https://github.com/")?;
    Some(format!(
        "https://api.github.com/repos/{owner_and_name}/releases/latest"
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const MAC: Platform = Platform::MacOs {
        architecture: "arm64",
    };
    pub const CHECKSUM: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";

    /// GitHub's answer for a release with the two installers `version` has.
    pub fn answer(version: &str) -> String {
        serde_json::json!({
            "tag_name": format!("v{version}"),
            "html_url": format!("{REPOSITORY}/releases/tag/v{version}"),
            "draft": false,
            "prerelease": false,
            "body": "### Added\r\n\r\n- Something new.\r\n",
            "assets": [
                {
                    "name": format!("photonation-{version}-macos-arm64.dmg"),
                    "size": 40_000_000,
                    "digest": format!("sha256:{CHECKSUM}"),
                    "browser_download_url": "https://example.com/elsewhere.dmg",
                },
                {
                    "name": format!("photonation-{version}-windows-x64-setup.exe"),
                    "size": 30_000_000,
                    "digest": format!("sha256:{CHECKSUM}"),
                },
                {
                    "name": format!("photonation-{version}-macos-arm64.dmg.sha256"),
                    "size": 100,
                },
            ],
        })
        .to_string()
    }

    #[test]
    fn versions_are_three_numbers_and_compare_by_them() {
        let version = |text| Version::parse(text).expect(text);
        assert!(version("0.27.0") > version("0.26.9"));
        assert!(version("0.9.0") < version("0.10.0"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert_eq!(version("0.27.0").to_string(), "0.27.0");
        for not_a_version in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "1.2.3-beta",
            "1.2.x",
            "1..3",
            " 1.2.3",
            "1.2.+3",
            "1.2.99999999999",
        ] {
            assert_eq!(Version::parse(not_a_version), None, "{not_a_version:?}");
        }
    }

    #[test]
    fn the_running_version_and_the_repository_are_understood() {
        assert_eq!(Version::running().to_string(), env!("CARGO_PKG_VERSION"));
        let url = latest_release_api_url().expect("the repository is on GitHub");
        assert!(url.starts_with("https://api.github.com/repos/"));
        assert!(url.ends_with("/releases/latest"));
    }

    #[test]
    fn a_release_offers_the_installer_for_this_computer() {
        let release = Release::from_json(&answer("0.30.0"), MAC).expect("a release");
        assert_eq!(release.version.to_string(), "0.30.0");
        assert_eq!(release.notes, "### Added\n\n- Something new.");
        assert_eq!(
            release.page_url,
            format!("{REPOSITORY}/releases/tag/v0.30.0")
        );
        let installer = release.installer.expect("the disk image");
        assert_eq!(installer.file_name, "photonation-0.30.0-macos-arm64.dmg");
        assert_eq!(installer.bytes, 40_000_000);
        assert_eq!(installer.sha256, CHECKSUM);

        let windows = Release::from_json(&answer("0.30.0"), Platform::Windows).expect("a release");
        let installer = windows.installer.expect("the setup program");
        assert_eq!(
            installer.file_name,
            "photonation-0.30.0-windows-x64-setup.exe"
        );
    }

    #[test]
    fn the_download_address_never_comes_from_the_answer() {
        let release = Release::from_json(&answer("0.30.0"), MAC).expect("a release");
        assert_eq!(
            release.installer.expect("the disk image").url,
            format!("{REPOSITORY}/releases/download/v0.30.0/photonation-0.30.0-macos-arm64.dmg")
        );
    }

    #[test]
    fn a_release_without_a_verifiable_installer_still_shows() {
        let intel = Platform::MacOs {
            architecture: "x86_64",
        };
        for platform in [intel, Platform::Other] {
            let release = Release::from_json(&answer("0.30.0"), platform).expect("a release");
            assert_eq!(release.installer, None);
        }
        let name = "photonation-0.30.0-macos-arm64.dmg";
        for asset in [
            serde_json::json!({ "name": name, "size": 40_000_000 }),
            serde_json::json!({ "name": name, "size": 40_000_000, "digest": "sha256:abc" }),
            serde_json::json!({ "name": name, "size": 40_000_000, "digest": format!("md5:{CHECKSUM}") }),
            serde_json::json!({ "name": name, "size": 40_000_000,
                "digest": format!("sha256:{}", CHECKSUM.to_uppercase()) }),
            serde_json::json!({ "name": name, "size": 0, "digest": format!("sha256:{CHECKSUM}") }),
            serde_json::json!({ "name": name, "size": LARGEST_INSTALLER_BYTES + 1,
                "digest": format!("sha256:{CHECKSUM}") }),
        ] {
            let json = serde_json::json!({ "tag_name": "v0.30.0", "assets": [asset] }).to_string();
            let release = Release::from_json(&json, MAC).expect("a release");
            assert_eq!(release.installer, None, "{json}");
        }
    }

    #[test]
    fn answers_that_are_not_a_release_of_this_project_are_refused() {
        for json in [
            "",
            "<html>",
            r#"{"message": "Not Found"}"#,
            r#"{"tag_name": "nightly"}"#,
            r#"{"tag_name": "v1.2.3-rc1"}"#,
            r#"{"tag_name": "v1.2.3", "draft": true}"#,
            r#"{"tag_name": "v1.2.3", "prerelease": true}"#,
        ] {
            assert!(Release::from_json(json, MAC).is_err(), "{json}");
        }
    }

    #[test]
    fn a_page_elsewhere_is_replaced_by_the_releases_page() {
        let json =
            r#"{"tag_name": "v1.2.3", "html_url": "https://example.com/releases/tag/v1.2.3"}"#;
        let release = Release::from_json(json, MAC).expect("a release");
        assert_eq!(release.page_url, format!("{REPOSITORY}/releases"));
    }

    #[test]
    fn long_notes_are_cut_between_characters() {
        let json =
            serde_json::json!({ "tag_name": "v1.2.3", "body": "é".repeat(5000) }).to_string();
        let release = Release::from_json(&json, MAC).expect("a release");
        assert_eq!(release.notes.chars().count(), NOTES_CHARACTERS + 1);
        assert!(release.notes.ends_with('…'));
    }
}
