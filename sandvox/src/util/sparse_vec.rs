use std::{
    marker::PhantomData,
    ops::{
        Index,
        IndexMut,
    },
};

use serde::{
    Serialize,
    ser::SerializeMap,
};

#[derive(Debug)]
pub struct SparseVec<I, T> {
    entries: Vec<Option<T>>,
    free_list: Vec<usize>,
    num_entries: usize,
    _marker: PhantomData<fn(I)>,
}

impl<I, T> Default for SparseVec<I, T> {
    fn default() -> Self {
        Self {
            entries: vec![],
            free_list: vec![],
            num_entries: 0,
            _marker: PhantomData,
        }
    }
}

impl<I, T> SparseVec<I, T> {
    pub fn len(&self) -> usize {
        self.num_entries
    }
}

impl<I, T> SparseVec<I, T>
where
    I: From<usize>,
{
    pub fn push(&mut self, value: T) -> I {
        let index = if let Some(index) = self.free_list.pop() {
            assert!(self.entries[index].is_none());
            self.entries[index] = Some(value);
            index
        }
        else {
            let index = self.entries.len();
            self.entries.push(Some(value));
            index
        };

        self.num_entries += 1;
        I::from(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, allocation)| Some((I::from(index), allocation.as_ref()?)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (I, &mut T)> {
        self.entries
            .iter_mut()
            .enumerate()
            .filter_map(|(index, allocation)| Some((I::from(index), allocation.as_mut()?)))
    }
}

impl<I, T> SparseVec<I, T>
where
    usize: From<I>,
{
    pub fn remove(&mut self, index: I) -> Option<T> {
        let index = usize::from(index);
        let value = self
            .entries
            .get_mut(index)
            .and_then(|allocation| allocation.take());
        if value.is_some() {
            self.num_entries -= 1;
            self.free_list.push(index);
        }
        value
    }
}

impl<I, T> Index<I> for SparseVec<I, T>
where
    usize: From<I>,
{
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        let index = usize::from(index);
        self.entries[index].as_ref().unwrap()
    }
}

impl<I, T> IndexMut<I> for SparseVec<I, T>
where
    usize: From<I>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        let index = usize::from(index);
        self.entries[index].as_mut().unwrap()
    }
}

impl<I, T> Clone for SparseVec<I, T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            free_list: self.free_list.clone(),
            num_entries: self.num_entries,
            _marker: PhantomData,
        }
    }
}

impl<I, T> Serialize for SparseVec<I, T>
where
    I: From<usize> + Serialize,
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;

        for (key, value) in self.iter() {
            map.serialize_entry(&key, value)?;
        }

        map.end()
    }
}
