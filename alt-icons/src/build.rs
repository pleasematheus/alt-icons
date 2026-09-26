//! Build-script side of `alt-icons`.
//!
//! Call [`configure`] from your `build.rs` with the complete icon set. It does two
//! things: bakes the `Default` icon into the executable as a real `RT_GROUP_ICON`
//! resource, so the binary you ship already has your icon, and generates the
//! `AppIcon` enum that [`alt_icons::set_icon`] takes.
//!
//! The build script is the single declaration site on purpose. It runs before macro
//! expansion, so it could never read an icon list declared by a macro — which means
//! a macro-based API would force the same list to be written twice, in two places
//! that can silently drift apart.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Declares the icon set. Exactly one entry must be named `Default`; it is the icon
/// baked into the binary at build time.
///
/// ```no_run
/// // build.rs
/// fn main() {
///     alt_icons::build::configure(&[
///         ("Default", "assets/default.ico"),
///         ("Dark", "assets/dark.ico"),
///     ]);
/// }
/// ```
///
/// # Panics
///
/// Panics on anything that would produce a broken build: an empty set, a missing
/// `Default`, a duplicate name, a name that is not a valid Rust identifier, or an
/// icon file that does not exist. A build script has no better failure channel, and
/// each of these is a mistake in the caller's own source.
// The `fn main` in the example above is the point: it is a build script, and showing
// it without one would be showing something that does not run.
#[allow(clippy::needless_doctest_main)]
pub fn configure(icons: &[(&str, &str)]) {
    let target_os = env::var("CARGO_CFG_TARGET_OS")
        .expect("CARGO_CFG_TARGET_OS is always set for Cargo build scripts");
    if target_os != "windows" {
        // alt-icons is Windows-only. Generate an empty set elsewhere so that a
        // cross-platform crate can still `cargo check` on other hosts.
        write_generated(&generate_stub());
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set for build scripts"),
    );

    let resolved = resolve(icons, &manifest_dir);
    let default = resolved
        .iter()
        .find(|entry| entry.name == "Default")
        .expect("the icon set must contain an entry named `Default`");

    for entry in &resolved {
        println!("cargo:rerun-if-changed={}", entry.path.display());
    }

    bake_default(&default.path);
    write_generated(&generate(&resolved));
}

struct Entry {
    name: String,
    path: PathBuf,
}

fn resolve(icons: &[(&str, &str)], manifest_dir: &Path) -> Vec<Entry> {
    assert!(!icons.is_empty(), "the icon set must not be empty");

    let mut resolved: Vec<Entry> = Vec::with_capacity(icons.len());
    for (name, path) in icons {
        assert!(
            is_ident(name),
            "`{name}` is not a valid Rust identifier, so it cannot be an AppIcon variant"
        );
        assert!(
            !resolved.iter().any(|entry| entry.name == *name),
            "duplicate icon name `{name}`"
        );

        // Relative paths are resolved against the crate root, which is what a person
        // writing `assets/dark.ico` in build.rs means. Canonicalize so the generated
        // `.rc` and the generated `include_bytes!` both hold an absolute path.
        let joined = manifest_dir.join(path);
        let canonical = fs::canonicalize(&joined)
            .unwrap_or_else(|e| panic!("icon `{}` not found at {}: {e}", name, joined.display()));

        resolved.push(Entry {
            name: (*name).to_owned(),
            path: strip_unc(canonical),
        });
    }
    resolved
}

/// Writes a one-line `.rc` into `OUT_DIR` and compiles it, so the shipped binary
/// carries the default icon as `RT_GROUP_ICON` id 1 — the same id `set_icon`
/// rewrites later.
fn bake_default(ico: &Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set for build scripts"));
    let rc_path = out_dir.join("alt_icons.rc");

    // The `.rc` lives in OUT_DIR while the icon lives in the crate, so the path in it
    // has to be absolute. Backslashes are escapes in `.rc` string literals.
    //
    // `LANGUAGE 0, 0` pins the baked resource to the neutral language, which is the
    // same language `set_icon` writes with. Without that, a later update would sit
    // beside the baked icon as a second translation instead of replacing it.
    let escaped = ico.display().to_string().replace('\\', "\\\\");
    fs::write(&rc_path, format!("LANGUAGE 0, 0\n1 ICON \"{escaped}\"\n"))
        .unwrap_or_else(|e| panic!("could not write {}: {e}", rc_path.display()));

    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap_or_else(|e| panic!("could not compile {}: {e}", rc_path.display()));
}

fn generate(entries: &[Entry]) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub enum AppIcon {\n");
    for entry in entries {
        let _ = writeln!(out, "    {},", entry.name);
    }
    out.push_str("}\n\nimpl AppIcon {\n    /// Every icon in the set, in declaration order.\n");
    out.push_str("    pub const ALL: &'static [AppIcon] = &[\n");
    for entry in entries {
        let _ = writeln!(out, "        AppIcon::{},", entry.name);
    }
    out.push_str("    ];\n}\n\nimpl alt_icons::Icon for AppIcon {\n");

    out.push_str("    fn name(&self) -> &'static str {\n        match self {\n");
    for entry in entries {
        let _ = writeln!(
            out,
            "            AppIcon::{} => \"{}\",",
            entry.name, entry.name
        );
    }
    out.push_str("        }\n    }\n\n");

    out.push_str("    fn bytes(&self) -> &'static [u8] {\n        match self {\n");
    for entry in entries {
        let _ = writeln!(
            out,
            "            AppIcon::{} => include_bytes!(r\"{}\"),",
            entry.name,
            entry.path.display()
        );
    }
    out.push_str("        }\n    }\n}\n");
    out
}

fn generate_stub() -> String {
    let mut out = String::from(HEADER);
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub enum AppIcon {}\n\n");
    out.push_str("impl AppIcon {\n    pub const ALL: &'static [AppIcon] = &[];\n}\n\n");
    out.push_str("impl alt_icons::Icon for AppIcon {\n");
    out.push_str("    fn name(&self) -> &'static str {\n        match *self {}\n    }\n\n");
    out.push_str("    fn bytes(&self) -> &'static [u8] {\n        match *self {}\n    }\n}\n");
    out
}

fn write_generated(code: &str) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set for build scripts"));
    let path = out_dir.join("alt_icons.rs");
    fs::write(&path, code).unwrap_or_else(|e| panic!("could not write {}: {e}", path.display()));
}

const HEADER: &str = "// Generated by alt-icons::build. Do not edit.\n\n";

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `canonicalize` hands back `\\?\C:\...` on Windows. The resource compiler and
/// `include_bytes!` both cope better with a plain path.
fn strip_unc(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}
