use std::path::{Path, PathBuf};

/// `cargo xtask install` bundles the plugin (release) and copies it into the user's CLAP folder.
/// Everything else goes to nih-plug's xtask (`cargo xtask bundle strings-plugin --release`).
fn main() -> nih_plug_xtask::Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) != Some("install") {
        return nih_plug_xtask::main_with_args("cargo xtask", args);
    }

    let extra = args.skip(1);
    let bundle_args = ["bundle", "strings-plugin", "--release"]
        .into_iter()
        .map(String::from)
        .chain(extra);
    nih_plug_xtask::main_with_args("cargo xtask", bundle_args)?;

    // main_with_args moved us to the workspace root.
    let bundle = Path::new("target/bundled/LibreStrings.clap");
    let dest_dir = clap_dir()?;
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join("LibreStrings.clap");
    copy_bundle(bundle, &dest)?;
    println!("Installed {} to {}", bundle.display(), dest.display());
    Ok(())
}

/// The per-user CLAP folder: `~/.clap` on Linux, `~/Library/Audio/Plug-Ins/CLAP` on macOS,
/// `%LOCALAPPDATA%\Programs\Common\CLAP` on Windows.
fn clap_dir() -> nih_plug_xtask::Result<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let dir = if cfg!(target_os = "windows") {
        var("LOCALAPPDATA").map(|d| d.join("Programs").join("Common").join("CLAP"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|d| d.join("Library/Audio/Plug-Ins/CLAP"))
    } else {
        var("HOME").map(|d| d.join(".clap"))
    };
    dir.ok_or_else(|| std::io::Error::other("cannot find the home directory").into())
}

/// Copies a bundle (a file on Linux and Windows, a directory on macOS). A file is written next to
/// the destination and renamed over it, so a host that has the old one loaded keeps its mapping
/// instead of crashing on a file rewritten under it.
fn copy_bundle(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_bundle(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        let tmp = dest.with_extension("clap.tmp");
        std::fs::copy(src, &tmp)?;
        std::fs::rename(&tmp, dest)
    }
}
