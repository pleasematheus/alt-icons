# alt-icons

`alt-icons` lets a Windows executable change the icon embedded in its own `.exe`.
The available icons are declared at build time, then selected by name at runtime.

The executable file itself is rewritten and replaced on disk, so the selected icon
persists after a restart. This is different from changing an app's launcher entry.

## Install

Add both crates to your `Cargo.toml`:

```toml
[dependencies]
alt-icons = "1"

[build-dependencies]
alt-icons-build = "1"
```

Declare the icon set in `build.rs`:

```rust
fn main() {
    alt_icons_build::configure(&[
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

Licensed under either of Apache License, Version 2.0 or the MIT license, at your
option.
