#[derive(Clone, Copy)]
pub(crate) struct AppDbEntry {
    pub(crate) executables: &'static [AppDbExecutable],
    pub(crate) required_assets: &'static [AppDbRequiredAsset],
}

#[derive(Clone, Copy)]
pub(crate) struct AppDbExecutable {
    pub(crate) filename: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct AppDbRequiredAsset {
    pub(crate) name: &'static str,
    pub(crate) asset_type: &'static str,
    pub(crate) locator: &'static str,
    pub(crate) mount: Option<AppDbMount>,
}

#[derive(Clone, Copy)]
pub(crate) struct AppDbMount {
    pub(crate) drive: char,
    pub(crate) device: &'static str,
}

include!("app_db_generated.rs");

pub(crate) fn find_by_exe_path(path: &str) -> Option<&'static AppDbEntry> {
    let filename = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    APP_DB.iter().find(|entry| {
        entry
            .executables
            .iter()
            .any(|exe| exe.filename.eq_ignore_ascii_case(&filename))
    })
}
