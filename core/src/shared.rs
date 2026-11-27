//! Una lista condivisa fra il core e i frontend: clonarla copia un puntatore, non gli elementi.
//!
//! Le liste della libreria (playlist, brani piaciuti, ascolti) stanno nella vista pubblicata, in
//! `config.json` e nelle copie dei frontend: con `Shared` sono lo stesso `Arc`. Il core la cambia
//! con [`Shared::make_mut`], che copia gli elementi solo se qualcuno tiene ancora la versione
//! pubblicata; un frontend la legge come uno slice e la confronta con `Arc::ptr_eq`.

use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Un `Vec<T>` immutabile e condiviso. Scrive e legge come un array JSON.
pub struct Shared<T>(Arc<Vec<T>>);

impl<T> Shared<T> {
    /// Le due liste sono la stessa allocazione: nessuno l'ha cambiata fra una lettura e l'altra.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone> Shared<T> {
    /// La lista da modificare: se è condivisa con una vista già pubblicata la copia prima.
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
    /// Lo stesso puntatore basta: il confronto degli elementi serve solo a due liste diverse.
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
