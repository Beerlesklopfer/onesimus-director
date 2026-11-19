// Module and type tree for rustdoc, generated at build time.
//
// Included with `include!` into the build.rs of the crates that follow the
// documentation rule (bareos-format, onesimus-daemon, onesimus-rig). The build
// script walks `src/` from `lib.rs`, follows every `mod name;` / `mod name {`
// declaration and lists the items each module declares (struct, enum, trait,
// fn, const, static, type, macro). The result is written to
// `$OUT_DIR/module_tree.md` as a fenced text block; the crate root includes it
// with `#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]`.
//
// It is a line scanner, not a Rust parser: it sees declarations that start a
// line (after indentation and visibility). Items in `#[cfg(test)]` modules are
// skipped. For a full, type-checked view install `cargo-modules`
// (`cargo install cargo-modules --locked`) and run
// `cargo modules structure --package <crate> --lib` – docs/wiki/dokumentation.md.

/// Writes `$OUT_DIR/module_tree.md` for the crate in `CARGO_MANIFEST_DIR`.
fn write_module_tree() {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let crate_name = std::env::var("CARGO_PKG_NAME").unwrap_or_default().replace('-', "_");
    let src = manifest.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut lines = vec![format!("{crate_name} (crate)")];
    let root = src.join("lib.rs");
    module_tree_walk(&root, &src, 1, &mut lines);
    let text = format!(
        "\n\n# Modul- und Typbaum\n\nBeim Bau erzeugt (`build.rs`, `scripts/doc/module_tree.rs`): Module und die \
         Elemente, die sie deklarieren. `pub` ist mit `+` markiert, privat mit `-`.\n\n```text\n{}\n```\n",
        lines.join("\n")
    );
    std::fs::write(out.join("module_tree.md"), text).expect("write module_tree.md");
}

/// Appends the items and child modules of the module in `file` to `lines`.
/// `dir` is the directory in which `mod child;` files of this module live.
fn module_tree_walk(file: &std::path::Path, dir: &std::path::Path, depth: usize, lines: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    let indent = "    ".repeat(depth);
    let mut skip_test = false;
    let mut brace_depth_at_test: Option<i32> = None;
    let mut braces = 0i32;
    let mut in_string = false;
    for raw in text.lines() {
        let line = raw.trim();
        let string_at_start = in_string;
        if let Some(start) = brace_depth_at_test {
            braces += module_tree_braces(line, &mut in_string);
            if braces <= start {
                brace_depth_at_test = None;
            }
            continue;
        }
        if line.starts_with("#[cfg(test)]") {
            skip_test = true;
            continue;
        }
        let opens = module_tree_braces(line, &mut in_string);
        let top_level = braces == 0;
        braces += opens;
        if string_at_start || line.starts_with("//") || line.starts_with('#') || !top_level {
            continue;
        }
        let (public, rest) = module_tree_visibility(line);
        let mut words = rest.split(|c: char| c.is_whitespace() || c == '(' || c == '<' || c == ':' || c == ';' || c == '{' || c == '=');
        let Some(mut kind) = words.next() else { continue };
        let mut name = words.find(|w| !w.is_empty()).unwrap_or("");
        if matches!(kind, "unsafe" | "async" | "const" | "extern") && matches!(name, "fn" | "trait" | "impl") {
            kind = name;
            name = rest.split_whitespace().nth(2).unwrap_or("").split(['(', '<', '{']).next().unwrap_or("");
        }
        if skip_test {
            skip_test = false;
            if kind == "mod" {
                if opens > 0 {
                    brace_depth_at_test = Some(braces - opens);
                }
                continue;
            }
        }
        let label = match kind {
            "mod" => "mod",
            "struct" => "struct",
            "enum" => "enum",
            "trait" => "trait",
            "fn" => "fn",
            "const" => "const",
            "static" => "static",
            "type" => "type",
            "macro_rules!" => "macro",
            _ => continue,
        };
        let marker = if public { '+' } else { '-' };
        lines.push(format!("{indent}{marker} {label} {name}"));
        if kind == "mod" {
            if opens > 0 {
                // inline module: its items follow inside the braces and are skipped
                continue;
            }
            let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let child_dir = if matches!(file_name, "lib.rs" | "main.rs" | "mod.rs") {
                dir.to_path_buf()
            } else {
                dir.join(file.file_stem().unwrap_or_default())
            };
            let flat = child_dir.join(format!("{name}.rs"));
            let nested = child_dir.join(name).join("mod.rs");
            if flat.exists() {
                module_tree_walk(&flat, &child_dir, depth + 1, lines);
            } else if nested.exists() {
                module_tree_walk(&nested, &child_dir.join(name), depth + 1, lines);
            }
        }
    }
}

/// `pub`, `pub(crate)` … and the rest of the line after the visibility.
fn module_tree_visibility(line: &str) -> (bool, &str) {
    if let Some(rest) = line.strip_prefix("pub(") {
        let after = rest.find(')').map(|i| &rest[i + 1..]).unwrap_or(rest);
        (true, after.trim_start())
    } else if let Some(rest) = line.strip_prefix("pub ") {
        (true, rest.trim_start())
    } else {
        (false, line)
    }
}

/// Net brace count of a line, ignoring braces in string and char literals;
/// `in_string` carries an open string literal over to the next line.
fn module_tree_braces(line: &str, in_string: &mut bool) -> i32 {
    let mut count = 0;
    let mut previous = ' ';
    let code = if !*in_string && line.trim_start().starts_with("//") { "" } else { line };
    for c in code.chars() {
        if !*in_string && c == '/' && previous == '/' {
            break;
        }
        match c {
            '"' if previous != '\\' && previous != '\'' => *in_string = !*in_string,
            '{' if !*in_string && previous != '\'' => count += 1,
            '}' if !*in_string && previous != '\'' => count -= 1,
            _ => {}
        }
        previous = c;
    }
    count
}
