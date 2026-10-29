use alfredo_tui::at_lookup::{AtLookup, Found};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{fs, path::Path, process::Command, time::Duration};
use tokio::runtime::Runtime;

fn repo(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("alfredo-lookup-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let status = Command::new("git")
        .args(["init", "-q", "--template="])
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(dir.join("alpha.txt"), "a\n").unwrap();
    fs::write(dir.join("beta.txt"), "b\n").unwrap();
    dir
}

/// Every answer the lookup gives until it has nothing running or queued.
fn answers(lookup: &mut AtLookup, runtime: &Runtime) -> Vec<Found> {
    let mut found = Vec::new();
    for _ in 0..2000 {
        found.extend(lookup.poll(runtime));
        if !lookup.busy() {
            return found;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("lookup never finished");
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn an_answer_for_an_older_draft_is_dropped() {
    let (dir, runtime) = (repo("stale"), runtime());
    let mut lookup = AtLookup::default();
    assert!(lookup.for_draft(&runtime, "@al", 3, &dir).is_none());
    assert!(lookup.for_draft(&runtime, "@be", 3, &dir).is_none());
    assert!(lookup.busy());
    assert_eq!(
        answers(&mut lookup, &runtime),
        [Found {
            draft: "@be".into(),
            paths: vec!["beta.txt".into()]
        }]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_lookup_answers_nothing() {
    let (dir, runtime) = (repo("cancel"), runtime());
    let mut lookup = AtLookup::default();
    assert!(lookup.for_draft(&runtime, "@al", 3, &dir).is_none());
    lookup.cancel();
    assert!(answers(&mut lookup, &runtime).is_empty());
    // A draft that is no longer an @ word cancels the lookup of the one before.
    assert!(lookup.for_draft(&runtime, "@al", 3, &dir).is_none());
    assert!(lookup.for_draft(&runtime, "@al x", 5, &dir).is_none());
    assert!(answers(&mut lookup, &runtime).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn slash_typeahead_is_immediate_and_a_cursor_inside_the_draft_looks_nothing_up() {
    let (dir, runtime) = (repo("slash"), runtime());
    let mut lookup = AtLookup::default();
    assert!(lookup.for_draft(&runtime, "@al", 3, &dir).is_none());
    let slash = lookup.for_draft(&runtime, "/pla", 4, &dir).unwrap();
    assert!(slash.choices.iter().any(|c| c.name == "/plan"));
    assert!(answers(&mut lookup, &runtime).is_empty());
    assert!(lookup.for_draft(&runtime, "@al", 1, &dir).is_none());
    assert!(!lookup.busy());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_listing_is_kept_for_one_word_and_dropped_with_it() {
    let (dir, runtime) = (repo("cache"), runtime());
    let mut lookup = AtLookup::default();
    lookup.for_draft(&runtime, "@", 1, &dir);
    assert_eq!(answers(&mut lookup, &runtime)[0].paths.len(), 2);
    fs::write(dir.join("gamma.txt"), "g\n").unwrap();
    // Same word: the cached listing does not know the new file yet.
    lookup.for_draft(&runtime, "@ga", 3, &dir);
    assert!(answers(&mut lookup, &runtime)[0].paths.is_empty());
    // Leaving the word drops the cache; the next word lists again.
    lookup.for_draft(&runtime, "x", 1, &dir);
    lookup.for_draft(&runtime, "@ga", 3, &dir);
    assert_eq!(
        answers(&mut lookup, &runtime)[0].paths,
        ["gamma.txt".to_string()]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_cached_listing_is_for_one_workspace() {
    let (first, second, runtime) = (repo("root-a"), repo("root-b"), runtime());
    fs::write(second.join("other.txt"), "o\n").unwrap();
    let mut lookup = AtLookup::default();
    lookup.for_draft(&runtime, "@ot", 3, &first);
    assert!(answers(&mut lookup, &runtime)[0].paths.is_empty());
    lookup.for_draft(&runtime, "@ot", 3, Path::new(&second));
    assert_eq!(
        answers(&mut lookup, &runtime)[0].paths,
        ["other.txt".to_string()]
    );
    let _ = fs::remove_dir_all(&first);
    let _ = fs::remove_dir_all(&second);
}

#[test]
fn only_plain_characters_and_backspace_keep_the_cached_listing() {
    let (dir, runtime) = (repo("keys"), runtime());
    let key = |code, modifiers| KeyEvent::new(code, modifiers);
    let mut lookup = AtLookup::default();
    for ended_by in [
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Tab, KeyModifiers::NONE),
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
        key(KeyCode::Char('x'), KeyModifiers::ALT),
        key(KeyCode::Esc, KeyModifiers::NONE),
    ] {
        lookup.for_draft(&runtime, "@", 1, &dir);
        answers(&mut lookup, &runtime);
        fs::write(dir.join("gamma.txt"), "g\n").unwrap();
        for kept in [
            key(KeyCode::Char('g'), KeyModifiers::NONE),
            key(KeyCode::Char('G'), KeyModifiers::SHIFT),
            key(KeyCode::Backspace, KeyModifiers::NONE),
        ] {
            lookup.on_key(&kept);
        }
        lookup.for_draft(&runtime, "@ga", 3, &dir);
        assert!(
            answers(&mut lookup, &runtime)[0].paths.is_empty(),
            "{ended_by:?}"
        );
        lookup.on_key(&ended_by);
        lookup.for_draft(&runtime, "@ga", 3, &dir);
        assert_eq!(
            answers(&mut lookup, &runtime)[0].paths,
            ["gamma.txt".to_string()],
            "{ended_by:?}"
        );
        fs::remove_file(dir.join("gamma.txt")).unwrap();
        lookup.cancel();
    }
    let _ = fs::remove_dir_all(&dir);
}
