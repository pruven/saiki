use anyhow::{Context, Result};
use semver::Version;
use serde_json::Value;
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

const LATEST_RELEASE: &str = "https://api.github.com/repos/pruven/saiki/releases/latest";
const RELEASES: &str = "https://github.com/pruven/saiki/releases";

pub struct Release {
    pub version: Version,
    pub page_url: String,
    pub download_url: Option<String>,
}

pub fn check() -> Result<Option<Release>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut response = match agent
        .get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("Saiki/", env!("CARGO_PKG_VERSION")))
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(error) => return Err(error).context("check GitHub for Saiki updates"),
    };
    let body = response
        .body_mut()
        .with_config()
        .limit(512 * 1024)
        .read_to_string()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    #[cfg(target_os = "linux")]
    let prefer_arch_package = std::fs::read_to_string("/etc/os-release")
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => std::fs::read_to_string("/usr/lib/os-release"),
            _ => Err(error),
        })
        .is_ok_and(|release| is_arch_family(&release));
    #[cfg(not(target_os = "linux"))]
    let prefer_arch_package = false;
    parse_release(
        &serde_json::from_str(&body)?,
        &current,
        std::env::consts::OS,
        std::env::consts::ARCH,
        prefer_arch_package,
    )
}

fn parse_release(
    release: &Value,
    current: &Version,
    os: &str,
    arch: &str,
    prefer_arch_package: bool,
) -> Result<Option<Release>> {
    if release["draft"] == true || release["prerelease"] == true {
        return Ok(None);
    }
    let tag = release["tag_name"].as_str().context("release has no tag")?;
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))?;
    if !version.pre.is_empty() || version.cmp_precedence(current).is_le() {
        return Ok(None);
    }

    let download_url = release["assets"].as_array().and_then(|assets| {
        let names = assets.iter().filter_map(|asset| asset["name"].as_str());
        let package =
            if os == "linux" && prefer_arch_package && matches!(arch, "x86_64" | "aarch64") {
                let prefix = format!("saiki-{version}-");
                let suffix = format!("-{arch}.pkg.tar.zst");
                names
                    .clone()
                    .find(|name| matches_arch_package(name, &prefix, &suffix))
            } else {
                None
            };
        package
            .or_else(|| {
                let expected = match (os, arch) {
                    ("windows", "x86_64") => "saiki-windows-x86_64.zip",
                    ("windows", "aarch64") => "saiki-windows-arm64.zip",
                    ("linux", "x86_64") => "saiki-linux-x86_64.tar.gz",
                    ("linux", "aarch64") => "saiki-linux-aarch64.tar.gz",
                    _ => return None,
                };
                names.into_iter().find(|name| *name == expected)
            })
            .map(|name| format!("{RELEASES}/download/{tag}/{name}"))
    });
    Ok(Some(Release {
        version,
        page_url: format!("{RELEASES}/tag/{tag}"),
        download_url,
    }))
}

fn matches_arch_package(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .is_some_and(|pkgrel| {
            pkgrel
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

#[cfg(any(target_os = "linux", test))]
fn is_arch_family(os_release: &str) -> bool {
    os_release
        .lines()
        .filter_map(|line| line.split_once('='))
        .any(|(key, value)| {
            let value = value.trim_matches(['"', '\'']);
            match key {
                "ID" => matches!(value, "arch" | "archarm"),
                "ID_LIKE" => value
                    .split_ascii_whitespace()
                    .any(|id| matches!(id, "arch" | "archarm")),
                _ => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_the_correct_package_and_keeps_notes_available_without_one() {
        let metadata = json!({
            "tag_name": "v0.10.0",
            "assets": [
                {"name": "saiki-0.10.0-fake-x86_64.pkg.tar.zst"},
                {"name": "saiki-0.10.0-1-x86_64.pkg.tar.zst"},
                {"name": "saiki-0.10.0-1-aarch64.pkg.tar.zst"},
                {"name": "saiki-linux-x86_64.tar.gz"},
                {"name": "saiki-linux-aarch64.tar.gz"},
                {"name": "saiki-windows-x86_64.zip"},
                {"name": "saiki-windows-arm64.zip"}
            ]
        });
        let current = Version::new(0, 9, 0);
        for (os, arch, arch_family, asset) in [
            (
                "linux",
                "x86_64",
                true,
                Some("saiki-0.10.0-1-x86_64.pkg.tar.zst"),
            ),
            (
                "linux",
                "aarch64",
                true,
                Some("saiki-0.10.0-1-aarch64.pkg.tar.zst"),
            ),
            ("linux", "x86_64", false, Some("saiki-linux-x86_64.tar.gz")),
            (
                "linux",
                "aarch64",
                false,
                Some("saiki-linux-aarch64.tar.gz"),
            ),
            ("linux", "riscv64", false, None),
            ("windows", "x86_64", false, Some("saiki-windows-x86_64.zip")),
            ("windows", "aarch64", false, Some("saiki-windows-arm64.zip")),
        ] {
            let release = parse_release(&metadata, &current, os, arch, arch_family)
                .unwrap()
                .unwrap();
            assert_eq!(release.version, Version::new(0, 10, 0));
            assert_eq!(release.page_url, format!("{RELEASES}/tag/v0.10.0"));
            assert_eq!(
                release.download_url,
                asset.map(|name| format!("{RELEASES}/download/v0.10.0/{name}"))
            );
        }
    }

    #[test]
    fn recognizes_arch_distributions_and_derivatives() {
        for release in [
            "ID=arch",
            "ID=archarm",
            "ID=omarchy\nID_LIKE=arch",
            "ID_LIKE=\"other arch\"",
        ] {
            assert!(is_arch_family(release), "{release}");
        }
        assert!(!is_arch_family("ID=ubuntu\nID_LIKE=debian"));
        assert!(!is_arch_family("NAME=Arch Linux"));
    }

    #[test]
    fn handles_releases_with_only_one_linux_package_format() {
        let current = Version::new(0, 9, 0);
        for (asset, prefer_arch, available) in [
            ("saiki-0.10.0-1-x86_64.pkg.tar.zst", false, false),
            ("saiki-linux-x86_64.tar.gz", true, true),
        ] {
            let release = json!({"tag_name":"v0.10.0","assets":[{"name":asset}]});
            assert_eq!(
                parse_release(&release, &current, "linux", "x86_64", prefer_arch)
                    .unwrap()
                    .unwrap()
                    .download_url
                    .is_some(),
                available
            );
        }
    }

    #[test]
    fn skips_current_older_and_unstable_releases() {
        let current = Version::new(1, 0, 0);
        for metadata in [
            json!({"tag_name": "v1.0.0"}),
            json!({"tag_name": "v1.0.0+rebuild"}),
            json!({"tag_name": "v0.10.0"}),
            json!({"tag_name": "v2.0.0-beta.1"}),
            json!({"tag_name": "v2.0.0", "draft": true}),
            json!({"tag_name": "v2.0.0", "prerelease": true}),
        ] {
            assert!(
                parse_release(&metadata, &current, "windows", "x86_64", false)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            parse_release(
                &json!({"tag_name": "invalid"}),
                &current,
                "linux",
                "x86_64",
                false
            )
            .is_err()
        );
    }
}
