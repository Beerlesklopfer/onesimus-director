//! Code-Generator: erzeugt Rust-Structs aus JSON-Schema.

use crate::schema::{CatalogSchema, TableDef};

/// Generiere Rust-Code für alle Tabellen im Schema.
pub fn generate_rust_models(schema: &CatalogSchema) -> String {
    let mut code = String::new();

    code.push_str("// Auto-generated from Bareos Catalog JSON Schema\n");
    code.push_str(&format!("// Schema Version: {}\n", schema.version));
    code.push_str("// DO NOT EDIT — regenerate with bareos-orm codegen\n\n");
    code.push_str("use serde::{Serialize, Deserialize};\n");
    code.push_str("use chrono::NaiveDateTime;\n\n");

    for table in &schema.tables {
        code.push_str(&generate_struct(table));
        code.push('\n');
    }

    code
}

fn generate_struct(table: &TableDef) -> String {
    let struct_name = to_pascal_case(&table.name);
    let mut code = String::new();

    if !table.description.is_empty() {
        code.push_str(&format!("/// {}\n", table.description));
    }
    code.push_str("#[derive(Debug, Clone, Serialize, Deserialize)]\n");
    code.push_str(&format!("pub struct {} {{\n", struct_name));

    for col in &table.columns {
        if !col.description.is_empty() {
            code.push_str(&format!("    /// {}\n", col.description));
        }
        let rust_type = sql_to_rust_type(&col.col_type, !col.not_null);
        code.push_str(&format!("    pub {}: {},\n", col.name, rust_type));
    }

    code.push_str("}\n");
    code
}

fn sql_to_rust_type(sql_type: &str, nullable: bool) -> String {
    let base = match sql_type.to_lowercase().as_str() {
        "serial" | "integer" | "int" | "int4" => "i32",
        "bigserial" | "bigint" | "int8" => "i64",
        "smallint" | "int2" => "i16",
        "boolean" | "bool" => "bool",
        "text" | "varchar" => "String",
        "char(1)" => "String",
        "timestamp" | "timestamptz" => "NaiveDateTime",
        "jsonb" | "json" => "serde_json::Value",
        "bytea" => "Vec<u8>",
        "real" | "float4" => "f32",
        "double precision" | "float8" => "f64",
        _ => "String", // fallback
    };

    if nullable {
        format!("Option<{base}>")
    } else {
        base.to_string()
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case() {
        assert_eq!(to_pascal_case("job"), "Job");
        assert_eq!(to_pascal_case("file_set"), "FileSet");
        assert_eq!(to_pascal_case("bareos_schema_version"), "BareosSchemaVersion");
    }

    #[test]
    fn sql_types() {
        assert_eq!(sql_to_rust_type("serial", false), "i32");
        assert_eq!(sql_to_rust_type("bigint", false), "i64");
        assert_eq!(sql_to_rust_type("text", false), "String");
        assert_eq!(sql_to_rust_type("boolean", false), "bool");
        assert_eq!(sql_to_rust_type("timestamp", true), "Option<NaiveDateTime>");
        assert_eq!(sql_to_rust_type("jsonb", true), "Option<serde_json::Value>");
    }

    #[test]
    fn generate_struct_from_schema() {
        let schema: CatalogSchema = serde_json::from_str(r#"{
            "version": 1, "description": "test", "created": "2026", "author": "test",
            "tables": [{
                "name": "job",
                "description": "Backup jobs",
                "primary_key": "jobid",
                "columns": [
                    {"name": "jobid", "type": "serial", "not_null": true},
                    {"name": "name", "type": "text", "not_null": true},
                    {"name": "starttime", "type": "timestamp"},
                    {"name": "metadata", "type": "jsonb"}
                ]
            }]
        }"#).unwrap();

        let code = generate_rust_models(&schema);
        assert!(code.contains("pub struct Job"));
        assert!(code.contains("pub jobid: i32"));
        assert!(code.contains("pub name: String"));
        assert!(code.contains("pub starttime: Option<NaiveDateTime>"));
        assert!(code.contains("pub metadata: Option<serde_json::Value>"));
    }
}
