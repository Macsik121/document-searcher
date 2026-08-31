use anyhow::Result;
use ashpd::desktop::file_chooser::{FileChooser, FileChooserFlags};
use ashpd::desktop::open_uri::OpenURI;
use ashpd::WindowIdentifier;
use gtk4::glib;
use std::path::PathBuf;

/// Open a folder selection dialog using the FileChooser portal
pub async fn select_folder(parent: &gtk4::Window) -> Result<Option<PathBuf>> {
    let identifier = WindowIdentifier::from_native(parent)?;

    let chooser = FileChooser::new()
        .title("Select Folder")
        .flags(FileChooserFlags::MODAL | FileChooserFlags::SELECT_FOLDERS);

    let response = chooser.select_folder(&identifier, "Select a folder to search").await?;

    Ok(response.folders().first().cloned())
}

/// Open a file with the default application using the OpenURI portal
pub async fn open_file(path: &PathBuf) -> Result<()> {
    let uri = format!("file://{}", path.display());
    OpenURI::open_uri(&uri).await?;
    Ok(())
}

/// Open a folder in the file manager
pub async fn open_folder(path: &PathBuf) -> Result<()> {
    let uri = format!("file://{}", path.display());
    OpenURI::open_uri(&uri).await?;
    Ok(())
}

pub fn setup_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}