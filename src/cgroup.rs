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

/// Finds the leaf cgroups (no child cgroups) under a root that pass `filter`, and what
/// changed since the previous scan.
pub(crate) struct CgroupTree<F> {
    root: PathBuf,
    filter: F,
    leaves: BTreeSet<PathBuf>,
}

/// A leaf that appeared or went away. A cgroup that gets children is no longer a leaf, so
/// it's `Removed` and its children are `Added`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Leaf {
    Added(PathBuf),
    Removed(PathBuf),
}

impl<F: Fn(&Path) -> bool> CgroupTree<F> {
    pub(crate) fn new(root: impl Into<PathBuf>, filter: F) -> Self {
        Self {
            root: root.into(),
            filter,
            leaves: BTreeSet::new(),
        }
    }

    pub(crate) fn rescan(&mut self) -> Result<Vec<Leaf>> {
        // Leaves may vanish mid-scan, but a missing root is an error.
        fs::metadata(&self.root).map_err(Error::io(&self.root))?;
        let mut leaves = BTreeSet::new();
        self.collect_leaves(&self.root, &mut leaves)?;
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

    /// Adds `dir` to `leaves` if it has no child cgroups and passes the filter, otherwise
    /// recurses into its children.
    fn collect_leaves(&self, dir: &Path, leaves: &mut BTreeSet<PathBuf>) -> Result<()> {
        let entries = match fs::read_dir(dir) {
            // Removed while scanning: it'll be gone from the next scan too.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result.map_err(Error::io(dir))?,
        };
        let mut has_children = false;
        for entry in entries {
            let entry = entry.map_err(Error::io(dir))?;
            if entry.file_type().map_err(Error::io(dir))?.is_dir() {
                has_children = true;
                self.collect_leaves(&entry.path(), leaves)?;
            }
        }
        if !has_children && (self.filter)(dir) {
            leaves.insert(dir.to_owned());
        }
        Ok(())
    }
}

/// Leaf filters, composed with `&&` into the one `CgroupTree` takes.
pub(crate) mod filter {
    use std::{fs, path::Path};

    /// Leaves with processes in them (`populated 1` in `cgroup.events`).
    pub(crate) fn populated(path: &Path) -> bool {
        fs::read_to_string(path.join("cgroup.events"))
            .is_ok_and(|events| events.lines().any(|line| line == "populated 1"))
    }

    /// Leaves whose path under `root` matches one of `globs`, oomd style: `*` and `?` match
    /// within a single path component, e.g. `*/*/cri-containerd-*.scope`. No globs match
    /// every leaf.
    pub(crate) fn glob(root: &Path, globs: &[String], path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        globs.is_empty()
            || globs.iter().any(|glob| {
                let mut components = relative.iter();
                glob.split('/').filter(|part| !part.is_empty()).all(|part| {
                    components
                        .next()
                        .and_then(|component| component.to_str())
                        .is_some_and(|component| matches(part.as_bytes(), component.as_bytes()))
                }) && components.next().is_none()
            })
    }

    fn matches(glob: &[u8], name: &[u8]) -> bool {
        match (glob.split_first(), name.split_first()) {
            (None, None) => true,
            (Some((b'*', rest)), _) => {
                matches(rest, name) || (!name.is_empty() && matches(glob, &name[1..]))
            }
            (Some((b'?', rest)), Some((_, name))) => matches(rest, name),
            (Some((g, rest)), Some((n, name))) if g == n => matches(rest, name),
            _ => false,
        }
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

        let mut tree = CgroupTree::new(&root, |_: &Path| true);
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

        assert!(CgroupTree::new(root.join("missing"), |_: &Path| true)
            .rescan()
            .is_err());

        // Forgotten leaves come back on the next scan.
        tree.forget(&root.join("a/x"));
        assert_eq!(tree.rescan().unwrap(), [added("a/x")]);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn glob_matches_per_component() {
        let root = PathBuf::from("/sys/fs/cgroup/kubepods.slice");
        let globs = ["*/*/cri-containerd-*.scope".into(), "*/cri-?.scope".into()];
        let matches = |path: &str| filter::glob(&root, &globs, &root.join(path));
        assert!(matches(
            "burstable.slice/pod1.slice/cri-containerd-ab12.scope"
        ));
        assert!(matches("pod2.slice/cri-x.scope"));
        // `*` doesn't cross a `/`, and the number of components must match.
        assert!(!matches("pod2.slice/cri-containerd-ab12.scope"));
        assert!(!matches("a/b/c/cri-containerd-ab12.scope"));
        assert!(!matches("burstable.slice/pod1.slice/crio-ab12.scope"));
        assert!(!filter::glob(
            &root,
            &globs,
            Path::new("/elsewhere/pod2.slice/cri-x.scope")
        ));
        assert!(filter::glob(&root, &[], &root.join("a/b/c.scope")));
    }

    #[test]
    fn populated_reads_cgroup_events() {
        let dir = std::env::temp_dir().join(format!("ebb-populated-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("cgroup.events"), "populated 1\nfrozen 0\n").unwrap();
        assert!(filter::populated(&dir));
        fs::write(dir.join("cgroup.events"), "populated 0\nfrozen 0\n").unwrap();
        assert!(!filter::populated(&dir));
        fs::remove_dir_all(&dir).unwrap();
        assert!(!filter::populated(&dir));
    }
}
