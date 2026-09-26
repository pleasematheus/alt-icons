# alt-icons

A Windows executable that changes its own icon, permanently.

Add the same crate to your application's runtime and build dependencies. The `build`
feature enables its helper API inside `build.rs`:

```toml
[dependencies]
alt-icons = "1"

[build-dependencies]
alt-icons = { version = "1.1", features = ["build"] }
```

```rust
alt_icons::include_icons!();

fn main() -> Result<(), alt_icons::Error> {
    alt_icons::init()?;
    alt_icons::set_icon(AppIcon::Dark)?;
    Ok(())
}
```

```rust
// build.rs
fn main() {
    alt_icons::build::configure(&[
        ("Default", "assets/default.ico"),
        ("Dark", "assets/dark.ico"),
    ]);
}
```

## What it actually does

The model is borrowed from iOS alternate icons: a closed set of icons, declared at
build time, switched by name at runtime.

What is **not** borrowed is where the icon lives. On iOS and Android the thing that
changes is the launcher entry. Here it is the executable file itself — the crate
rewrites the icon resources inside the running binary and replaces it on disk. So the
new icon is the one Explorer shows for the `.exe`, and it survives a reboot.

Under the hood, a swap: takes a named mutex, parses and validates the whole `.ico` in
memory, writes a patched copy of the executable next to it, renames the running image
aside, moves the patched copy into place, and tells the shell. A later run deletes the
file it parked.

## What it costs

**The binary rewrites itself, so its hash changes on every swap.** Heuristic antivirus
and SmartScreen reputation both notice that. The default icon is baked at build time
by `alt-icons`'s build feature precisely so this only happens when your user asks for a different
icon — never on the first launch of a freshly downloaded binary.

**It is incompatible with code signing.** Rewriting the PE invalidates an Authenticode
signature.

**Windows only.** On other platforms every call is a no-op or an
`Error::UnsupportedPlatform`, so a cross-platform crate still builds.

**It needs to write to its own directory.** If it cannot, you get
`Error::PathNotWritable` and nothing happens. The crate will not copy itself somewhere
writable, and will not ask for elevation.

## The Explorer caveat

The file's icon changes, and any process that asks the shell afterwards gets the new
one — measured, both at 16×16 and 32×32.

An Explorer window that was **already showing that file** is another matter. It can
keep the old icon, most visibly the small one, and when it does, nothing fixes it
except restarting Explorer. `SHCNE_ASSOCCHANGED`, `SHCNE_UPDATEITEM`, `SHCNE_UPDATEDIR`,
`SHCNE_DELETE` + `SHCNE_CREATE`, `SHCNE_RENAMEITEM`, `SHCNE_ATTRIBUTES`, touching the
timestamp and pressing F5 were all tried; none of them reliably clears it. The stale
entry lives in `explorer.exe`'s own memory, keyed by path, and the path is the one
thing this crate cannot change.

The crate still calls `SHChangeNotify`, because it sometimes helps and costs nothing.
It is not what makes the change take effect.

See [`DESIGN.md`](DESIGN.md) for what was measured and how.

## Why the icon set lives in `build.rs`

Because it has to. A build script runs before macro expansion, so it can never read a
list declared by a macro — and the default icon must be baked at build time, or the
binary you ship has no icon at all. Declaring the set in both places would mean two
sources of truth that drift apart silently. So there is one, and it is the build
script; it generates the `AppIcon` enum that `include_icons!()` pulls in.

## Layout

| Path | What it is |
|---|---|
| `alt-icons/` | The runtime crate. |
| `fixtures/demo/` | A tiny binary that swaps its own icon, and the integration tests. |

## License

MIT or Apache-2.0, at your option.
