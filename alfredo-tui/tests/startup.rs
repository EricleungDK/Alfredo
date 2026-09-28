//! Zero-ceremony startup and selector usability; no terminal or model required.
use alfredo_tui::{
    missions::Discovery,
    model::Session,
    selection::{self, acknowledge, SelectorView},
    selection_command::{MissionChoice, WorkspaceChoice},
    tasks::TaskStore,
};
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-startup-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mission(choice: &alfredo_tui::selection_command::Choice) -> &MissionChoice {
    &choice.mission
}

#[tokio::test]
async fn automatic_start_resolves_root_from_subdirectory_and_creates_then_resumes_default() {
    let f = Fixture::new();
    let state = f.0.join("state");
    let repo = acknowledge(&f.0.join("repo"), &state, true).await.unwrap();
    let nested = repo.join("src/deep");
    fs::create_dir_all(&nested).unwrap();

    let choice = selection::automatic(&nested, &state)
        .await
        .unwrap()
        .expect("inside a repository");
    assert_eq!(
        choice.workspace,
        WorkspaceChoice::Existing { path: repo.clone() }
    );
    assert_eq!(
        mission(&choice),
        &MissionChoice::StartNew {
            name: "default".into()
        }
    );
    // Preflight only: nothing is created before the launcher admits the choice.
    assert!(!state.exists());

    TaskStore::new(&state, &repo, "default")
        .unwrap()
        .select_mission(true)
        .unwrap();
    let choice = selection::automatic(&repo, &state).await.unwrap().unwrap();
    assert_eq!(
        mission(&choice),
        &MissionChoice::Resume {
            name: "default".into()
        }
    );
}

#[tokio::test]
async fn automatic_start_outside_a_repository_defers_to_the_selector() {
    let f = Fixture::new();
    let plain = f.0.join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(selection::automatic(&plain, &f.0.join("state"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn automatic_start_reports_corrupt_mission_state_instead_of_replacing_it() {
    let f = Fixture::new();
    let state = f.0.join("state");
    let repo = acknowledge(&f.0.join("repo"), &state, true).await.unwrap();
    let store = TaskStore::new(&state, &repo, "default").unwrap();
    store.select_mission(true).unwrap();
    let manifest = fs::read_dir(state.join("rust-tasks-v1"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("mission.json"))
        .find(|path| path.exists())
        .unwrap();
    fs::write(&manifest, b"{corrupt").unwrap();
    assert!(selection::automatic(&repo, &state).await.is_err());
    assert_eq!(fs::read(&manifest).unwrap(), b"{corrupt");
}

#[tokio::test]
async fn mission_entry_opens_existing_or_creates_missing_names() {
    let f = Fixture::new();
    let state = f.0.join("state");
    let repo = acknowledge(&f.0.join("repo"), &state, true).await.unwrap();
    let workspace = WorkspaceChoice::Existing { path: repo.clone() };
    let created = selection::open_or_create(&state, workspace.clone(), "demo1").unwrap();
    assert_eq!(
        created.mission,
        MissionChoice::StartNew {
            name: "demo1".into()
        }
    );
    assert!(!state.exists());
    TaskStore::new(&state, &repo, "demo1")
        .unwrap()
        .select_mission(true)
        .unwrap();
    let resumed = selection::open_or_create(&state, workspace, "demo1").unwrap();
    assert_eq!(
        resumed.mission,
        MissionChoice::Resume {
            name: "demo1".into()
        }
    );
}

fn render(width: u16, height: u16, view: &SelectorView) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| selection::render_selector(frame, view))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn find(buffer: &Buffer, needle: &str) -> Option<(u16, u16)> {
    rows(buffer).iter().enumerate().find_map(|(y, row)| {
        row.find(needle)
            .map(|byte| (row[..byte].chars().count() as u16, y as u16))
    })
}

#[test]
fn selector_notice_is_adjacent_to_the_input_at_every_size() {
    let workspace = WorkspaceChoice::Existing {
        path: PathBuf::from("/tmp/repository"),
    };
    let saved = Discovery::default();
    let input = Session::new("selection".into());
    for accepted in [None, Some(&workspace)] {
        let view = SelectorView {
            starting: Path::new("/tmp/start"),
            accepted,
            create: false,
            start_new: false,
            notice: "NOTICE_MARK something went wrong",
            input: &input,
            placeholder: "default",
            saved: &saved,
            saved_index: 0,
            loading: false,
            pending: false,
        };
        for (width, height) in [(80, 24), (40, 12), (120, 60), (200, 90)] {
            let buffer = render(width, height, &view);
            let (x, notice_row) = find(&buffer, "NOTICE_MARK")
                .unwrap_or_else(|| panic!("notice missing at {width}x{height}"));
            let input_top = rows(&buffer)
                .iter()
                .position(|row| row.starts_with('┌'))
                .expect("input box") as u16;
            assert_eq!(
                notice_row + 1,
                input_top,
                "notice not adjacent at {width}x{height}:\n{}",
                rows(&buffer).join("\n")
            );
            let style = buffer[(x, notice_row)].style();
            assert!(
                matches!(style.fg, Some(Color::Yellow) | Some(Color::Red)),
                "notice not highlighted: {style:?}"
            );
        }
    }
}

#[test]
fn empty_selector_input_shows_a_placeholder_that_typing_replaces() {
    let workspace = WorkspaceChoice::Existing {
        path: PathBuf::from("/tmp/repository"),
    };
    let saved = Discovery::default();
    let mut input = Session::new("selection".into());
    fn view<'a>(
        workspace: &'a WorkspaceChoice,
        saved: &'a Discovery,
        input: &'a Session,
    ) -> SelectorView<'a> {
        SelectorView {
            starting: Path::new("/tmp/start"),
            accepted: Some(workspace),
            create: false,
            start_new: false,
            notice: "",
            input,
            placeholder: "default",
            saved,
            saved_index: 0,
            loading: false,
            pending: false,
        }
    }
    let empty = render(80, 24, &view(&workspace, &saved, &input));
    let (x, y) = find(&empty, "default").expect("placeholder shown");
    assert_eq!(empty[(x, y)].style().fg, Some(Color::DarkGray));
    assert!(rows(&empty)
        .iter()
        .any(|row| row.contains("Enter open or create")));
    input.insert("demo1");
    let typed = rows(&render(80, 24, &view(&workspace, &saved, &input))).join("\n");
    assert!(typed.contains("demo1"));
    assert!(!typed.contains("defaultdemo1"));
    assert_eq!(selection::entry(&input, "default"), "demo1");
    assert_eq!(
        selection::entry(&Session::new("selection".into()), "default"),
        "default"
    );
}
