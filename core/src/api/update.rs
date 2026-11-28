//! App update: the backend reads the latest release and serves the package for this
//! platform. The client asks the backend for everything, as for the rest.

use serde::Deserialize;

/// The release this copy was built from: the workflow sets it from `github.ref_name`.
/// Without it (local build) nothing is checked.
pub const BUILD_TAG: Option<&str> = option_env!("PSYTIFY_BUILD_TAG");

/// The latest release, as the backend serves it.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
}

/// An update to offer: the release and this platform's package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub tag: String,
    pub asset: Asset,
}

/// The offer, if the release differs from this copy's and has a package for this
/// platform. Without `build` (local build) nothing is offered.
pub fn offer(release: Release, build: Option<&str>) -> Option<Offer> {
    let build = build?;
    if release.tag == build {
        return None;
    }
    let asset = release.assets.into_iter().find(|asset| asset.name.ends_with(package_suffix()))?;
    Some(Offer { tag: release.tag, asset })
}

/// The suffix of this platform's package, as the release workflow names it.
fn package_suffix() -> &'static str {
    if cfg!(target_os = "android") {
        "-android.apk"
    } else if cfg!(windows) {
        "-windows-x86_64.zip"
    } else {
        "-linux-x86_64.tar.gz"
    }
}

/// The name of a package, if it is a simple file name: letters, digits, `.`, `_` and `-`, without a
/// leading dot, at most 128 characters. The name comes from the server and ends up in a path and in
/// a URL: `../x`, a separator or a `?` do not pass.
pub fn safe_asset_name(name: &str) -> Option<&str> {
    let simple = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    simple.then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag: tag.into(),
            assets: vec![
                Asset { name: format!("psytify-{tag}-android.apk"), size: 1 },
                Asset { name: format!("psytify-{tag}-windows-x86_64.zip"), size: 1 },
                Asset { name: format!("psytify-{tag}-linux-x86_64.tar.gz"), size: 1 },
            ],
        }
    }

    #[test]
    fn offers_only_a_different_release_with_this_platforms_package() {
        assert_eq!(offer(release("v2"), None), None, "una compilazione locale non controlla");
        assert_eq!(offer(release("v2"), Some("v2")), None, "la stessa release non si ripropone");
        let offered = offer(release("v3"), Some("v2")).expect("una release nuova con pacchetto");
        assert_eq!(offered.tag, "v3");
        assert!(offered.asset.name.ends_with(package_suffix()));
    }

    #[test]
    fn only_simple_file_names_are_accepted_for_an_asset() {
        assert_eq!(safe_asset_name("psytify-v3-linux-x86_64.tar.gz"), Some("psytify-v3-linux-x86_64.tar.gz"));
        for bad in ["", "../x", "a/b", "a\\b", ".hidden", "a?b", "a b", &"x".repeat(129)] {
            assert_eq!(safe_asset_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_release_without_this_platforms_package_is_not_offered() {
        let other = Release { tag: "v3".into(), assets: vec![Asset { name: "x-other.bin".into(), size: 1 }] };
        assert_eq!(offer(other, Some("v2")), None);
    }
}
