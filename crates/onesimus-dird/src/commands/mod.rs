//! Command implementations, grouped by category.

pub mod basic;
pub mod status;
pub mod dot;
pub mod bvfs;
pub mod ovfs;
pub mod job;
pub mod cancel;
pub mod list;
pub mod admin;
pub mod estimate;
pub mod restore;
pub mod restore_bsr;
pub mod restore_tree;
pub mod configure;
pub mod sql;
pub mod acl;
pub mod catalog;
pub mod extra;
pub mod purge;
pub mod show;
pub mod show_items;

pub mod debug;
pub mod bandwidth;
pub mod resolve;
pub mod wait;

pub mod defaults;

pub mod configure_usage;
pub mod prompt;
