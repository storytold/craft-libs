//! The group hierarchy: listing it depth first and resolving `/a/b/c` paths.

use super::error::Hdf5Error;
use super::group::Group;
use super::object::Object;
use super::sizes::Address;
use super::storage::Storage;
use super::{Entry, EntryKind};
use std::collections::HashSet;

/// Deeper nesting than this is treated as corrupt (the reference app nests 3–4 levels).
const MAX_DEPTH: usize = 64;

/// Every group and dataset below the root group, depth first, members in name order.
///
/// A group reachable twice (hard links, loops) is listed each time but only descended into once.
pub(crate) fn list(storage: &Storage, root: Address) -> Result<Vec<Entry>, Hdf5Error> {
    let mut lister = Lister { storage, entries: Vec::new(), expanded: HashSet::from([root]) };
    lister.group(&root_group(storage, root)?, "", 0)?;
    Ok(lister.entries)
}

/// The object at `path` (absolute or relative to the root; empty components are ignored).
pub(crate) fn resolve(storage: &Storage, root: Address, path: &str) -> Result<Object, Hdf5Error> {
    let mut object = Object::load(storage, root)?;
    for name in path.split('/').filter(|c| !c.is_empty()) {
        let Object::Group(group) = &object else {
            return Err(Hdf5Error::NotFound(path.to_string()));
        };
        let member = group.member(storage, name)?.ok_or_else(|| Hdf5Error::NotFound(path.to_string()))?;
        object = Object::load(storage, member.header)?;
    }
    Ok(object)
}

fn root_group(storage: &Storage, root: Address) -> Result<Group, Hdf5Error> {
    match Object::load(storage, root)? {
        Object::Group(group) => Ok(group),
        Object::Dataset(_) => Err(Hdf5Error::Corrupt("the root object is not a group".into())),
    }
}

struct Lister<'s> {
    storage: &'s Storage,
    entries: Vec<Entry>,
    /// Object header addresses of groups already descended into.
    expanded: HashSet<Address>,
}

impl Lister<'_> {
    fn group(&mut self, group: &Group, prefix: &str, depth: usize) -> Result<(), Hdf5Error> {
        if depth > MAX_DEPTH {
            return Err(Hdf5Error::Corrupt(format!("groups nested deeper than {MAX_DEPTH} levels")));
        }
        for member in group.members(self.storage)? {
            let path = format!("{prefix}/{}", member.name);
            match Object::load(self.storage, member.header)? {
                Object::Dataset(dataset) => {
                    let len = dataset.len()?;
                    self.entries.push(Entry { path, kind: EntryKind::Dataset { len } });
                }
                Object::Group(child) => {
                    self.entries.push(Entry { path: path.clone(), kind: EntryKind::Group });
                    if self.expanded.insert(member.header) {
                        self.group(&child, &path, depth + 1)?;
                    }
                }
            }
        }
        Ok(())
    }
}
