use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::{Error, Result};

const ZIP_MOUNT_ROOT: &str = "C:\\mnt";

pub(crate) struct ZipEntryData {
    pub(crate) guest_path: String,
    pub(crate) data: Vec<u8>,
}

pub(crate) struct ZipMountData {
    pub(crate) files: Vec<ZipEntryData>,
    pub(crate) exes: Vec<String>,
}

pub(crate) fn read_zip(path: &Path) -> Result<ZipMountData> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| Error::Cli(format!("failed to open ZIP {}: {err}", path.display())))?;
    let mut files = Vec::new();
    let mut exes = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            Error::Cli(format!(
                "failed to read ZIP entry {index} in {}: {err}",
                path.display()
            ))
        })?;
        if entry.is_dir() {
            continue;
        }
        let Some(archive_path) = normalize_archive_path(entry.name()) else {
            continue;
        };
        let guest_path = guest_path_for_archive_path(&archive_path);
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|err| {
            Error::Cli(format!(
                "failed to extract ZIP entry {} from {}: {err}",
                entry.name(),
                path.display()
            ))
        })?;
        if archive_path.to_ascii_lowercase().ends_with(".exe") {
            exes.push(guest_path.clone());
        }
        files.push(ZipEntryData { guest_path, data });
    }

    Ok(ZipMountData { files, exes })
}

fn guest_path_for_archive_path(path: &str) -> String {
    format!("{ZIP_MOUNT_ROOT}\\{}", path.replace('/', "\\"))
}

fn normalize_archive_path(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| *part == "." || *part == ".." || part.contains(':'))
    {
        return None;
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::{guest_path_for_archive_path, normalize_archive_path};

    #[test]
    fn normalizes_browser_zip_paths() {
        assert_eq!(
            normalize_archive_path("Game\\rich4.exe").as_deref(),
            Some("Game/rich4.exe")
        );
        assert_eq!(
            normalize_archive_path("/Game//Data/file.dat").as_deref(),
            Some("Game/Data/file.dat")
        );
        assert_eq!(normalize_archive_path("../rich4.exe"), None);
        assert_eq!(normalize_archive_path("C:/rich4.exe"), None);
    }

    #[test]
    fn maps_archive_paths_under_browser_mount_root() {
        assert_eq!(
            guest_path_for_archive_path("Game/rich4.exe"),
            "C:\\mnt\\Game\\rich4.exe"
        );
    }
}
