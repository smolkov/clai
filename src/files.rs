//! File modes shared by the places that create private files.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Sets the mode of `path` to `mode` (`0o700` for a directory, `0o600`
/// for a file) unless it already is.
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    if fs::metadata(path)?.permissions().mode() & 0o777 != mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn sets_the_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        fs::write(&file, "").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        set_mode(&file, 0o600).unwrap();
        assert_eq!(mode(&file), 0o600);
        set_mode(&file, 0o600).unwrap();
        assert_eq!(mode(&file), 0o600);
        assert!(set_mode(&tmp.path().join("missing"), 0o600).is_err());
    }
}
