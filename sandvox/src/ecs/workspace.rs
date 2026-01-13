use std::{
    any::type_name,
    ops::{
        Deref,
        DerefMut,
    },
    sync::Arc,
};

use parking_lot::Mutex;

#[derive(Debug)]
pub struct Workspaces<T> {
    inner: Arc<Mutex<Vec<T>>>,
}

impl<T> Clone for Workspaces<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Default for Workspaces<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl<T> Workspaces<T>
where
    T: Send + Sync + Default,
{
    pub fn get(&self) -> WorkspaceGuard<T> {
        let mut inner = self.inner.lock();
        let inner = inner.pop().unwrap_or_else(|| {
            tracing::debug!("allocating workspace: {}", type_name::<T>());
            T::default()
        });

        WorkspaceGuard {
            inner: Some(inner),
            pool: self.clone(),
        }
    }
}

#[derive(Debug)]
pub struct WorkspaceGuard<T> {
    pool: Workspaces<T>,
    inner: Option<T>,
}

impl<T> Drop for WorkspaceGuard<T> {
    fn drop(&mut self) {
        let mut pool = self.pool.inner.lock();
        pool.extend(self.inner.take());
    }
}

impl<T> Deref for WorkspaceGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<T> DerefMut for WorkspaceGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}
