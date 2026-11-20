//! JSON-basiertes Schema-ORM für den Bareos Catalog.
//!
//! Die JSON-Dateien in `/var/lib/bareos/catalog/` sind die Single Source of Truth
//! für das Datenbank-Schema. Jede JSON-Datei enthält eine Versionsnummer.
//!
//! # Architektur
//!
//! ```text
//! /var/lib/bareos/catalog/
//!   schema/
//!     v001_initial.json          ← Basis-Schema
//!     v002_add_encryption.json   ← Migration
//!     v003_add_dedup.json        ← Migration
//!   current_version.json         ← Aktuelle Schema-Version
//! ```
//!
//! Jede Schema-Datei beschreibt Tabellen, Spalten, Typen, Indizes, Constraints.
//! Das ORM:
//! 1. Liest die JSON-Schema-Dateien
//! 2. Generiert DDL-SQL daraus
//! 3. Führt Migrationen durch (Version N → N+1)
//! 4. Validiert das Live-Schema gegen die JSON-Definition
//! 5. Generiert Rust-Structs (optional, zur Compile-Zeit)
//!
//! # Schemas
//!
//! The Bareos tables (v2250) live in the default schema of the connection
//! (`public`). Every Onesimus object (views, functions, tables of v2251+)
//! is schema-qualified in the JSON files (`onesimus.vw_job_summary`) and
//! lives in its own PostgreSQL schema `onesimus`; v2253 moves the objects
//! older versions created in `public` there ([`schema::RelocateDef`]). The
//! version bookkeeping `bareos_schema_version` deliberately stays in the
//! default schema, where every odir release looks for it.
//!
//! # Bareos upgrades and self-healing
//!
//! No Onesimus object depends on a Bareos table: functions have string
//! bodies (resolved when called), and since v2254 every view reads a
//! set-returning SQL function (`onesimus.vw_job_summary` = `SELECT … FROM
//! onesimus.fn_vw_job_summary()`) instead of the tables, so the Bareos
//! update scripts may alter or drop columns and tables. [`Migrator::heal`]
//! (module [`heal`]) runs after the migration at every odir start and
//! re-applies the shipped definition of any Onesimus object that is missing
//! or broken.

pub mod schema;
pub mod migration;
pub mod codegen;
pub mod error;
pub mod heal;

pub use schema::{CatalogSchema, TableDef, ColumnDef, IndexDef, ObjectKind, RelocateDef};
pub use migration::Migrator;
pub use error::OrmError;
pub use heal::HealReport;
