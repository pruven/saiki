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
    parse_release(
        &serde_json::from_str(&body)?,
        &current,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn parse_release(
    release: &Value,
    current: &Version,
    os: &str,
    arch: &str,
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
        assets
            .iter()
            .filter_map(|asset| asset["name"].as_str())
            .find(|name| matches_package(name, &version, os, arch))
            .map(|name| format!("{RELEASES}/download/{tag}/{name}"))
    });
    Ok(Some(Release {
        version,
        page_url: format!("{RELEASES}/tag/{tag}"),
        download_url,
    }))
}

fn matches_package(name: &str, version: &Version, os: &str, arch: &str) -> bool {
    match (os, arch) {
        ("windows", "x86_64") => name == "saiki-windows-x86_64.zip",
        ("windows", "aarch64") => name == "saiki-windows-arm64.zip",
        ("linux", "x86_64" | "aarch64") => {
            let prefix = format!("saiki-{version}-");
            let suffix = format!("-{arch}.pkg.tar.zst");
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(&suffix))
                .is_some_and(|pkgrel| {
                    !pkgrel.is_empty()
                        && pkgrel.split('.').all(|part| {
                            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                        })
                })
        }
        _ => false,
    }
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
                {"name": "saiki-windows-x86_64.zip"},
                {"name": "saiki-windows-arm64.zip"}
            ]
        });
        let current = Version::new(0, 9, 0);
        for (os, arch, asset) in [
            ("linux", "x86_64", Some("saiki-0.10.0-1-x86_64.pkg.tar.zst")),
            ("linux", "aarch64", None),
            ("windows", "x86_64", Some("saiki-windows-x86_64.zip")),
            ("windows", "aarch64", Some("saiki-windows-arm64.zip")),
        ] {
            let release = parse_release(&metadata, &current, os, arch)
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
                parse_release(&metadata, &current, "windows", "x86_64")
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            parse_release(&json!({"tag_name": "invalid"}), &current, "linux", "x86_64").is_err()
        );
    }
}
