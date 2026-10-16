//! Schema-Definition aus JSON-Dateien.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::OrmError;

/// Vollständiges Catalog-Schema mit Versionierung.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSchema {
    /// Schema-Version (monoton steigend)
    pub version: u32,
    /// Beschreibung dieser Version
    pub description: String,
    /// Datum der Erstellung (ISO 8601)
    pub created: String,
    /// Autor
    pub author: String,
    /// Minimale kompatible Version (für Downgrades)
    pub min_compatible_version: Option<u32>,
    /// Tabellen-Definitionen
    pub tables: Vec<TableDef>,
    /// Views
    #[serde(default)]
    pub views: Vec<ViewDef>,
    /// Stored Procedures / Functions
    #[serde(default)]
    pub functions: Vec<FunctionDef>,
}

/// Tabellen-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDef {
    /// Tabellenname (z.B. "job", "media", "client")
    pub name: String,
    /// Beschreibung
    #[serde(default)]
    pub description: String,
    /// Spalten
    pub columns: Vec<ColumnDef>,
    /// Primärschlüssel: Spaltenname, bei zusammengesetzten Schlüsseln durch
    /// Komma getrennt (`"jobid, pathid"`); leer = kein Primärschlüssel (wie
    /// `version`, `jobhisto`, `devicestats` in der Bareos-DDL).
    #[serde(default)]
    pub primary_key: String,
    /// Indizes
    #[serde(default)]
    pub indices: Vec<IndexDef>,
    /// Foreign Keys
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeyDef>,
    /// Zusätzliche Constraints
    #[serde(default)]
    pub constraints: Vec<ConstraintDef>,
    /// Zeilen, die beim Anlegen eingefügt werden (Werte in Spaltenreihenfolge),
    /// z.B. die `status`-Zeilen und die `version`-Zeile der Bareos-DDL.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_rows: Vec<Vec<serde_json::Value>>,
}

/// Spalten-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    /// Spaltenname
    pub name: String,
    /// SQL-Typ (z.B. "serial", "text", "bigint", "timestamp", "jsonb", "boolean")
    #[serde(rename = "type")]
    pub col_type: String,
    /// NOT NULL?
    #[serde(default)]
    pub not_null: bool,
    /// Default-Wert (SQL-Ausdruck)
    #[serde(default)]
    pub default: Option<String>,
    /// Beschreibung
    #[serde(default)]
    pub description: String,
    /// JSONB: Schema-Validierung für JSON-Spalten
    #[serde(default)]
    pub json_schema: Option<serde_json::Value>,
    /// `ALTER TABLE ... ALTER COLUMN ... SET STATISTICS n`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statistics: Option<i32>,
}

/// Index-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// Index-Name
    pub name: String,
    /// Spalten
    pub columns: Vec<String>,
    /// UNIQUE?
    #[serde(default)]
    pub unique: bool,
    /// Index-Typ (btree, hash, gin, gist)
    #[serde(default = "default_index_type")]
    pub index_type: String,
    /// WHERE-Klausel (partial index)
    #[serde(default)]
    pub where_clause: Option<String>,
}

fn default_index_type() -> String {
    "btree".to_string()
}

/// Foreign-Key-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDef {
    /// Spalte in dieser Tabelle
    pub column: String,
    /// Referenzierte Tabelle
    pub references_table: String,
    /// Referenzierte Spalte
    pub references_column: String,
    /// ON DELETE Aktion
    #[serde(default = "default_fk_action")]
    pub on_delete: String,
}

fn default_fk_action() -> String {
    "CASCADE".to_string()
}

/// Constraint-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintDef {
    /// Constraint-Name
    pub name: String,
    /// Typ: "check", "unique", "exclude"
    #[serde(rename = "type")]
    pub constraint_type: String,
    /// SQL-Ausdruck
    pub expression: String,
}

/// View-Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewDef {
    /// View-Name (z.B. "job_summary")
    pub name: String,
    /// Beschreibung
    #[serde(default)]
    pub description: String,
    /// SELECT-Query (ohne "CREATE VIEW ... AS")
    pub query: String,
    /// Materialized View? (Default: false)
    #[serde(default)]
    pub materialized: bool,
}

/// Stored Procedure / Function Definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    /// Funktionsname (z.B. "create_job")
    pub name: String,
    /// Beschreibung
    #[serde(default)]
    pub description: String,
    /// Parameter als SQL-Fragment (z.B. "p_name text, p_type char(1)")
    #[serde(default)]
    pub params: String,
    /// Rückgabetyp (z.B. "integer", "void", "TABLE(id int, name text)")
    #[serde(default = "default_returns")]
    pub returns: String,
    /// Sprache (default: "plpgsql")
    #[serde(default = "default_language")]
    pub language: String,
    /// Funktions-Body (PL/pgSQL oder SQL)
    pub body: String,
    /// SECURITY DEFINER? (Default: false)
    #[serde(default)]
    pub security_definer: bool,
    /// Volatility: VOLATILE, STABLE, IMMUTABLE (Default: VOLATILE)
    #[serde(default = "default_volatility")]
    pub volatility: String,
    /// PARALLEL: SAFE, RESTRICTED, UNSAFE (Default: nicht angegeben = UNSAFE)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<String>,
}

fn default_returns() -> String { "void".to_string() }
fn default_language() -> String { "plpgsql".to_string() }
fn default_volatility() -> String { "VOLATILE".to_string() }

impl CatalogSchema {
    /// Lade Schema aus einer JSON-Datei.
    pub fn load(path: &Path) -> Result<Self, OrmError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| OrmError::Other(format!("read {}: {e}", path.display())))?;
        let schema: Self = serde_json::from_str(&content)?;
        Ok(schema)
    }

    /// Speichere Schema als JSON-Datei.
    pub fn save(&self, path: &Path) -> Result<(), OrmError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
            .map_err(|e| OrmError::Other(format!("write {}: {e}", path.display())))?;
        Ok(())
    }

    /// Generiere CREATE TABLE SQL für alle Tabellen.
    pub fn to_ddl(&self) -> String {
        let mut sql = String::new();
        sql.push_str(&format!("-- Bareos Catalog Schema v{}\n", self.version));
        sql.push_str(&format!("-- {}\n", self.description));
        sql.push_str(&format!("-- Generated from JSON schema\n\n"));

        for table in &self.tables {
            for stmt in table.to_statements() {
                sql.push_str(&stmt);
                sql.push('\n');
            }
            sql.push('\n');
        }

        if !self.functions.is_empty() {
            sql.push_str("-- Functions / Stored Procedures\n\n");
            for func in &self.functions {
                sql.push_str(&func.to_sql());
                sql.push_str("\n\n");
            }
        }

        if !self.views.is_empty() {
            sql.push_str("-- Views\n\n");
            for view in &self.views {
                sql.push_str(&view.to_sql());
                sql.push_str("\n\n");
            }
        }

        sql
    }

    /// Generiere einzelne SQL-Statements (Tables + Indices + Functions + Views).
    pub fn to_statements(&self) -> Vec<String> {
        let mut stmts = Vec::new();
        // 1. Tables + Indices + initial rows
        for table in &self.tables {
            stmts.extend(table.to_statements());
        }
        // 2. Functions (before views, views may reference functions)
        for func in &self.functions {
            stmts.push(func.to_sql());
        }
        // 3. Views (may reference tables and functions)
        for view in &self.views {
            stmts.push(view.to_sql());
        }
        stmts
    }

    /// Finde eine Tabelle nach Name.
    pub fn find_table(&self, name: &str) -> Option<&TableDef> {
        self.tables.iter().find(|t| t.name == name)
    }
}

impl TableDef {
    /// Primärschlüssel-Spalten (leer: kein Primärschlüssel).
    pub fn primary_key_columns(&self) -> Vec<&str> {
        self.primary_key.split(',').map(str::trim).filter(|c| !c.is_empty()).collect()
    }

    /// Alle Anweisungen zum Anlegen der Tabelle: CREATE TABLE, Spalten-
    /// Statistikziele, Indizes, Anfangszeilen.
    pub fn to_statements(&self) -> Vec<String> {
        let mut stmts = vec![self.to_create_sql()];
        for col in &self.columns {
            if let Some(target) = col.statistics {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET STATISTICS {target};",
                    self.name, col.name
                ));
            }
        }
        for idx in &self.indices {
            stmts.push(idx.to_sql(&self.name));
        }
        stmts.extend(self.insert_statements());
        stmts
    }

    /// INSERTs für `initial_rows`. Idempotent: eine Zeile wird nur eingefügt,
    /// wenn es noch keine mit demselben Primärschlüssel (ohne Primärschlüssel:
    /// mit denselben Werten) gibt.
    pub fn insert_statements(&self) -> Vec<String> {
        let pk = self.primary_key_columns();
        self.initial_rows.iter().map(|row| {
            let cols: Vec<&str> = self.columns.iter().take(row.len()).map(|c| c.name.as_str()).collect();
            let values: Vec<String> = row.iter().map(sql_literal).collect();
            let matches: Vec<String> = cols.iter().zip(&values)
                .filter(|(c, _)| pk.is_empty() || pk.contains(c))
                .map(|(c, v)| format!("{c} = {v}"))
                .collect();
            format!(
                "INSERT INTO {t} ({}) SELECT {} WHERE NOT EXISTS (SELECT 1 FROM {t} WHERE {});",
                cols.join(", "), values.join(", "), matches.join(" AND "), t = self.name
            )
        }).collect()
    }

    /// Generiere CREATE TABLE SQL.
    pub fn to_create_sql(&self) -> String {
        let mut items: Vec<String> = self.columns.iter().map(|col| {
            let mut item = format!("    {} {}", col.name, col.col_type);
            if col.not_null {
                item.push_str(" NOT NULL");
            }
            if let Some(ref default) = col.default {
                item.push_str(&format!(" DEFAULT {default}"));
            }
            item
        }).collect();

        let pk = self.primary_key_columns();
        if !pk.is_empty() {
            items.push(format!("    PRIMARY KEY ({})", pk.join(", ")));
        }
        for fk in &self.foreign_keys {
            items.push(format!(
                "    FOREIGN KEY ({}) REFERENCES {}({}) ON DELETE {}",
                fk.column, fk.references_table, fk.references_column, fk.on_delete
            ));
        }
        for c in &self.constraints {
            items.push(format!("    CONSTRAINT {} CHECK ({})", c.name, c.expression));
        }

        format!("CREATE TABLE IF NOT EXISTS {} (\n{}\n);\n", self.name, items.join(",\n"))
    }

    /// Generiere ALTER TABLE SQL für eine Spalten-Änderung.
    pub fn alter_add_column(table: &str, col: &ColumnDef) -> String {
        let mut sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, col.name, col.col_type);
        if col.not_null {
            if let Some(ref default) = col.default {
                sql.push_str(&format!(" NOT NULL DEFAULT {default}"));
            }
        }
        sql.push_str(";\n");
        sql
    }
}

/// SQL-Literal für einen JSON-Wert einer Anfangszeile.
fn sql_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        other => other.to_string(),
    }
}

impl IndexDef {
    /// Generiere CREATE INDEX SQL.
    pub fn to_sql(&self, table: &str) -> String {
        let unique = if self.unique { "UNIQUE " } else { "" };
        let using = if self.index_type != "btree" {
            format!(" USING {}", self.index_type)
        } else {
            String::new()
        };
        let cols = self.columns.join(", ");
        let where_clause = self.where_clause.as_ref()
            .map(|w| format!(" WHERE {w}"))
            .unwrap_or_default();

        format!(
            "CREATE {unique}INDEX IF NOT EXISTS {} ON {}{using} ({cols}){where_clause};",
            self.name, table
        )
    }
}

impl ViewDef {
    /// Generiere CREATE VIEW / CREATE MATERIALIZED VIEW SQL.
    pub fn to_sql(&self) -> String {
        if self.materialized {
            format!(
                "CREATE MATERIALIZED VIEW IF NOT EXISTS {} AS\n{};\n",
                self.name, self.query
            )
        } else {
            format!(
                "CREATE OR REPLACE VIEW {} AS\n{};\n",
                self.name, self.query
            )
        }
    }
}

impl FunctionDef {
    /// Generiere CREATE OR REPLACE FUNCTION SQL.
    pub fn to_sql(&self) -> String {
        let security = if self.security_definer { "\nSECURITY DEFINER" } else { "" };
        let parallel = self.parallel.as_deref().map(|p| format!(" PARALLEL {p}")).unwrap_or_default();
        format!(
            "CREATE OR REPLACE FUNCTION {}({}) RETURNS {} AS $$\n{}\n$$ LANGUAGE {}\n{}{}{};",
            self.name, self.params, self.returns,
            self.body,
            self.language, self.volatility, parallel, security,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schema() -> CatalogSchema {
        serde_json::from_str(SAMPLE_SCHEMA_JSON).unwrap()
    }

    #[test]
    fn parse_schema() {
        let schema = sample_schema();
        assert_eq!(schema.version, 1);
        assert_eq!(schema.tables.len(), 5);
        assert_eq!(schema.tables[0].name, "client");
    }

    #[test]
    fn generate_ddl() {
        let schema = sample_schema();
        let ddl = schema.to_ddl();
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS client"));
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS job"));
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS media"));
        assert!(ddl.contains("PRIMARY KEY"));
        assert!(ddl.contains("FOREIGN KEY"));
    }

    #[test]
    fn find_table() {
        let schema = sample_schema();
        assert!(schema.find_table("job").is_some());
        assert!(schema.find_table("nonexistent").is_none());
    }

    #[test]
    fn roundtrip_json() {
        let schema = sample_schema();
        let json = serde_json::to_string_pretty(&schema).unwrap();
        let parsed: CatalogSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, schema.version);
        assert_eq!(parsed.tables.len(), schema.tables.len());
    }

    #[test]
    fn index_sql() {
        let idx = IndexDef {
            name: "idx_job_name".to_string(),
            columns: vec!["name".to_string()],
            unique: false,
            index_type: "btree".to_string(),
            where_clause: None,
        };
        let sql = idx.to_sql("job");
        assert_eq!(sql, "CREATE INDEX IF NOT EXISTS idx_job_name ON job (name);");
    }

    #[test]
    fn gin_index_sql() {
        let idx = IndexDef {
            name: "idx_job_metadata".to_string(),
            columns: vec!["metadata".to_string()],
            unique: false,
            index_type: "gin".to_string(),
            where_clause: None,
        };
        let sql = idx.to_sql("job");
        assert!(sql.contains("USING gin"));
    }

    #[test]
    fn partial_index_sql() {
        let idx = IndexDef {
            name: "idx_job_running".to_string(),
            columns: vec!["jobstatus".to_string()],
            unique: false,
            index_type: "btree".to_string(),
            where_clause: Some("jobstatus = 'R'".to_string()),
        };
        let sql = idx.to_sql("job");
        assert!(sql.contains("WHERE jobstatus = 'R'"));
    }

    const SAMPLE_SCHEMA_JSON: &str = r#"{
  "version": 1,
  "description": "Bareos Catalog Initial Schema",
  "created": "2026-03-21",
  "author": "baReos",
  "tables": [
    {
      "name": "client",
      "description": "Registered backup clients (File Daemons)",
      "primary_key": "clientid",
      "columns": [
        {"name": "clientid",  "type": "serial",    "not_null": true, "description": "Unique client ID"},
        {"name": "name",      "type": "text",      "not_null": true, "description": "Client name"},
        {"name": "uname",     "type": "text",      "not_null": false, "description": "Client uname output"},
        {"name": "autoprune", "type": "boolean",   "not_null": true, "default": "true"},
        {"name": "fileretention", "type": "bigint", "not_null": true, "default": "5184000"},
        {"name": "jobretention",  "type": "bigint", "not_null": true, "default": "15552000"},
        {"name": "metadata",  "type": "jsonb",     "description": "Extensible client metadata"}
      ],
      "indices": [
        {"name": "idx_client_name", "columns": ["name"], "unique": true}
      ]
    },
    {
      "name": "pool",
      "description": "Volume pools",
      "primary_key": "poolid",
      "columns": [
        {"name": "poolid",    "type": "serial",  "not_null": true},
        {"name": "name",      "type": "text",    "not_null": true},
        {"name": "pooltype",  "type": "text",    "not_null": true, "default": "'Backup'"},
        {"name": "numvols",   "type": "integer", "not_null": true, "default": "0"},
        {"name": "maxvols",   "type": "integer", "not_null": true, "default": "0"},
        {"name": "labelformat", "type": "text"},
        {"name": "recycle",   "type": "boolean", "not_null": true, "default": "true"},
        {"name": "autoprune", "type": "boolean", "not_null": true, "default": "true"},
        {"name": "volretention", "type": "bigint", "not_null": true, "default": "31536000"},
        {"name": "maxvolbytes", "type": "bigint", "not_null": true, "default": "0"},
        {"name": "metadata",  "type": "jsonb"}
      ],
      "indices": [
        {"name": "idx_pool_name", "columns": ["name"], "unique": true}
      ]
    },
    {
      "name": "media",
      "description": "Backup volumes",
      "primary_key": "mediaid",
      "columns": [
        {"name": "mediaid",     "type": "serial",  "not_null": true},
        {"name": "volumename",  "type": "text",    "not_null": true},
        {"name": "poolid",      "type": "integer", "not_null": true},
        {"name": "mediatype",   "type": "text",    "not_null": true},
        {"name": "volstatus",   "type": "text",    "not_null": true, "default": "'Append'"},
        {"name": "volbytes",    "type": "bigint",  "not_null": true, "default": "0"},
        {"name": "volfiles",    "type": "integer", "not_null": true, "default": "0"},
        {"name": "maxvolbytes", "type": "bigint",  "not_null": true, "default": "0"},
        {"name": "recycle",     "type": "boolean", "not_null": true, "default": "true"},
        {"name": "firstwritten", "type": "timestamp"},
        {"name": "lastwritten",  "type": "timestamp"}
      ],
      "indices": [
        {"name": "idx_media_volumename", "columns": ["volumename"], "unique": true},
        {"name": "idx_media_poolid", "columns": ["poolid"]},
        {"name": "idx_media_volstatus", "columns": ["volstatus"]}
      ],
      "foreign_keys": [
        {"column": "poolid", "references_table": "pool", "references_column": "poolid"}
      ]
    },
    {
      "name": "job",
      "description": "Backup/restore/verify job history",
      "primary_key": "jobid",
      "columns": [
        {"name": "jobid",       "type": "serial",    "not_null": true},
        {"name": "job",         "type": "text",      "not_null": true, "description": "Unique job instance name"},
        {"name": "name",        "type": "text",      "not_null": true, "description": "Job resource name"},
        {"name": "type",        "type": "char(1)",   "not_null": true, "description": "B=Backup R=Restore V=Verify"},
        {"name": "level",       "type": "char(1)",   "not_null": true, "description": "F=Full I=Incr D=Diff"},
        {"name": "clientid",    "type": "integer",   "not_null": true, "default": "0"},
        {"name": "jobstatus",   "type": "char(1)",   "not_null": true, "description": "T=OK E=Error A=Cancel R=Running"},
        {"name": "schedtime",   "type": "timestamp"},
        {"name": "starttime",   "type": "timestamp"},
        {"name": "endtime",     "type": "timestamp"},
        {"name": "realendtime", "type": "timestamp"},
        {"name": "jobtdate",    "type": "bigint",    "not_null": true, "default": "0"},
        {"name": "volsessionid", "type": "integer",  "not_null": true, "default": "0"},
        {"name": "volsessiontime", "type": "integer", "not_null": true, "default": "0"},
        {"name": "jobfiles",    "type": "bigint",    "not_null": true, "default": "0"},
        {"name": "jobbytes",    "type": "bigint",    "not_null": true, "default": "0"},
        {"name": "joberrors",   "type": "integer",   "not_null": true, "default": "0"},
        {"name": "poolid",      "type": "integer",   "not_null": true, "default": "0"},
        {"name": "filesetid",   "type": "integer",   "not_null": true, "default": "0"},
        {"name": "priorjobid",  "type": "integer",   "not_null": true, "default": "0"},
        {"name": "comment",     "type": "text"},
        {"name": "metadata",    "type": "jsonb",     "description": "Extensible job metadata"}
      ],
      "indices": [
        {"name": "idx_job_name",     "columns": ["name"]},
        {"name": "idx_job_clientid", "columns": ["clientid"]},
        {"name": "idx_job_status",   "columns": ["jobstatus"]},
        {"name": "idx_job_starttime", "columns": ["starttime"]},
        {"name": "idx_job_metadata", "columns": ["metadata"], "index_type": "gin"}
      ],
      "foreign_keys": [
        {"column": "clientid", "references_table": "client", "references_column": "clientid"},
        {"column": "poolid", "references_table": "pool", "references_column": "poolid"}
      ]
    },
    {
      "name": "file",
      "description": "Backed up files catalog",
      "primary_key": "fileid",
      "columns": [
        {"name": "fileid",   "type": "bigserial", "not_null": true},
        {"name": "jobid",    "type": "integer",   "not_null": true},
        {"name": "pathid",   "type": "integer",   "not_null": true},
        {"name": "filename", "type": "text",      "not_null": true},
        {"name": "markid",   "type": "integer",   "not_null": true, "default": "0"},
        {"name": "lstat",    "type": "text",      "not_null": true},
        {"name": "md5",      "type": "text"},
        {"name": "deltaseq", "type": "smallint",  "not_null": true, "default": "0"}
      ],
      "indices": [
        {"name": "idx_file_jobid", "columns": ["jobid"]},
        {"name": "idx_file_pathid_filename", "columns": ["pathid", "filename"]}
      ],
      "foreign_keys": [
        {"column": "jobid", "references_table": "job", "references_column": "jobid"}
      ]
    }
  ]
}"#;
}
