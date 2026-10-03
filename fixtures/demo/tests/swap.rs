//! End-to-end test of a real swap.
//!
//! It runs the fixture binary, then asks Windows itself which `RT_ICON` the
//! executable resolves to and compares those bytes against the source `.ico`. That
//! is the assertion that matters: it proves the `RT_GROUP_ICON` directory and the
//! `RT_ICON` entries agree, which is the part that is easy to get subtly wrong.
//!
//! It deliberately does not assert on the resource *id*. The id comes out of
//! `LookupIconIdFromDirectoryEx`, whose size pick depends on the icon file and on
//! system metrics, so asserting on it makes the test brittle for no extra proof.

#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod win;

const DEFAULT_ICO: &[u8] = include_bytes!("../assets/default.ico");
const DARK_ICO: &[u8] = include_bytes!("../assets/dark.ico");

#[test]
fn swaps_icons_and_reports_the_active_one() {
    let dir = scratch_dir("swap");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).expect("fixture binary should be copyable");

    let baseline = len(&exe);
    assert_eq!(run(&exe, &[]), "Default", "a fresh binary reports Default");
    assert_resolves_to(&exe, DEFAULT_ICO, "the baked default icon");

    assert_eq!(run(&exe, &["Dark"]), "Dark");
    assert_eq!(
        run(&exe, &[]),
        "Dark",
        "the active icon is read back from disk"
    );
    assert_resolves_to(&exe, DARK_ICO, "the icon after switching to Dark");

    assert_eq!(run(&exe, &["Default"]), "Default");
    assert_resolves_to(&exe, DEFAULT_ICO, "the icon after switching back");

    assert_eq!(
        len(&exe),
        baseline,
        "switching away and back must not accrete dead RT_ICON resources"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn several_swaps_in_one_process_do_not_collide() {
    // The first swap of a run renames the running image out of the way; every later
    // swap in that same process finds the path free and takes the plain-replace
    // path instead. Separate invocations can only ever reach the first branch, so
    // this has to happen inside one process.
    let dir = scratch_dir("cycle");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).expect("fixture binary should be copyable");

    assert_eq!(
        run(&exe, &["--keep-old", "cycle", "Dark", "Default", "Dark"]),
        "Dark"
    );
    assert_resolves_to(&exe, DARK_ICO, "the icon after three swaps in one process");

    // Only the first swap parks anything. If this is 3, the later swaps are taking
    // the rename branch when the path is already free.
    assert_eq!(
        leftovers(&dir),
        1,
        "only the first swap of a process should park the running image"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn leftovers_are_cleaned_up_by_a_later_run() {
    let dir = scratch_dir("cleanup");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).expect("fixture binary should be copyable");

    run(&exe, &["--keep-old", "Dark"]);
    assert_eq!(leftovers(&dir), 1, "a swap parks the running image");

    // The parked file stays locked only while the process that swapped is alive.
    // These runs are short-lived, so by the time the next one starts the lock is
    // gone and `init` can delete it. A long-running app keeps its own leftover until
    // it exits, and a later run clears it.
    run(&exe, &[]);
    assert_eq!(
        leftovers(&dir),
        0,
        "a later run clears what the swap parked"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_cleanup_runs_after_exit_without_another_launch() {
    // Apostrophes, spaces, Unicode, and shell metacharacters must stay literal in
    // the helper command. A similarly named unrelated file must remain untouched.
    let dir = scratch_dir("automatic ' ação $ &");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).unwrap();
    let unrelated = dir.join("demo.exe.backup.old");
    std::fs::write(&unrelated, b"keep").unwrap();
    let mut app = hold(&exe, Some("Dark"));
    assert_eq!(old_files(&dir), 1, "the running image must still exist");
    app.0.stdin.take();
    assert!(app.0.wait().unwrap().success());
    wait_for_no_old_files(&dir);
    assert_resolves_to(&exe, DARK_ICO, "the new executable survives cleanup");
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn automatic_cleanup_handles_several_swaps_in_one_process() {
    let dir = scratch_dir("automatic-cycle");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).unwrap();
    assert_eq!(run(&exe, &["cycle", "Dark", "Default", "Dark"]), "Dark");
    wait_for_no_old_files(&dir);
    assert_resolves_to(&exe, DARK_ICO, "the icon after automatic cycle cleanup");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn automatic_cleanup_survives_an_abrupt_exit() {
    let dir = scratch_dir("abrupt");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).unwrap();
    let mut app = hold(&exe, Some("Dark"));
    assert_eq!(old_files(&dir), 1);
    app.0.kill().unwrap();
    app.0.wait().unwrap();
    wait_for_no_old_files(&dir);
    assert_resolves_to(&exe, DARK_ICO, "the icon survives a forced exit");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cleanup_waits_for_other_instances_sharing_the_old_image() {
    let dir = scratch_dir("shared");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).unwrap();
    let mut other = hold(&exe, None);
    run(&exe, &["Dark"]);
    assert_eq!(
        old_files(&dir),
        1,
        "the other instance still uses the old image"
    );
    // Give the helper time to discover the remaining reader; cleanup must not
    // depend on the instance that swapped still being alive.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(old_files(&dir), 1);
    other.0.stdin.take();
    assert!(other.0.wait().unwrap().success());
    wait_for_no_old_files(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn disabling_cleanup_preserves_old_files_across_launches() {
    let dir = scratch_dir("preserve");
    let exe = dir.join("demo.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_demo"), &exe).unwrap();
    run(&exe, &["--keep-old", "Dark"]);
    run(&exe, &["--keep-old"]);
    assert_eq!(
        old_files(&dir),
        1,
        "init with cleanup disabled preserves old files"
    );
    run(&exe, &["--keep-old", "Default"]);
    run(&exe, &["--keep-old"]);
    assert_eq!(
        old_files(&dir),
        2,
        "later swaps preserve earlier images too"
    );
    let staging = dir.join("demo.exe.1234-0.new");
    std::fs::write(&staging, b"abandoned").unwrap();
    run(&exe, &["--keep-old"]);
    assert!(
        !staging.exists(),
        "abandoned staging files are still removed"
    );
    run(&exe, &[]);
    assert_eq!(
        old_files(&dir),
        0,
        "enabling cleanup removes preserved images"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

struct HeldDemo(Child);

impl Drop for HeldDemo {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn hold(exe: &Path, icon: Option<&str>) -> HeldDemo {
    let mut command = Command::new(exe);
    command.arg("hold");
    if let Some(icon) = icon {
        command.arg(icon);
    }
    let mut app = HeldDemo(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut ready = String::new();
    BufReader::new(app.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), icon.unwrap_or("Default"));
    app
}

fn old_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("demo.exe.") && name.ends_with(".old") && name != "demo.exe.backup.old"
        })
        .count()
}

fn wait_for_no_old_files(dir: &Path) {
    let started = Instant::now();
    while old_files(dir) != 0 {
        assert!(
            started.elapsed() < Duration::from_secs(25),
            "automatic cleanup did not finish in {dir:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Asserts that the image Windows resolves for this executable is byte-for-byte one
/// of the images inside `source`.
fn assert_resolves_to(exe: &Path, source: &[u8], what: &str) {
    let group = win::read_group(exe).unwrap_or_else(|| panic!("{what}: no icon group in {exe:?}"));
    let id = win::lookup_icon_id(&group, 32);
    let stored = win::read_icon(exe, id)
        .unwrap_or_else(|| panic!("{what}: the group points at RT_ICON {id}, which is missing"));

    assert!(
        images(source).any(|image| image == stored),
        "{what}: the stored image does not match any image in the source .ico"
    );
}

/// Walks the images of an `.ico`, the same layout the crate itself parses.
fn images(ico: &[u8]) -> impl Iterator<Item = &[u8]> {
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    (0..count).map(move |index| {
        let at = 6 + index * 16;
        let size = u32::from_le_bytes(ico[at + 8..at + 12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(ico[at + 12..at + 16].try_into().unwrap()) as usize;
        &ico[offset..offset + size]
    })
}

fn run(exe: &Path, args: &[&str]) -> String {
    let output = Command::new(exe)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("could not run {exe:?}: {e}"));
    assert!(
        output.status.success(),
        "{exe:?} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .expect("scratch directory should be listable")
        .flatten()
        .filter(|entry| entry.file_name() != std::ffi::OsStr::new("demo.exe"))
        .count()
}

fn len(path: &Path) -> u64 {
    std::fs::metadata(path)
        .expect("the executable should exist")
        .len()
}

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("alt-icons-test-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory should be creatable");
    dir
}
