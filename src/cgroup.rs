use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    error::{Error, Result},
    io::CgroupIo,
    memory::CgroupMemory,
};

/// A cgroup v2 directory, e.g. `/sys/fs/cgroup/user.slice`.
pub struct Cgroup {
    pub(crate) memory: CgroupMemory,
    pub(crate) io: CgroupIo,
}

impl Cgroup {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Self {
            memory: CgroupMemory::open(path)?,
            io: CgroupIo::open(path)?,
        })
    }
}

/// Finds the leaf cgroups (no child cgroups) under a root, and what changed since the
/// previous scan.
pub(crate) struct CgroupTree {
    root: PathBuf,
    leaves: BTreeSet<PathBuf>,
}

/// A leaf that appeared or went away. A cgroup that gets children is no longer a leaf, so
/// it's `Removed` and its children are `Added`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Leaf {
    Added(PathBuf),
    Removed(PathBuf),
}

impl CgroupTree {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            leaves: BTreeSet::new(),
        }
    }

    pub(crate) fn rescan(&mut self) -> Result<Vec<Leaf>> {
        // Leaves may vanish mid-scan, but a missing root is an error.
        fs::metadata(&self.root).map_err(|source| Error::Io {
            path: self.root.clone(),
            source,
        })?;
        let mut leaves = BTreeSet::new();
        Self::collect_leaves(&self.root, &mut leaves)?;
        let removed = self.leaves.difference(&leaves).cloned().map(Leaf::Removed);
        let added = leaves.difference(&self.leaves).cloned().map(Leaf::Added);
        let changes = removed.chain(added).collect();
        self.leaves = leaves;
        Ok(changes)
    }

    /// Makes the next rescan report `path` as added again if it's still a leaf.
    pub(crate) fn forget(&mut self, path: &Path) {
        self.leaves.remove(path);
    }

    /// Adds `dir` to `leaves` if it has no child cgroups, otherwise recurses into them.
    fn collect_leaves(dir: &Path, leaves: &mut BTreeSet<PathBuf>) -> Result<()> {
        let io_err = |source| Error::Io {
            path: dir.to_owned(),
            source,
        };
        let entries = match fs::read_dir(dir) {
            // Removed while scanning: it'll be gone from the next scan too.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result.map_err(io_err)?,
        };
        let mut has_children = false;
        for entry in entries {
            let entry = entry.map_err(io_err)?;
            if entry.file_type().map_err(io_err)?.is_dir() {
                has_children = true;
                Self::collect_leaves(&entry.path(), leaves)?;
            }
        }
        if !has_children {
            leaves.insert(dir.to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_leaf_changes() {
        let root = std::env::temp_dir().join(format!("ebb-tree-{}", std::process::id()));
        let mkdir = |dir: &str| fs::create_dir_all(root.join(dir)).unwrap();
        let added = |dir: &str| Leaf::Added(root.join(dir));
        let removed = |dir: &str| Leaf::Removed(root.join(dir));
        mkdir("a/x");
        mkdir("a/y");
        mkdir("b");
        // Files don't count as children.
        fs::write(root.join("b/memory.current"), "0").unwrap();

        let mut tree = CgroupTree::new(&root);
        assert_eq!(
            tree.rescan().unwrap(),
            [added("a/x"), added("a/y"), added("b")]
        );
        assert_eq!(tree.rescan().unwrap(), []);

        // b gets a child: b is no longer a leaf, b/c is. a/y goes away.
        mkdir("b/c");
        fs::remove_dir(root.join("a/y")).unwrap();
        assert_eq!(
            tree.rescan().unwrap(),
            [removed("a/y"), removed("b"), added("b/c")]
        );

        assert!(CgroupTree::new(root.join("missing")).rescan().is_err());

        // Forgotten leaves come back on the next scan.
        tree.forget(&root.join("a/x"));
        assert_eq!(tree.rescan().unwrap(), [added("a/x")]);

        fs::remove_dir_all(&root).unwrap();
    }
}
