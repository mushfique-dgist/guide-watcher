use sha2::{Digest, Sha256};
use std::io::Read;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

const MAX_TREE_FILES: usize = 512;
const MAX_TREE_BYTES: u64 = 768 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 8;

pub(crate) fn digest_plain_tree(root: &Path, label: &str) -> Result<String, String> {
    ensure_plain(root, true, label)?;
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut directories = Vec::<String>::new();
    let mut files = Vec::<(String, PathBuf, u64)>::new();
    let mut total_bytes = 0u64;

    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(format!("{label} exceeds the maximum directory depth"));
        }
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("could not inspect {label}: {error}"))?
        {
            let entry = entry.map_err(|error| format!("could not inspect {label}: {error}"))?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("could not inspect {label} entry: {error}"))?;
            if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
                return Err(format!("{label} contains a link or reparse point"));
            }
            if metadata.is_dir() {
                let relative = relative_name(root, &path, label)?;
                directories.push(relative);
                if directories.len() + files.len() > MAX_TREE_FILES {
                    return Err(format!("{label} contains too many entries"));
                }
                pending.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                return Err(format!("{label} contains a non-file entry"));
            }
            let relative = relative_name(root, &path, label)?;
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .filter(|total| *total <= MAX_TREE_BYTES)
                .ok_or_else(|| format!("{label} exceeds the cumulative byte limit"))?;
            files.push((relative, path, metadata.len()));
            if directories.len() + files.len() > MAX_TREE_FILES {
                return Err(format!("{label} contains too many entries"));
            }
        }
    }

    directories.sort();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    digest.update(b"guide-watcher-circuit-context-tree-v1\0");
    for relative in directories {
        let relative = relative.as_bytes();
        digest.update(b"D");
        digest.update((relative.len() as u64).to_be_bytes());
        digest.update(relative);
    }
    let mut buffer = [0u8; 64 * 1024];
    for (relative, path, expected_len) in files {
        let relative = relative.as_bytes();
        digest.update(b"F");
        digest.update((relative.len() as u64).to_be_bytes());
        digest.update(relative);
        digest.update(expected_len.to_be_bytes());
        let mut file = std::fs::File::open(&path)
            .map_err(|error| format!("could not open {label} entry: {error}"))?;
        let opened = file
            .metadata()
            .map_err(|error| format!("could not inspect opened {label} entry: {error}"))?;
        if !opened.is_file() || is_reparse_point(&opened) || opened.len() != expected_len {
            return Err(format!("{label} changed while it was being captured"));
        }
        let mut read_len = 0u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("could not read {label} entry: {error}"))?;
            if count == 0 {
                break;
            }
            read_len = read_len
                .checked_add(count as u64)
                .ok_or_else(|| format!("{label} byte count overflowed"))?;
            digest.update(&buffer[..count]);
        }
        if read_len != expected_len {
            return Err(format!("{label} changed while it was being read"));
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn relative_name(root: &Path, path: &Path, label: &str) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("{label} entry escaped its root"))?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{label} contains an unsafe relative path"));
    }
    Ok(relative
        .to_str()
        .ok_or_else(|| format!("{label} contains a non-Unicode filename"))?
        .replace('\\', "/"))
}

fn ensure_plain(path: &Path, directory: bool, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {label}: {error}"))?;
    if metadata.file_type().is_symlink()
        || is_reparse_point(&metadata)
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(format!("{label} is not a plain local path"));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::digest_plain_tree;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn scratch_dir() -> PathBuf {
        let root = std::env::temp_dir().join(format!("guide-watcher-tree-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn digest_binds_paths_and_bytes_independent_of_creation_order() {
        let first = scratch_dir();
        let second = scratch_dir();
        std::fs::create_dir(first.join("nested")).unwrap();
        std::fs::write(first.join("nested/b.txt"), b"two").unwrap();
        std::fs::write(first.join("a.txt"), b"one").unwrap();
        std::fs::write(second.join("a.txt"), b"one").unwrap();
        std::fs::create_dir(second.join("nested")).unwrap();
        std::fs::write(second.join("nested/b.txt"), b"two").unwrap();

        assert_eq!(
            digest_plain_tree(&first, "first").unwrap(),
            digest_plain_tree(&second, "second").unwrap()
        );
        std::fs::write(second.join("nested/b.txt"), b"changed").unwrap();
        assert_ne!(
            digest_plain_tree(&first, "first").unwrap(),
            digest_plain_tree(&second, "second").unwrap()
        );
        std::fs::write(second.join("nested/b.txt"), b"two").unwrap();
        let before_empty_directory = digest_plain_tree(&second, "second").unwrap();
        std::fs::create_dir(second.join("unexpected-empty")).unwrap();
        assert_ne!(
            before_empty_directory,
            digest_plain_tree(&second, "second").unwrap()
        );
        std::fs::remove_dir_all(first).unwrap();
        std::fs::remove_dir_all(second).unwrap();
    }
}
