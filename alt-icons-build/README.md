# alt-icons-build

`alt-icons-build` is the build-script helper for [`alt-icons`](https://crates.io/crates/alt-icons).
It embeds the default `.ico` as a Windows executable resource and generates the
`AppIcon` enum used by the runtime crate.

## Install

Add both crates to your application's `Cargo.toml`:

```toml
[dependencies]
alt-icons = "1"

[build-dependencies]
alt-icons-build = "1"
```

Call `configure` from your application's `build.rs`:

```rust
fn main() {
    alt_icons_build::configure(&[
        ("Default", "assets/default.ico"),
        ("Dark", "assets/dark.ico"),
    ]);
}
```

Exactly one icon must be named `Default`. Names become Rust enum variants, and
paths are resolved relative to the application's crate root. Include the generated
enum in the application with `alt_icons::include_icons!()`.

For Windows targets, the default icon is compiled into the `.exe` as a resource.
For other targets, the helper emits an empty enum so applications can still compile
for those platforms.

## License

Licensed under either of Apache License, Version 2.0 or the MIT license, at your
option.
