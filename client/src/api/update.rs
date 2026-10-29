//! Aggiornamento dell'app: il backend legge l'ultima release e serve il pacchetto di questa
//! piattaforma. Il client chiede tutto al backend, come per il resto.

use std::path::PathBuf;

use serde::Deserialize;

/// La release da cui è stata compilata questa copia: il workflow la imposta da `github.ref_name`.
/// Senza (compilazione locale) non si controlla nulla.
pub const BUILD_TAG: Option<&str> = option_env!("PSYTIFY_BUILD_TAG");

/// L'ultima release, come la serve il backend.
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

/// Un aggiornamento da proporre: la release e il pacchetto di questa piattaforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub tag: String,
    pub asset: Asset,
}

/// L'offerta, se la release è diversa da quella di questa copia e ha un pacchetto per questa
/// piattaforma. Senza `build` (compilazione locale) non si offre nulla.
pub fn offer(release: Release, build: Option<&str>) -> Option<Offer> {
    let build = build?;
    if release.tag == build {
        return None;
    }
    let asset = release.assets.into_iter().find(|asset| asset.name.ends_with(package_suffix()))?;
    Some(Offer { tag: release.tag, asset })
}

/// Il suffisso del pacchetto di questa piattaforma, come lo chiama il workflow di release.
fn package_suffix() -> &'static str {
    if cfg!(target_os = "android") {
        "-android.apk"
    } else if cfg!(windows) {
        "-windows-x86_64.zip"
    } else {
        "-linux-x86_64.tar.gz"
    }
}

/// La cartella dei pacchetti scaricati: dentro ai dati dell'app, non nella cartella Download.
pub fn updates_dir() -> PathBuf {
    crate::config::dir().unwrap_or_else(std::env::temp_dir).join("updates")
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
    fn a_release_without_this_platforms_package_is_not_offered() {
        let other = Release { tag: "v3".into(), assets: vec![Asset { name: "x-other.bin".into(), size: 1 }] };
        assert_eq!(offer(other, Some("v2")), None);
    }
}
