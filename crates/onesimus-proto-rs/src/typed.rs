//! Descriptor-driven codec between the Director's API 2 JSON results
//! (`.api 2`) and the typed messages of `director_api.proto`.
//!
//! The typed records use the API 2 JSON keys as field names, so one generic
//! mapping serves every RPC:
//!
//! * a JSON key is matched to the field of the same name (lower case, `-`
//!   and blanks become `_`, a leading `.` is dropped);
//! * strings are converted to the field type only when the conversion is
//!   lossless (`"0042"` does not become 42); booleans accept `1/0`,
//!   `t/f`, `true/false`, `yes/no`;
//! * enums accept the number, a one-character catalog code (the numbers of
//!   `JobStatus`, `JobType`, `JobLevel` are the ASCII codes) or the value
//!   name without prefix, ignoring case, `_`, `-` and blanks
//!   (`"VolumeToCatalog"` -> `JOB_LEVEL_VOLUME_TO_CATALOG`);
//! * a `[{"count": n}]` array becomes the `count` field of the parent;
//! * a JSON array for a message field whose type name ends in `List` fills
//!   the first repeated field of that wrapper;
//! * every key without a field, or whose value cannot be converted, goes to
//!   `extra_fields` (strings as they are, other values as JSON text), so no
//!   value is lost; a `google.protobuf.Struct directives` field receives the
//!   whole object;
//! * `null` means "absent".
//!
//! [`message_to_json`] is the inverse direction (wire bytes -> JSON object
//! with the proto field names), used to turn requests into console arguments
//! and by tests to compare typed results with the console output.

use std::collections::HashMap;
use std::sync::OnceLock;

use prost::encoding::{decode_varint, encode_key, encode_varint, WireType};
use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorSet, ServiceDescriptorProto};
use serde_json::{Map, Value};

/// Serialized `FileDescriptorSet` of all compiled protos (with imports).
pub const DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/onesimus_descriptor.bin"));

/// Full name of the typed service.
pub const DIRECTOR_API_SERVICE: &str = "onesimus.director.api.DirectorApi";

pub struct Registry {
    messages: HashMap<String, DescriptorProto>,
    enums: HashMap<String, EnumDescriptorProto>,
    services: HashMap<String, ServiceDescriptorProto>,
}

/// The descriptors of [`DESCRIPTOR_SET`], parsed once.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::load(DESCRIPTOR_SET))
}

impl Registry {
    fn load(bytes: &[u8]) -> Self {
        let set = FileDescriptorSet::decode(bytes).expect("embedded descriptor set");
        let mut registry = Registry { messages: HashMap::new(), enums: HashMap::new(), services: HashMap::new() };
        for file in set.file {
            let package = file.package().to_string();
            let prefix = format!(".{package}");
            for message in file.message_type {
                registry.add_message(&prefix, message);
            }
            for e in file.enum_type {
                registry.enums.insert(format!("{prefix}.{}", e.name()), e);
            }
            for service in file.service {
                registry.services.insert(format!("{package}.{}", service.name()), service);
            }
        }
        registry
    }

    fn add_message(&mut self, prefix: &str, message: DescriptorProto) {
        let full = format!("{prefix}.{}", message.name());
        for nested in &message.nested_type {
            self.add_message(&full, nested.clone());
        }
        for e in &message.enum_type {
            self.enums.insert(format!("{full}.{}", e.name()), e.clone());
        }
        self.messages.insert(full, message);
    }

    /// Message descriptor by full name (with or without leading `.`).
    pub fn message(&self, name: &str) -> Option<&DescriptorProto> {
        self.messages.get(&dotted(name))
    }

    pub fn enumeration(&self, name: &str) -> Option<&EnumDescriptorProto> {
        self.enums.get(&dotted(name))
    }

    pub fn service(&self, full_name: &str) -> Option<&ServiceDescriptorProto> {
        self.services.get(full_name.trim_start_matches('.'))
    }

    /// RPC names of a service in declaration order.
    pub fn rpc_names(&self, service: &str) -> Vec<String> {
        self.service(service).map(|s| s.method.iter().map(|m| m.name().to_string()).collect()).unwrap_or_default()
    }

    /// (input type, output type) of an RPC, full names with leading `.`.
    pub fn rpc_types(&self, service: &str, rpc: &str) -> Option<(String, String)> {
        self.service(service)?.method.iter().find(|m| m.name() == rpc)
            .map(|m| (m.input_type().to_string(), m.output_type().to_string()))
    }

    fn map_entry(&self, field: &FieldDescriptorProto) -> Option<&DescriptorProto> {
        if field.label() != Label::Repeated || field.r#type() != Type::Message {
            return None;
        }
        self.message(field.type_name()).filter(|m| m.options.as_ref().is_some_and(|o| o.map_entry()))
    }
}

fn dotted(name: &str) -> String {
    if name.starts_with('.') { name.to_string() } else { format!(".{name}") }
}

/// Normalized JSON key -> proto field name.
pub fn field_name(key: &str) -> String {
    key.trim_start_matches('.').to_ascii_lowercase().replace(['-', ' '], "_")
}

/// Text stored in `extra_fields`: strings as they are, other values as JSON.
pub fn extra_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// JSON -> protobuf
// ---------------------------------------------------------------------------

/// Result of [`json_to_message`].
#[derive(Debug, Default)]
pub struct Encoded {
    /// Wire bytes of the message (decode with the generated type).
    pub bytes: Vec<u8>,
    /// JSON keys that found neither a field nor an `extra_fields` map
    /// (path like `header.foo`); empty when nothing was lost.
    pub unmapped: Vec<String>,
}

/// Encode a JSON object as the message `type_name` (see module docs).
pub fn json_to_message(type_name: &str, value: &Value) -> Encoded {
    let registry = registry();
    let mut encoder = Encoder { registry, unmapped: Vec::new() };
    let mut bytes = Vec::new();
    match (registry.message(type_name), value) {
        (Some(desc), Value::Object(object)) => encoder.message(desc, object, "", &mut bytes),
        (None, _) => encoder.unmapped.push(format!("<unknown message {type_name}>")),
        (_, _) => encoder.unmapped.push("<root is not an object>".into()),
    }
    Encoded { bytes, unmapped: encoder.unmapped }
}

struct Encoder<'a> {
    registry: &'a Registry,
    unmapped: Vec<String>,
}

fn put_len_delimited(number: i32, data: &[u8], buf: &mut Vec<u8>) {
    encode_key(number as u32, WireType::LengthDelimited, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

fn put_string(number: i32, s: &str, buf: &mut Vec<u8>) {
    put_len_delimited(number, s.as_bytes(), buf);
}

fn put_varint(number: i32, v: u64, buf: &mut Vec<u8>) {
    encode_key(number as u32, WireType::Varint, buf);
    encode_varint(v, buf);
}

/// `[{"count": n}]` (list ... count)
fn count_only(items: &[Value]) -> Option<&Value> {
    match items {
        [Value::Object(o)] if o.len() == 1 => o.get("count"),
        _ => None,
    }
}

impl Encoder<'_> {
    fn message(&mut self, desc: &DescriptorProto, object: &Map<String, Value>, path: &str, buf: &mut Vec<u8>) {
        let extra = desc.field.iter().find(|f| f.name() == "extra_fields" && self.registry.map_entry(f).is_some());
        let directives = desc.field.iter()
            .find(|f| f.name() == "directives" && f.type_name() == ".google.protobuf.Struct");
        let count = desc.field.iter().find(|f| f.name() == "count" && f.label() != Label::Repeated);
        let mut extras = Vec::new();
        for (key, value) in object {
            if value.is_null() {
                continue;
            }
            let name = field_name(key);
            let special = |f: &FieldDescriptorProto| [extra, directives].iter().flatten().any(|s| s.number() == f.number());
            let field = desc.field.iter().find(|f| f.name() == name && !special(f));
            let mut converted = false;
            if let Some(field) = field {
                if let (Value::Array(items), Some(count)) = (value, count) {
                    if let Some(n) = count_only(items).filter(|_| field.r#type() == Type::Message) {
                        let mut tmp = Vec::new();
                        if self.singular(count, n, &format!("{path}{key}."), &mut tmp).is_ok() {
                            buf.extend(tmp);
                            continue;
                        }
                    }
                }
                let mut tmp = Vec::new();
                if self.field(field, value, &format!("{path}{key}."), &mut tmp).is_ok() {
                    buf.extend(tmp);
                    converted = true;
                }
            }
            if !converted {
                if extra.is_some() {
                    extras.push((key.clone(), extra_text(value)));
                } else if directives.is_none() {
                    self.unmapped.push(format!("{path}{key}"));
                }
            }
        }
        if let Some(directives) = directives {
            let mut inner = Vec::new();
            encode_struct(object, &mut inner);
            put_len_delimited(directives.number(), &inner, buf);
        }
        if let Some(extra) = extra {
            for (key, text) in extras {
                let mut entry = Vec::new();
                put_string(1, &key, &mut entry);
                put_string(2, &text, &mut entry);
                put_len_delimited(extra.number(), &entry, buf);
            }
        }
    }

    fn field(&mut self, field: &FieldDescriptorProto, value: &Value, path: &str, buf: &mut Vec<u8>) -> Result<(), ()> {
        if field.label() != Label::Repeated {
            return self.singular(field, value, path, buf);
        }
        if let Some(entry) = self.registry.map_entry(field) {
            let Value::Object(object) = value else { return Err(()) };
            let value_field = entry.field.iter().find(|f| f.number() == 2).ok_or(())?;
            for (key, item) in object {
                let mut data = Vec::new();
                put_string(1, key, &mut data);
                if !item.is_null() {
                    self.singular(value_field, item, &format!("{path}{key}."), &mut data)?;
                }
                put_len_delimited(field.number(), &data, buf);
            }
            return Ok(());
        }
        let Value::Array(items) = value else { return Err(()) };
        for item in items {
            self.singular(field, item, path, buf)?;
        }
        Ok(())
    }

    fn singular(&mut self, field: &FieldDescriptorProto, value: &Value, path: &str, buf: &mut Vec<u8>) -> Result<(), ()> {
        let n = field.number();
        match field.r#type() {
            Type::String => {
                let Value::String(s) = value else { return Err(()) };
                put_string(n, s, buf);
            }
            Type::Bool => put_varint(n, parse_bool(value)? as u64, buf),
            Type::Int32 | Type::Sint32 | Type::Sfixed32 => {
                let v = parse_i64(value)?;
                i32::try_from(v).map_err(|_| ())?;
                put_varint(n, v as u64, buf);
            }
            Type::Int64 => put_varint(n, parse_i64(value)? as u64, buf),
            Type::Uint32 => {
                let v = parse_u64(value)?;
                u32::try_from(v).map_err(|_| ())?;
                put_varint(n, v, buf);
            }
            Type::Uint64 => put_varint(n, parse_u64(value)?, buf),
            Type::Double => {
                let v = parse_f64(value)?;
                encode_key(n as u32, WireType::SixtyFourBit, buf);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Type::Enum => {
                let v = resolve_enum(self.registry, field.type_name(), value)?;
                put_varint(n, v as i64 as u64, buf);
            }
            Type::Message => {
                let mut inner = Vec::new();
                match field.type_name() {
                    ".google.protobuf.Struct" => {
                        let Value::Object(object) = value else { return Err(()) };
                        encode_struct(object, &mut inner);
                    }
                    ".google.protobuf.Value" => encode_value(value, &mut inner),
                    ".google.protobuf.ListValue" => {
                        let Value::Array(items) = value else { return Err(()) };
                        encode_list(items, &mut inner);
                    }
                    type_name => {
                        let desc = self.registry.message(type_name).ok_or(())?;
                        match value {
                            Value::Object(object) => self.message(desc, object, path, &mut inner),
                            Value::Array(_) if type_name.ends_with("List") => {
                                let first = desc.field.iter().find(|f| f.label() == Label::Repeated).ok_or(())?;
                                let mut wrapper = Map::new();
                                wrapper.insert(first.name().to_string(), value.clone());
                                self.message(desc, &wrapper, path, &mut inner);
                            }
                            _ => return Err(()),
                        }
                    }
                }
                put_len_delimited(n, &inner, buf);
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

pub fn parse_bool(value: &Value) -> Result<bool, ()> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => match n.as_u64() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(()),
        },
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "1" | "t" | "true" | "yes" => Ok(true),
            "0" | "f" | "false" | "no" => Ok(false),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

pub fn parse_i64(value: &Value) -> Result<i64, ()> {
    match value {
        Value::Number(n) => n.as_i64().ok_or(()),
        Value::String(s) => s.parse::<i64>().ok().filter(|v| v.to_string() == *s).ok_or(()),
        _ => Err(()),
    }
}

pub fn parse_u64(value: &Value) -> Result<u64, ()> {
    match value {
        Value::Number(n) => n.as_u64().ok_or(()),
        Value::String(s) => s.parse::<u64>().ok().filter(|v| v.to_string() == *s).ok_or(()),
        _ => Err(()),
    }
}

pub fn parse_f64(value: &Value) -> Result<f64, ()> {
    match value {
        Value::Number(n) => n.as_f64().ok_or(()),
        Value::String(s) => s.parse::<f64>().ok().filter(|v| v.is_finite() && v.to_string() == *s).ok_or(()),
        _ => Err(()),
    }
}

fn normalize_enum_text(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// Enum number for a JSON value: number, one-character code or name.
pub fn resolve_enum(registry: &Registry, type_name: &str, value: &Value) -> Result<i32, ()> {
    let e = registry.enumeration(type_name).ok_or(())?;
    let exists = |n: i32| n != 0 && e.value.iter().any(|v| v.number() == n);
    match value {
        Value::Number(n) => {
            let n = n.as_i64().and_then(|n| i32::try_from(n).ok()).ok_or(())?;
            if exists(n) { Ok(n) } else { Err(()) }
        }
        Value::String(s) => {
            let mut chars = s.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                if exists(c as i32) {
                    return Ok(c as i32);
                }
            }
            let prefix = e.value.first().map(|v| v.name().trim_end_matches("UNSPECIFIED")).unwrap_or("");
            let wanted = normalize_enum_text(s);
            if wanted.is_empty() {
                return Err(());
            }
            e.value.iter()
                .find(|v| v.number() != 0
                    && normalize_enum_text(v.name().strip_prefix(prefix).unwrap_or(v.name())) == wanted)
                .map(|v| v.number()).ok_or(())
        }
        _ => Err(()),
    }
}

fn encode_struct(object: &Map<String, Value>, buf: &mut Vec<u8>) {
    for (key, value) in object {
        let mut entry = Vec::new();
        put_string(1, key, &mut entry);
        let mut inner = Vec::new();
        encode_value(value, &mut inner);
        put_len_delimited(2, &inner, &mut entry);
        put_len_delimited(1, &entry, buf);
    }
}

fn encode_list(items: &[Value], buf: &mut Vec<u8>) {
    for item in items {
        let mut inner = Vec::new();
        encode_value(item, &mut inner);
        put_len_delimited(1, &inner, buf);
    }
}

fn encode_value(value: &Value, buf: &mut Vec<u8>) {
    match value {
        Value::Null => put_varint(1, 0, buf),
        Value::Number(n) => {
            encode_key(2, WireType::SixtyFourBit, buf);
            buf.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_le_bytes());
        }
        Value::String(s) => put_string(3, s, buf),
        Value::Bool(b) => put_varint(4, *b as u64, buf),
        Value::Object(o) => {
            let mut inner = Vec::new();
            encode_struct(o, &mut inner);
            put_len_delimited(5, &inner, buf);
        }
        Value::Array(a) => {
            let mut inner = Vec::new();
            encode_list(a, &mut inner);
            put_len_delimited(6, &inner, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// protobuf -> JSON
// ---------------------------------------------------------------------------

enum Raw<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

fn read_fields(mut buf: &[u8]) -> Vec<(i32, Raw<'_>)> {
    let mut out = Vec::new();
    while !buf.is_empty() {
        let Ok(key) = decode_varint(&mut buf) else { break };
        let number = (key >> 3) as i32;
        let raw = match key & 7 {
            0 => match decode_varint(&mut buf) {
                Ok(v) => Raw::Varint(v),
                Err(_) => break,
            },
            1 if buf.len() >= 8 => {
                let v = u64::from_le_bytes(buf[..8].try_into().unwrap());
                buf = &buf[8..];
                Raw::Fixed64(v)
            }
            2 => {
                let Ok(len) = decode_varint(&mut buf) else { break };
                let len = len as usize;
                if len > buf.len() {
                    break;
                }
                let (data, rest) = buf.split_at(len);
                buf = rest;
                Raw::Bytes(data)
            }
            5 if buf.len() >= 4 => {
                let v = u32::from_le_bytes(buf[..4].try_into().unwrap());
                buf = &buf[4..];
                Raw::Fixed32(v)
            }
            _ => break,
        };
        out.push((number, raw));
    }
    out
}

/// Decode wire bytes of `type_name` into a JSON object keyed by the proto
/// field names. Only fields present on the wire appear; enums are numbers,
/// maps are objects, `google.protobuf.Struct/Value` become plain JSON.
pub fn message_to_json(type_name: &str, bytes: &[u8]) -> Value {
    let registry = registry();
    match registry.message(type_name) {
        Some(desc) => Value::Object(decode_message(registry, desc, bytes)),
        None => Value::Null,
    }
}

fn decode_message(registry: &Registry, desc: &DescriptorProto, bytes: &[u8]) -> Map<String, Value> {
    let mut object = Map::new();
    for (number, raw) in read_fields(bytes) {
        let Some(field) = desc.field.iter().find(|f| f.number() == number) else { continue };
        let name = field.name().to_string();
        if let Some(entry) = registry.map_entry(field) {
            let Raw::Bytes(data) = raw else { continue };
            // proto3 leaves default key/value out of map entries
            let value_field = entry.field.iter().find(|f| f.number() == 2);
            let entry = decode_message(registry, entry, data);
            let key = match entry.get("key") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            let value = entry.get("value").cloned()
                .unwrap_or_else(|| value_field.map(default_value).unwrap_or(Value::Null));
            if let Value::Object(map) = object.entry(name).or_insert_with(|| Value::Object(Map::new())) {
                map.insert(key, value);
            }
            continue;
        }
        let values = decode_values(registry, field, raw);
        if field.label() == Label::Repeated {
            if let Value::Array(items) = object.entry(name).or_insert_with(|| Value::Array(Vec::new())) {
                items.extend(values);
            }
        } else if let Some(value) = values.into_iter().last() {
            object.insert(name, value);
        }
    }
    object
}

/// JSON of a proto3 default value (field absent on the wire).
fn default_value(field: &FieldDescriptorProto) -> Value {
    match field.r#type() {
        Type::String => Value::String(String::new()),
        Type::Bool => Value::Bool(false),
        Type::Message => Value::Object(Map::new()),
        Type::Bytes => Value::Array(Vec::new()),
        _ => Value::from(0),
    }
}

fn varint_value(t: Type, v: u64) -> Value {
    match t {
        Type::Bool => Value::Bool(v != 0),
        Type::Int32 | Type::Enum => Value::from(v as i64 as i32),
        Type::Int64 => Value::from(v as i64),
        Type::Sint32 | Type::Sint64 => Value::from(((v >> 1) as i64) ^ -((v & 1) as i64)),
        _ => Value::from(v),
    }
}

fn decode_values(registry: &Registry, field: &FieldDescriptorProto, raw: Raw<'_>) -> Vec<Value> {
    let t = field.r#type();
    match raw {
        Raw::Varint(v) => vec![varint_value(t, v)],
        Raw::Fixed64(v) => vec![match t {
            Type::Double => serde_json::Number::from_f64(f64::from_bits(v)).map(Value::Number).unwrap_or(Value::Null),
            Type::Sfixed64 => Value::from(v as i64),
            _ => Value::from(v),
        }],
        Raw::Fixed32(v) => vec![match t {
            Type::Float => serde_json::Number::from_f64(f32::from_bits(v) as f64).map(Value::Number).unwrap_or(Value::Null),
            Type::Sfixed32 => Value::from(v as i32),
            _ => Value::from(v),
        }],
        Raw::Bytes(data) => match t {
            Type::String => vec![Value::String(String::from_utf8_lossy(data).into_owned())],
            Type::Bytes => vec![Value::Array(data.iter().map(|b| Value::from(*b)).collect())],
            Type::Message => vec![match field.type_name() {
                ".google.protobuf.Struct" => Value::Object(decode_struct(data)),
                ".google.protobuf.Value" => decode_value(data),
                ".google.protobuf.ListValue" => decode_list(data),
                type_name => registry.message(type_name)
                    .map(|d| Value::Object(decode_message(registry, d, data))).unwrap_or(Value::Null),
            }],
            // packed repeated scalars
            Type::Double => data.chunks_exact(8)
                .map(|c| serde_json::Number::from_f64(f64::from_le_bytes(c.try_into().unwrap()))
                    .map(Value::Number).unwrap_or(Value::Null)).collect(),
            _ => {
                let mut values = Vec::new();
                let mut rest = data;
                while !rest.is_empty() {
                    let Ok(v) = decode_varint(&mut rest) else { break };
                    values.push(varint_value(t, v));
                }
                values
            }
        },
    }
}

fn decode_struct(bytes: &[u8]) -> Map<String, Value> {
    let mut object = Map::new();
    for (number, raw) in read_fields(bytes) {
        let (1, Raw::Bytes(entry)) = (number, raw) else { continue };
        let mut key = String::new();
        let mut value = Value::Null;
        for (n, r) in read_fields(entry) {
            match (n, r) {
                (1, Raw::Bytes(k)) => key = String::from_utf8_lossy(k).into_owned(),
                (2, Raw::Bytes(v)) => value = decode_value(v),
                _ => {}
            }
        }
        object.insert(key, value);
    }
    object
}

fn decode_list(bytes: &[u8]) -> Value {
    Value::Array(read_fields(bytes).into_iter()
        .filter_map(|(n, r)| match (n, r) { (1, Raw::Bytes(v)) => Some(decode_value(v)), _ => None })
        .collect())
}

fn decode_value(bytes: &[u8]) -> Value {
    let mut value = Value::Null;
    for (number, raw) in read_fields(bytes) {
        value = match (number, raw) {
            (1, _) => Value::Null,
            (2, Raw::Fixed64(v)) => {
                let f = f64::from_bits(v);
                if f.fract() == 0.0 && f.abs() < 9.0e15 { Value::from(f as i64) }
                else { serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null) }
            }
            (3, Raw::Bytes(s)) => Value::String(String::from_utf8_lossy(s).into_owned()),
            (4, Raw::Varint(b)) => Value::Bool(b != 0),
            (5, Raw::Bytes(s)) => Value::Object(decode_struct(s)),
            (6, Raw::Bytes(l)) => decode_list(l),
            _ => continue,
        };
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onesimus::director::api::{
        JobLevel, JobStatus, JobType, ListJobsResponse, ListVolumesResponse, ShowResponse, VolumeStatus,
    };
    use serde_json::json;

    fn decode<M: Message + Default>(name: &str, value: Value) -> (M, Vec<String>) {
        let encoded = json_to_message(name, &value);
        (M::decode(encoded.bytes.as_slice()).unwrap(), encoded.unmapped)
    }

    #[test]
    fn job_records_map_with_enums_counts_and_extra_fields() {
        let (r, unmapped): (ListJobsResponse, _) = decode(".onesimus.director.api.ListJobsResponse", json!({
            "jobs": [{"jobid": "479", "name": "benchmark-small", "type": "B", "level": "F",
                      "jobstatus": "T", "jobbytes": "824021543", "purgedfiles": "0", "duration": "00:00:02",
                      "jobtdate": "", "newcolumn": "x"}],
            "meta": {"range": {"limit": 2, "filtered": 0}}}));
        assert!(unmapped.is_empty(), "{unmapped:?}");
        let job = &r.jobs[0];
        assert_eq!(job.jobid, 479);
        assert_eq!(job.r#type(), JobType::Backup);
        assert_eq!(job.level(), JobLevel::Full);
        assert_eq!(job.jobstatus(), JobStatus::Terminated);
        assert_eq!(job.jobbytes, 824021543);
        assert!(!job.purgedfiles);
        // not convertible ("" for int64) and unknown keys are kept as text
        assert_eq!(job.extra_fields.get("jobtdate").map(String::as_str), Some(""));
        assert_eq!(job.extra_fields.get("newcolumn").map(String::as_str), Some("x"));
        assert_eq!(r.meta.unwrap().range.unwrap().limit, 2);

        let (r, _): (ListJobsResponse, _) = decode(".onesimus.director.api.ListJobsResponse",
            json!({"jobs": [{"count": "161"}]}));
        assert_eq!(r.count, Some(161));
        assert!(r.jobs.is_empty());
    }

    #[test]
    fn lossless_numbers_and_enum_names() {
        let (r, _): (ListJobsResponse, _) = decode(".onesimus.director.api.ListJobsResponse",
            json!({"jobs": [{"jobid": "0042", "level": "VirtualFull", "type": "Job Copy"}]}));
        assert_eq!(r.jobs[0].jobid, 0);
        assert_eq!(r.jobs[0].extra_fields["jobid"], "0042");
        assert_eq!(r.jobs[0].level(), JobLevel::VirtualFull);
        assert_eq!(r.jobs[0].r#type(), JobType::JobCopy);
    }

    #[test]
    fn volumes_by_pool_with_count_wrapper() {
        let (r, unmapped): (ListVolumesResponse, _) = decode(".onesimus.director.api.ListVolumesResponse", json!({
            "pools": {"full": [{"mediaid": "1", "volumename": "Full-0001", "volstatus": "Read-Only"}],
                      "scratch": [{"count": "0"}]}}));
        assert!(unmapped.is_empty(), "{unmapped:?}");
        assert_eq!(r.pools["full"].volumes[0].volstatus(), VolumeStatus::ReadOnly);
        assert_eq!(r.pools["scratch"].count, Some(0));
    }

    #[test]
    fn show_keeps_all_directives_and_roundtrips() {
        let client = json!({"name": "fd", "address": "localhost", "fdport": 9102, "enabled": true,
                            "tls": {"enable": true}, "list": ["a", 1]});
        let value = json!({"clients": {"fd": client.clone()}});
        let encoded = json_to_message("onesimus.director.api.ShowResponse", &value);
        let r = ShowResponse::decode(encoded.bytes.as_slice()).unwrap();
        assert_eq!(r.clients["fd"].fdport, 9102);
        let back = message_to_json("onesimus.director.api.ShowResponse", &encoded.bytes);
        assert_eq!(back["clients"]["fd"]["directives"], client);
        assert_eq!(back["clients"]["fd"]["address"], json!("localhost"));
    }

    #[test]
    fn service_rpcs_are_known() {
        let r = registry();
        assert!(r.rpc_names(DIRECTOR_API_SERVICE).iter().any(|n| n == "ListJobs"));
        let (input, output) = r.rpc_types(DIRECTOR_API_SERVICE, "GetVersion").unwrap();
        assert_eq!(input, ".onesimus.director.commands.VersionCommand");
        assert_eq!(output, ".onesimus.director.api.VersionResponse");
    }
}
