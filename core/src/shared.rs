//! A list shared between the core and the frontends: cloning it copies a pointer, not the elements.
//!
//! The library lists (playlists, liked tracks, plays) sit in the published view, in
//! `config.json` and in the frontends' copies: with `Shared` they are the same `Arc`. The core changes it
//! with [`Shared::make_mut`], which copies the elements only if someone still holds the published
//! version; a frontend reads it as a slice and compares it with `Arc::ptr_eq`.

use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An immutable, shared `Vec<T>`. It reads and writes like a JSON array.
pub struct Shared<T>(Arc<Vec<T>>);

impl<T> Shared<T> {
    /// The two lists are the same allocation: nobody changed it between one read and the next.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone> Shared<T> {
    /// The list to modify: if it is shared with an already published view, it is copied first.
    pub fn make_mut(&mut self) -> &mut Vec<T> {
        Arc::make_mut(&mut self.0)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T> Deref for Shared<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T> From<Vec<T>> for Shared<T> {
    fn from(items: Vec<T>) -> Self {
        Self(Arc::new(items))
    }
}

impl<T> FromIterator<T> for Shared<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        Self(Arc::new(items.into_iter().collect()))
    }
}

impl<'a, T> IntoIterator for &'a Shared<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    /// The same pointer is enough: comparing the elements is only for two different lists.
    fn eq(&self, other: &Self) -> bool {
        self.same(other) || *self.0 == *other.0
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_shares_the_list_until_one_side_changes() {
        let mut list = Shared::from(vec![1, 2, 3]);
        let published = list.clone();
        assert!(list.same(&published));
        list.make_mut().push(4);
        assert!(!list.same(&published));
        assert_eq!((published.len(), list.len()), (3, 4));
    }

    #[test]
    fn a_unique_list_changes_in_place() {
        let mut list = Shared::from(vec![1]);
        let before = Arc::as_ptr(&list.0);
        list.make_mut().push(2);
        assert_eq!(before, Arc::as_ptr(&list.0), "nobody else holds it: no copy");
    }

    #[test]
    fn it_reads_and_writes_a_plain_json_array() {
        let list: Shared<u32> = serde_json::from_str("[1,2]").unwrap();
        assert_eq!(serde_json::to_string(&list).unwrap(), "[1,2]");
        assert_eq!(list, Shared::from(vec![1, 2]));
    }
}
