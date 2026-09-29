//! Read-only Mission Work hierarchy. Dependencies explain inputs, never ownership.
use crate::{
    task_view::ScopeStatus,
    tasks::{Action, Snapshot, Task},
};
use std::collections::{BTreeMap, BTreeSet};

const LABEL_BYTES: usize = 512;
const DETAIL_BYTES: usize = 8192;

/// Plan identity is its original canonical receipt revision, not its current title.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum NodeId {
    Plan(u64),
    Manual,
    Task(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub depth: usize,
    pub task: Option<u64>,
    pub label: String,
    pub status: String,
    pub detail: String,
    /// Number of tasks in this subtree, including this row if it is a task.
    pub task_count: usize,
    pub expandable: bool,
    pub expanded: bool,
    /// A direct task match, distinct from an ancestor retained for navigation.
    pub matches_filter: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tree {
    pub rows: Vec<Row>,
    /// Canonical tasks, independent of filtering or collapsed groups.
    pub total_tasks: usize,
    /// Direct task matches, independent of retained ancestors or collapsed groups.
    pub matched_tasks: usize,
}

/// Projects a validated snapshot without recording view state or authorizing work.
/// Repair ancestry is indexed once, then walked iteratively in canonical ID order.
/// A search reveals matching paths even if an ancestor is locally collapsed.
pub fn project(
    snapshot: &Snapshot,
    scope: &ScopeStatus,
    query: &str,
    collapsed: &BTreeSet<NodeId>,
) -> Tree {
    project_with(snapshot, scope, query, collapsed, None)
}

/// As `project`, and with `expanded` given, a completed plan group starts
/// collapsed while another group still has open work, unless listed there.
pub fn project_with(
    snapshot: &Snapshot,
    scope: &ScopeStatus,
    query: &str,
    collapsed: &BTreeSet<NodeId>,
    expanded: Option<&BTreeSet<NodeId>>,
) -> Tree {
    let tasks: BTreeMap<_, _> = snapshot.tasks.iter().map(|task| (task.id, task)).collect();
    let mut membership = BTreeMap::new();
    let mut plans = BTreeMap::new();
    for receipt in &snapshot.receipts {
        if let Action::Plan { plan } = &receipt.request.action {
            let id = NodeId::Plan(receipt.revision);
            plans.entry(id).or_insert(plan);
            for offset in 0..plan.tasks.len() {
                if let Some(task) = receipt.task.checked_add(offset as u64) {
                    if tasks.contains_key(&task) {
                        membership.entry(task).or_insert(id);
                    }
                }
            }
        }
    }

    let query = query.trim().to_lowercase();
    let filtering = !query.is_empty();
    let mut rows = BTreeMap::new();
    let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut counts: BTreeMap<NodeId, usize> = BTreeMap::new();
    let mut included = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let mut matched_tasks = 0;

    for task in tasks.values() {
        let id = NodeId::Task(task.id);
        // Canonical repair parents precede their children. Invalid external
        // fixtures fall back to a group rather than forming a cycle or vanishing.
        let parent = task
            .repair_of
            .filter(|parent| *parent < task.id && tasks.contains_key(parent))
            .map(NodeId::Task)
            .unwrap_or_else(|| membership.get(&task.id).copied().unwrap_or(NodeId::Manual));
        if !matches!(parent, NodeId::Task(_)) {
            roots.insert(parent);
        }
        children.entry(parent).or_default().push(id);
        counts.insert(id, 1);
        let status = snapshot.task_status_label(task);
        let detail = task_detail(snapshot, scope, task, membership.get(&task.id).copied());
        let matches_filter = matches(task, &status, &detail, &query);
        if matches_filter {
            included.insert(id);
            matched_tasks += 1;
        }
        rows.insert(
            id,
            Row {
                id,
                parent: Some(parent),
                depth: 0,
                task: Some(task.id),
                label: bounded(&task.title, LABEL_BYTES),
                status: bounded(&status, LABEL_BYTES),
                detail,
                task_count: 1,
                expandable: false,
                expanded: false,
                matches_filter,
            },
        );
    }

    // Reverse task order propagates each subtree exactly once. No recursive
    // traversal or repeated ancestor search is needed for counts and filtering.
    for task in tasks.keys().rev() {
        let id = NodeId::Task(*task);
        let parent = rows[&id].parent.expect("task rows have a parent");
        let count = counts[&id];
        *counts.entry(parent).or_default() += count;
        if included.contains(&id) {
            included.insert(parent);
        }
    }

    let mut automatic = BTreeSet::new();
    if let Some(expanded) = expanded {
        use crate::tasks::TaskStatus;
        // Canonical order: a repair's parent is resolved before the repair.
        let mut root_of: BTreeMap<u64, NodeId> = BTreeMap::new();
        let mut complete: BTreeMap<NodeId, bool> = BTreeMap::new();
        let mut open = BTreeSet::new();
        for task in tasks.values() {
            let root = match rows[&NodeId::Task(task.id)].parent {
                Some(NodeId::Task(parent)) => root_of[&parent],
                Some(root) => root,
                None => NodeId::Manual,
            };
            root_of.insert(task.id, root);
            if task.repair_of.is_none() {
                let done = task.status == TaskStatus::Accepted
                    || snapshot.resolution_for_family(task.id).is_some();
                *complete.entry(root).or_insert(true) &= done;
            }
            if matches!(
                task.status,
                TaskStatus::Proposed
                    | TaskStatus::Approved
                    | TaskStatus::Running
                    | TaskStatus::ReviewReady
                    | TaskStatus::NeedsHumanReview
            ) {
                open.insert(root);
            }
        }
        for (root, done) in complete {
            if done
                && matches!(root, NodeId::Plan(_))
                && !expanded.contains(&root)
                && open.iter().any(|other| *other != root)
            {
                automatic.insert(root);
            }
        }
    }

    for id in &roots {
        let task_count = counts[id];
        let (label, detail) = match id {
            NodeId::Plan(revision) => {
                let plan = plans[id];
                (
                    bounded(
                        &format!("Plan r{revision} · {}", crate::planner::goal(&plan.prompt)),
                        LABEL_BYTES,
                    ),
                    bounded(
                        &format!(
                            "{task_count} tasks including repairs · {} original steps · planner {}",
                            plan.tasks.len(),
                            plan.planner
                        ),
                        DETAIL_BYTES,
                    ),
                )
            }
            _ => (
                "Manual tasks".into(),
                format!("{task_count} tasks including repairs · no original Plan receipt"),
            ),
        };
        rows.insert(
            *id,
            Row {
                id: *id,
                parent: None,
                depth: 0,
                task: None,
                label,
                status: format!("{task_count} tasks"),
                detail,
                task_count,
                expandable: false,
                expanded: false,
                matches_filter: false,
            },
        );
    }

    let mut tree = Tree {
        rows: Vec::with_capacity(rows.len()),
        total_tasks: tasks.len(),
        matched_tasks,
    };
    let mut pending: Vec<_> = roots.into_iter().rev().map(|id| (id, 0)).collect();
    while let Some((id, depth)) = pending.pop() {
        if !included.contains(&id) {
            continue;
        }
        let Some(mut row) = rows.remove(&id) else {
            continue;
        };
        row.depth = depth;
        row.task_count = counts[&id];
        let descendants = children.get(&id).map(Vec::as_slice).unwrap_or_default();
        row.expandable = descendants.iter().any(|id| included.contains(id));
        row.expanded =
            row.expandable && (filtering || !(collapsed.contains(&id) || automatic.contains(&id)));
        if row.expanded {
            pending.extend(descendants.iter().rev().map(|child| (*child, depth + 1)));
        }
        tree.rows.push(row);
    }
    tree
}

fn task_detail(
    snapshot: &Snapshot,
    scope: &ScopeStatus,
    task: &Task,
    membership: Option<NodeId>,
) -> String {
    let mut parts = Vec::new();
    // Include every declared edge, even after acceptance. Resolved repair inputs
    // remain explanatory edges and cannot reparent or duplicate their consumer.
    if !task.dependencies.is_empty() {
        parts.push(format!(
            "Dependencies: {}",
            task.dependencies
                .iter()
                .map(|id| match snapshot.dependency_source(*id) {
                    Ok(source) if source.id != *id => {
                        format!("#{id} via repair #{}", source.id)
                    }
                    _ => format!("#{id}"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    parts.push(scope.readiness(snapshot, task));
    if let Some(parent) = task.repair_of {
        parts.push(format!("Repair of #{parent}"));
        if let Some(NodeId::Plan(revision)) = membership {
            parts.push(format!("Architect Plan r{revision}"));
        }
    }
    bounded(&parts.join(" · "), DETAIL_BYTES)
}

fn matches(task: &Task, status: &str, detail: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if let Some(id) = query.strip_prefix('#') {
        return id.parse::<u64>().ok() == Some(task.id);
    }
    task.title.to_lowercase().contains(query)
        || task.model.to_lowercase().contains(query)
        || status.to_lowercase().contains(query)
        || format!("{:?}", task.status).to_lowercase().contains(query)
        || detail.to_lowercase().contains(query)
}

fn bounded(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.into();
    }
    let mut end = bytes.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
