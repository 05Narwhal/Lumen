//! Free space on the drive holding a path (round 3), for the import's 20 GB buffer.

use std::path::{Path, PathBuf};

/// The free bytes of the disk whose mount point is the longest prefix of `path`.
pub fn pick_disk(disks: &[(PathBuf, u64)], path: &Path) -> Option<u64> {
    // `Path::starts_with` compares whole components, so `/Volumes/Photo` never claims
    // `/Volumes/Photos`.
    disks
        .iter()
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.components().count())
        .map(|(_, free)| *free)
}

/// Free bytes on the drive holding `path`, as the system reports it now.
pub fn free_space(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let list: Vec<(PathBuf, u64)> = disks.list().iter().map(|d| (d.mount_point().to_path_buf(), d.available_space())).collect();
    pick_disk(&list, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_mount_point_that_holds_the_path_wins() {
        let disks = vec![(PathBuf::from("/"), 10), (PathBuf::from("/Volumes/Photos"), 20), (PathBuf::from("/Volumes/Photo"), 30)];
        assert_eq!(pick_disk(&disks, Path::new("/Volumes/Photos/Lumen Library")), Some(20));
        assert_eq!(pick_disk(&disks, Path::new("/Users/sample/Pictures")), Some(10));
        // A path component must match whole: /Volumes/Photo is not a prefix of /Volumes/Photos.
        assert_eq!(pick_disk(&[(PathBuf::from("/Volumes/Photo"), 30)], Path::new("/Volumes/Photos/x")), None);
        // Windows drive letters.
        let win = vec![(PathBuf::from("C:\\"), 5), (PathBuf::from("X:\\"), 7)];
        assert_eq!(pick_disk(&win, Path::new("X:\\Photos\\Lumen Library")), Some(7));
    }
}
