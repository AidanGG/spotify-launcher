use crate::crypto;
use crate::deb::{self, Pkg};
use crate::errors::*;
use crate::http;
use crate::pgp;
use std::fs;
use std::path::Path;

pub struct Client {
    client: http::Client,
}

impl Client {
    pub fn new() -> Result<Client> {
        let client = http::Client::new()?;
        Ok(Client { client })
    }

    pub async fn fetch_pkg_release(&self, keyring_path: &Path) -> Result<Pkg> {
        info!("Downloading release file...");
        let release = self
            .client
            .fetch("http://repository.spotify.com/dists/testing/Release")
            .await?;

        info!("Downloading signature...");
        let sig = self
            .client
            .fetch("http://repository.spotify.com/dists/testing/Release.gpg")
            .await?;

        info!("Verifying pgp signature...");
        let tmp = tempfile::tempdir().context("Failed to create temporary directory")?;
        let tmp_path = tmp.path();

        let artifact_path = tmp_path.join("artifact");
        fs::write(&artifact_path, &release)?;
        let sig_path = tmp_path.join("sig");
        fs::write(&sig_path, &sig)?;

        pgp::verify_sig::<&Path>(&sig_path, &artifact_path, keyring_path).await?;

        info!("Signature verified successfully!");
        let release = deb::parse_release_file(&String::from_utf8(release)?)?;
        let packages_sha256sum = release
            .get("non-free/binary-amd64/Packages")
            .context("Missing sha256sum for package index")?;

        info!("Downloading package index...");
        let pkg_index = self
            .client
            .fetch("http://repository.spotify.com/dists/testing/non-free/binary-amd64/Packages")
            .await?;

        info!("Verifying with sha256sum hash...");
        let downloaded_sha256sum = crypto::sha256sum(&pkg_index);
        if *packages_sha256sum != downloaded_sha256sum {
            bail!(
                "Downloaded bytes don't match signed sha256sum (signed: {:?}, downloaded: {:?})",
                packages_sha256sum,
                downloaded_sha256sum
            );
        }

        let pkg_index = deb::parse_package_index(&String::from_utf8(pkg_index)?)?;
        debug!("Parsed package index: {:?}", pkg_index);
        let pkg = pkg_index
            .into_iter()
            .find(|p| p.package == "spotify-client")
            .context("Repository didn't contain spotify-client")?;

        debug!("Found package: {:?}", pkg);
        Ok(pkg)
    }

    pub async fn download_pkg(&self, pkg: &Pkg) -> Result<Vec<u8>> {
        let filename = pkg
            .filename
            .rsplit_once('/')
            .map(|(_, x)| x)
            .unwrap_or("???");

        info!(
            "Downloading deb file for {:?} version={:?} ({:?})",
            filename, pkg.package, pkg.version
        );
        let url = format!("http://repository.spotify.com/{}", pkg.filename);
        let deb = self.client.fetch(&url).await?;

        info!("Verifying with sha256sum hash...");
        let downloaded_sha256sum = crypto::sha256sum(&deb);
        if pkg.sha256sum != downloaded_sha256sum {
            bail!(
                "Downloaded bytes don't match signed sha256sum (signed: {:?}, downloaded: {:?})",
                pkg.sha256sum,
                downloaded_sha256sum
            );
        }

        Ok(deb)
    }
}
