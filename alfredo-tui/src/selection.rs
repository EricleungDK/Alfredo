//! Startup selection. Starting location never implicitly grants workspace identity.
use crate::{
    model::Session,
    selection_command::{Choice, MissionChoice, WorkspaceChoice},
    tasks::TaskStore,
    worker::git,
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Paragraph, Wrap},
};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{runtime::Runtime, task::JoinHandle};
type Result<T> = std::result::Result<T, String>;

// Resolve existing ancestors without requiring the future runtime directory to exist.
// A dangling symlink or unresolvable parent is an error, not evidence of separation.
pub(crate) fn future_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = ancestor
                    .canonicalize()
                    .map_err(|e| format!("Cannot resolve runtime state: {e}"))?;
                if !resolved.is_dir() {
                    return Err("Runtime state ancestor must be a directory".into());
                }
                for name in suffix.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or("Cannot resolve runtime state parent")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("Cannot resolve runtime state parent")?;
            }
            Err(error) => return Err(format!("Cannot inspect runtime state: {error}")),
        }
    }
}

/// Resolve an immutable picker choice without creating repository or mission files.
pub async fn prepare_workspace(path: &Path, state: &Path, create: bool) -> Result<WorkspaceChoice> {
    if path
        .to_str()
        .is_none_or(|s| s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control))
    {
        return Err("Repository path must be UTF-8, at most 4096 bytes, without controls".into());
    }
    if create {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Choose a new directory name")?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent
            .canonicalize()
            .map_err(|e| format!("Parent directory unavailable: {e}"))?;
        if !parent.is_dir() {
            return Err("Parent must be a directory".into());
        }
        let target = parent.join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(_) => {
                return Err(
                    "New repository target already exists; choose another path or open it".into(),
                )
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot inspect new repository path: {error}")),
        }
        if git(&parent, &["rev-parse", "--show-toplevel"])
            .await
            .is_ok()
        {
            return Err("Create the repository outside an existing repository".into());
        }
        if future_path(state)?.starts_with(&target) {
            return Err("Runtime state must be outside the new repository".into());
        }
        return Ok(WorkspaceChoice::Create {
            parent,
            name: name.into(),
        });
    }
    let target = path
        .canonicalize()
        .map_err(|e| format!("Repository unavailable: {e}"))?;
    if !target.is_dir() {
        return Err("Choose a repository directory".into());
    }
    let root = git(&target, &["rev-parse", "--show-toplevel"]).await?;
    let root = Path::new(root.trim())
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if root != target {
        return Err(format!(
            "Choose the exact repository root: {}",
            root.display()
        ));
    }
    TaskStore::new(state, &target, "selection-validation")?;
    Ok(WorkspaceChoice::Existing { path: target })
}

/// Effect helper for an already admitted selection. Picker code uses prepare_workspace.
pub async fn acknowledge(path: &Path, state: &Path, create: bool) -> Result<PathBuf> {
    let choice = prepare_workspace(path, state, create).await?;
    prepare_effects(choice).await
}

/// Revalidate the saved path without silently resolving to a different target.
pub async fn acknowledge_prepared(choice: &WorkspaceChoice, state: &Path) -> Result<PathBuf> {
    let checked = prepare_workspace(
        &choice.path(),
        state,
        matches!(choice, WorkspaceChoice::Create { .. }),
    )
    .await?;
    if &checked != choice {
        return Err(
            "Workspace identity changed after selection; inspect and choose it again".into(),
        );
    }
    prepare_effects(checked).await
}

async fn prepare_effects(choice: WorkspaceChoice) -> Result<PathBuf> {
    let target = choice.path();
    if matches!(choice, WorkspaceChoice::Create { .. }) {
        std::fs::create_dir(&target)
            .map_err(|e| format!("Cannot create repository directory: {e}"))?;
        let initialized: Result<()> = async {
            git(&target, &["init", "--template=", "--initial-branch=main"]).await?;
            let tree = git(&target, &["mktree"]).await?;
            let commit = git(
                &target,
                &[
                    "-c",
                    "user.name=Alfredo",
                    "-c",
                    "user.email=alfredo@localhost",
                    "-c",
                    "commit.gpgSign=false",
                    "commit-tree",
                    tree.trim(),
                    "-m",
                    "Initialize empty Alfredo workspace",
                ],
            )
            .await?;
            git(
                &target,
                &[
                    "update-ref",
                    "refs/heads/main",
                    commit.trim(),
                    &"0".repeat(40),
                ],
            )
            .await?;
            Ok(())
        }
        .await;
        if let Err(error) = initialized {
            return Err(format!("Repository creation incomplete at {}: {error}. Directory retained; inspect it before retrying", target.display()));
        }
    }
    Ok(target)
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

pub fn choose(
    runtime: &Runtime,
    starting: &Path,
    explicit: Option<PathBuf>,
    mission: Option<String>,
    start_new: bool,
    state: &Path,
) -> Result<Option<Choice>> {
    if let (Some(path), Some(name)) = (&explicit, &mission) {
        let workspace = runtime.block_on(prepare_workspace(path, state, false))?;
        return Ok(Some(mission_choice(state, workspace, name, start_new)?));
    }
    let mut terminal = ratatui::init();
    let _guard = Guard;
    execute!(io::stdout(), EnableBracketedPaste).map_err(|e| e.to_string())?;
    choose_in_terminal(
        runtime,
        starting,
        explicit,
        mission,
        start_new,
        state,
        &mut terminal,
    )
}

pub fn choose_in_terminal(
    runtime: &Runtime,
    starting: &Path,
    explicit: Option<PathBuf>,
    mission: Option<String>,
    mut start_new: bool,
    state: &Path,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<Option<Choice>> {
    let mut accepted = match explicit {
        Some(path) => Some(runtime.block_on(prepare_workspace(&path, state, false))?),
        None => None,
    };
    if let (Some(path), Some(name)) = (&accepted, &mission) {
        return Ok(Some(mission_choice(state, path.clone(), name, start_new)?));
    }
    let mut input = Session::new("selection".into());
    input.insert(if accepted.is_some() {
        mission.as_deref().unwrap_or("default")
    } else {
        starting.to_str().unwrap_or("")
    });
    let mut create = false;
    let mut notice = String::new();
    let mut pending: Option<JoinHandle<Result<WorkspaceChoice>>> = None;
    let discovery_job = |path: Option<PathBuf>| {
        let state = state.to_owned();
        runtime.spawn_blocking(move || match path {
            Some(path) => crate::missions::discover(&state, &path),
            None => crate::missions::discover_workspaces(&state),
        })
    };
    let mut discovery = Some(discovery_job(accepted.as_ref().map(WorkspaceChoice::path)));
    let mut saved = crate::missions::Discovery::default();
    let mut saved_index: usize = 0;
    loop {
        if pending.as_ref().is_some_and(|job| job.is_finished()) {
            match runtime
                .block_on(pending.take().unwrap())
                .map_err(|e| e.to_string())?
            {
                Ok(path) => {
                    discovery = if matches!(path, WorkspaceChoice::Existing { .. }) {
                        Some(discovery_job(Some(path.path())))
                    } else {
                        start_new = true;
                        None
                    };
                    saved = Default::default();
                    saved_index = 0;
                    accepted = Some(path);
                    input = Session::new("selection".into());
                    input.insert(mission.as_deref().unwrap_or("default"));
                    notice = if create { "New repository path validated. Nothing created yet; choose a mission to continue." } else { "Repository validated." }.into();
                }
                Err(error) => notice = error,
            }
        }
        if discovery.as_ref().is_some_and(|job| job.is_finished()) {
            match runtime
                .block_on(discovery.take().unwrap())
                .map_err(|e| e.to_string())?
            {
                Ok(found) => saved = found,
                Err(error) => {
                    notice =
                        format!("Saved work discovery unavailable: {error}. Enter a name manually.")
                }
            }
        }
        terminal.draw(|frame| {
            let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(2), Constraint::Length(3), Constraint::Length(3)]).split(frame.area());
            frame.render_widget(Paragraph::new(" ALFREDO · Open your work").style(Style::default().fg(Color::Cyan)), rows[0]);
            let description = if let Some(path) = &accepted {
                format!("Workspace: {}\nMission selection required\n\n{}\n{}", path.path().display(), if start_new { "Start New Mission · existing names are refused" } else { "Resume Mission · saved identity required" }, notice)
            } else {
                format!("Starting location: {}\nWorkspace selection required · no workspace or mission bound\n\n{}\n{}", starting.display(), if create { "Create a new repository with an empty initial commit at an unused path." } else { "Open an existing repository by its exact root path." }, notice)
            };
            let description = if accepted.is_some() {
                let names = saved.names.iter().skip(saved_index.saturating_sub(2)).take(6).cloned().collect::<Vec<_>>().join(" · ");
                format!("{description}\n\nSaved mission names: {}\n{}", if discovery.is_some() { "loading…" } else if names.is_empty() { "none found" } else { &names }, if matches!(accepted, Some(WorkspaceChoice::Create { .. })) { "Choose a new mission name. Enter saves the request before creating anything." } else if start_new { "Choose an unused name; F2 returns to Resume." } else if discovery.is_some() { "Reading saved mission names…" } else if saved.limited || saved.skipped > 0 { "Some records omitted; manual names remain available." } else if saved.names.is_empty() { "Type a mission name to begin." } else { "Tab fills a saved name; Enter opens it." })
            } else if !create {
                let paths = saved.workspaces.iter().skip(saved_index.saturating_sub(2)).take(4).map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                format!("{description}\n\nSaved repositories:\n{}\n{}", if discovery.is_some() { "loading…" } else if paths.is_empty() { "none found" } else { &paths }, if discovery.is_some() { "Reading saved repositories…" } else if saved.limited || saved.skipped > 0 { "Some records omitted; type a path manually." } else if saved.workspaces.is_empty() { "Type a repository path to begin." } else { "Tab fills a saved path; Enter validates it." })
            } else { description };
            let description: String = description.chars().map(|c| if c.is_control() && c != '\n' { '�' } else { c }).collect();
            frame.render_widget(Paragraph::new(description).wrap(Wrap {trim: false}), rows[1]);
            let title = if accepted.is_some() { if start_new { "New mission name" } else { "Mission to resume" } } else if create { "New repository path" } else { "Repository path" };
            frame.render_widget(Paragraph::new(input.draft_view(rows[2].width.saturating_sub(2) as usize)).block(Block::bordered().title(title)), rows[2]);
            frame.render_widget(Paragraph::new(if pending.is_some() { "Validating selection · waiting for result" } else if accepted.is_some() { if matches!(accepted, Some(WorkspaceChoice::Create { .. })) { "Enter create repository and mission · Ctrl+U clear · Esc cancel" } else if start_new { "Enter start new · F2 resume · Ctrl+U clear · Esc cancel" } else { "Tab names · Enter resume · F2 start new · Esc cancel" } } else { "Tab saved paths · Enter validate · F2 open/create · Ctrl+U clear · Esc cancel" }).wrap(Wrap {trim: true}), rows[3]);
        }).map_err(|e| e.to_string())?;
        if !event::poll(Duration::from_millis(40)).map_err(|e| e.to_string())? {
            continue;
        }
        let event = event::read().map_err(|e| e.to_string())?;
        if matches!(&event, Event::Key(key) if key.kind != KeyEventKind::Release && (key.code == KeyCode::Esc || (key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL))))
        {
            if let Some(job) = pending.take() {
                job.abort();
            }
            if let Some(job) = discovery.take() {
                job.abort();
            }
            return Ok(None);
        }
        if pending.is_some() {
            continue;
        } // Read-only validation may always be cancelled.
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None)
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input = Session::new("selection".into());
                }
                KeyCode::F(2) if accepted.is_none() => {
                    create = !create;
                    notice.clear();
                }
                KeyCode::F(2) if matches!(accepted, Some(WorkspaceChoice::Existing { .. })) => {
                    start_new = !start_new;
                    notice.clear();
                }
                KeyCode::Tab if accepted.is_some() && !start_new && !saved.names.is_empty() => {
                    input = Session::new("selection".into());
                    input.insert(&saved.names[saved_index % saved.names.len()]);
                    saved_index = (saved_index + 1) % saved.names.len();
                }
                KeyCode::Tab if accepted.is_none() && !create && !saved.workspaces.is_empty() => {
                    input = Session::new("selection".into());
                    input.insert(
                        saved.workspaces[saved_index % saved.workspaces.len()]
                            .to_str()
                            .unwrap_or(""),
                    );
                    saved_index = (saved_index + 1) % saved.workspaces.len();
                }
                KeyCode::Enter => {
                    if let Some(path) = &accepted {
                        match mission_choice(state, path.clone(), &input.draft, start_new) {
                            Ok(choice) => return Ok(Some(choice)),
                            Err(error) => notice = error,
                        }
                    } else {
                        let path = PathBuf::from(&input.draft);
                        let state = state.to_owned();
                        pending =
                            Some(runtime.spawn(async move {
                                prepare_workspace(&path, &state, create).await
                            }));
                        notice.clear();
                    }
                }
                KeyCode::Left => input.left(),
                KeyCode::Right => input.right(),
                KeyCode::Home => input.home(),
                KeyCode::End => input.end(),
                KeyCode::Backspace => input.backspace(),
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    input.insert(&c.to_string())
                }
                _ => {}
            },
            Event::Paste(text) => input.insert(&text),
            _ => {}
        }
    }
}

fn mission_choice(
    state: &Path,
    workspace: WorkspaceChoice,
    name: &str,
    start_new: bool,
) -> Result<Choice> {
    let mission = if start_new {
        MissionChoice::StartNew { name: name.into() }
    } else {
        MissionChoice::Resume { name: name.into() }
    };
    let choice = Choice { workspace, mission };
    choice.validate()?;
    if let WorkspaceChoice::Existing { path } = &choice.workspace {
        TaskStore::new(state, path, name)?.check_mission_selection(start_new)?;
    }
    Ok(choice)
}
