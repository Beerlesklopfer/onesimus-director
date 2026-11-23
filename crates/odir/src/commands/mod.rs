// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
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
pub mod label;
pub mod estimate;
pub mod restore;
pub mod restore_bsr;
pub mod restore_tree;
pub mod configure;
pub mod config_crud;
pub mod sql;
pub mod acl;
pub mod grammar;
pub mod catalog;
pub mod extra;
pub mod purge;
pub mod purge_bulk;
pub mod volume_damaged;
pub mod autoprune;
pub mod show;
pub mod show_items;

pub mod debug;
pub mod bandwidth;
pub mod resolve;
pub mod wait;

pub mod defaults;
pub mod pool_db;
pub mod update_db;

pub mod configure_usage;
pub mod prompt;
