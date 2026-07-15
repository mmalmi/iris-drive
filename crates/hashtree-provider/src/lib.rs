//! Backend-neutral filesystem provider abstraction over hashtree trees.
//!
//! `hashtree-provider` exposes the directory-tree semantics used by FUSE,
//! WinFsp, NSFileProvider, and Android SAF adapters. The trait
//! [`ProviderFs`] is the source of truth; each OS adapter consumes it.
//!
//! It also exposes [`path_diff`], a path-level diff between two hashtree
//! roots that emits `Added` / `Modified` / `Removed` events suitable for
//! NSFileProvider's `changes-since-anchor` enumeration.

pub mod diff;
pub mod error;
pub mod hashtree_impl;
pub mod provider;

pub use diff::{EntryInfo, PathChange, path_diff};
pub use error::ProviderError;
pub use hashtree_impl::{HashTreeProviderFs, RootObserver};
pub use provider::{DirItem, Item, ItemKind, ProviderFs, SyncAnchor};
