# alt-icons

`alt-icons` lets a Windows executable change the icon embedded in its own `.exe`.
The available icons are declared at build time, then selected by name at runtime.

The executable file itself is rewritten and replaced on disk, so the selected icon
persists after a restart. This is different from changing an app's launcher entry.

## Install

Add `alt-icons` as both a runtime dependency and a build dependency. The `build`
feature enables the helper used from `build.rs`:

```toml
[dependencies]
alt-icons = "1.2"

[build-dependencies]
alt-icons = { version = "1.2", features = ["build"] }
```

Declare the icon set in `build.rs`:

```rust
fn main() {
    alt_icons::build::configure(&[
        ("Default", "assets/default.ico"),
        ("Dark", "assets/dark.ico"),
    ]);
}
```

Then include the generated enum and switch icons from your application:

```rust
alt_icons::include_icons!();

fn main() -> Result<(), alt_icons::Error> {
    alt_icons::init()?;
    alt_icons::set_icon(AppIcon::Dark)?;
    Ok(())
}
```

`Default` is required and is embedded in the executable during the build. Icon
paths in `build.rs` are relative to the application's crate root.

## Cleanup options

By default, the first swap parks the running executable in a uniquely named `.old`
file. A hidden system Windows PowerShell helper deletes it after the application
exits, without needing another launch. If other instances still use the old image,
the helper waits for other instances launched from the same executable path too.

To preserve `.old` files, replace `init()` with:

```rust
alt_icons::init_with_options(alt_icons::Options {
    cleanup_old: false,
})?;
```

Configure this once at startup, before changing icons. The option applies
process-wide to startup cleanup and later swaps. Use it on each launch to keep
preserving files; the default `init()` enables cleanup again. Abandoned `.new`
staging files are always cleaned up. Disabling cleanup does not cancel helpers
already started by an earlier swap or by another instance.

Automatic cleanup uses Windows PowerShell with no window, profiles, inherited
console streams, or additional files. Temporary locks are retried for up to 15
seconds after the application instances exit. If PowerShell is unavailable or
blocked by system policy, or deletion fails, the next `init()` retries cleanup.
Cleanup failure does not turn a completed icon change into an error.

## Requirements and behavior

- The runtime icon change works on Windows. The crate still builds on other
  platforms, where `init()` is a no-op and `set_icon()` returns
  `Error::UnsupportedPlatform`.
- The executable's directory must be writable by the application. The crate does
  not copy the executable elsewhere or request elevation.
- Rewriting the executable changes its hash and invalidates any Authenticode
  signature. Heuristic antivirus and SmartScreen may flag this behavior.
- An Explorer window that was already showing the executable can temporarily keep
  a cached icon. Restarting Explorer clears that stale display.

## License

Apache-2.0
