use crate::issue::{Issue, Status};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResolveError {
    Missing {
        id: String,
    },
    Unusable {
        id: String,
        path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Missing { id } => write!(f, "missing issue {id}"),
            ResolveError::Unusable { id, path, reason } => {
                write!(f, "issue {id} is unusable: {reason} ({})", path.display())
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Unresolved {
    Missing {
        role: &'static str,
        id: String,
    },
    Unusable {
        role: &'static str,
        id: String,
        path: PathBuf,
    },
    Subtask {
        path: PathBuf,
    },
    Own(PathBuf),
}

impl fmt::Display for Unresolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unresolved::Missing { role, id } => write!(f, "{role} {id} (missing)"),
            Unresolved::Unusable { role, id, path } => {
                write!(f, "{role} {id} (unusable {})", path.display())
            }
            Unresolved::Subtask { path } => write!(f, "subtask {} (unusable)", path.display()),
            Unresolved::Own(path) => write!(f, "{}", path.display()),
        }
    }
}

fn unresolved_from_error(role: &'static str, error: ResolveError) -> Unresolved {
    match error {
        ResolveError::Missing { id } => Unresolved::Missing { role, id },
        ResolveError::Unusable { id, path, .. } => Unresolved::Unusable { role, id, path },
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Approve,
    Revise,
    Claim,
    Unclaim,
    Review,
    Rework,
    Close,
    Reopen,
}

impl Action {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Action::Approve => "approve",
            Action::Revise => "revise",
            Action::Claim => "claim",
            Action::Unclaim => "unclaim",
            Action::Review => "review",
            Action::Rework => "rework",
            Action::Close => "close",
            Action::Reopen => "reopen",
        }
    }
}

pub(crate) fn transition(issue: &Issue, action: Action) -> Result<Status, String> {
    use Action::*;
    use Status::{Claimed, Closed, Open, Proposed};
    let next = match (action, issue.status) {
        (Approve, Proposed) => Open,
        (Revise, Open) => Proposed,
        (Claim, Open) => Claimed,
        (Unclaim, Claimed) => Open,
        (Review, Claimed) => Status::Review,
        (Rework, Status::Review) => Claimed,
        (Close, Proposed | Open | Claimed | Status::Review) => Closed,
        (Reopen, Closed) => Open,
        (Claim, Proposed) => return Err(format!("issue {} awaits approval", issue.id)),
        (Claim, Claimed) => {
            return Err(format!(
                "issue {} already claimed{}",
                issue.id,
                issue
                    .assignee
                    .as_deref()
                    .map(|name| format!(" by {name}"))
                    .unwrap_or_default()
            ));
        }
        (Claim, Status::Review) => return Err(format!("issue {} is in review", issue.id)),
        (Close, Closed) => return Err(format!("issue {} is already closed", issue.id)),
        (Reopen, _) => return Err(format!("issue {} is already open", issue.id)),
        _ => {
            return Err(format!(
                "issue {} is {} and cannot {}",
                issue.id,
                issue.status.as_str(),
                action.as_str()
            ));
        }
    };
    Ok(next)
}

pub(crate) fn field_problems(issue: &Issue) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    match issue.status {
        Status::Proposed | Status::Open => {
            if issue.assignee.is_some() {
                out.push((
                    "proposed or open issue has an assignee",
                    "remove the assignee value",
                ));
            }
            if issue.claim_id.is_some() {
                out.push((
                    "proposed or open issue has a claim_id",
                    "remove the claim_id value",
                ));
            }
        }
        Status::Claimed if issue.claim_id.is_none() => {
            out.push(("claimed issue has no claim_id", "add a claim_id value"));
        }
        _ => {}
    }
    if issue.status.active() {
        if issue.closed_at.is_some() {
            out.push(("active issue has closed_at", "remove the closed_at value"));
        }
        if issue.release.is_some() {
            out.push(("active issue has release", "remove the release value"));
        }
        if issue.closing_summary.is_some() {
            out.push((
                "active issue has a closing summary",
                "remove the Closing Summary section",
            ));
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Eligibility {
    pub status: Option<String>,
    pub deferred_until: Option<String>,
    pub deferred_by: Option<String>,
    pub unresolved: Vec<Unresolved>,
    pub blockers: Vec<String>,
    pub subtasks: Vec<String>,
}

impl Eligibility {
    pub(crate) fn ready(&self) -> bool {
        self.status.is_none()
            && self.deferred_until.is_none()
            && self.unresolved.is_empty()
            && self.blockers.is_empty()
            && self.subtasks.is_empty()
    }

    pub(crate) fn reason_text(&self, check_hint: bool) -> String {
        let mut parts = Vec::new();
        if let Some(status) = &self.status {
            parts.push(format!("status {status}"));
        }
        if let Some(until) = &self.deferred_until {
            let mut text = format!("deferred until {until}");
            if let Some(by) = &self.deferred_by {
                text.push_str(&format!(" by ancestor {by}"));
            }
            parts.push(text);
        }
        for reference in &self.unresolved {
            parts.push(format!("unresolved {reference}"));
        }
        if !self.blockers.is_empty() {
            parts.push(format!("blocked by {}", self.blockers.join(", ")));
        }
        if !self.subtasks.is_empty() {
            parts.push(format!("waiting for subtasks {}", self.subtasks.join(", ")));
        }
        let mut result = parts.join("; ");
        if check_hint && !self.unresolved.is_empty() {
            result.push_str(" (run `jig check`)");
        }
        result
    }

    pub(crate) fn bypassed(&self) -> Vec<String> {
        self.blockers
            .iter()
            .cloned()
            .chain(self.subtasks.iter().cloned())
            .collect()
    }
}

pub(crate) fn eligibility(
    issue: &Issue,
    today: &str,
    mut resolve: impl FnMut(&str) -> Result<Issue, ResolveError>,
    descendants: impl Fn(&str) -> Vec<(String, Status)>,
    unresolved_descendants: impl Fn(&str) -> Vec<PathBuf>,
) -> Eligibility {
    let mut out = Eligibility::default();
    if issue.status != Status::Open {
        out.status = Some(issue.status.as_str().to_string());
    }
    if issue.defer_until.as_deref().is_some_and(|d| d > today) {
        out.deferred_until = issue.defer_until.clone();
    }
    let mut parent = issue.parent.clone();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = parent {
        if !seen.insert(id.clone()) {
            out.unresolved.push(Unresolved::Missing {
                role: "ancestor",
                id,
            });
            break;
        }
        let ancestor = match resolve(&id) {
            Ok(ancestor) => ancestor,
            Err(error) => {
                out.unresolved
                    .push(unresolved_from_error("ancestor", error));
                break;
            }
        };
        if out.deferred_until.is_none()
            && ancestor.status.active()
            && ancestor.defer_until.as_deref().is_some_and(|d| d > today)
        {
            out.deferred_until = ancestor.defer_until.clone();
            out.deferred_by = Some(ancestor.id.clone());
        }
        parent = ancestor.parent;
    }
    for id in &issue.depends_on {
        match resolve(id) {
            Ok(blocker) if blocker.status.active() => out.blockers.push(id.clone()),
            Ok(_) => {}
            Err(error) => out.unresolved.push(unresolved_from_error("blocker", error)),
        }
    }
    out.subtasks = descendants(&issue.id)
        .into_iter()
        .filter(|(_, status)| status.active())
        .map(|(id, _)| id)
        .collect();
    out.unresolved.extend(
        unresolved_descendants(&issue.id)
            .into_iter()
            .map(|path| Unresolved::Subtask { path }),
    );
    out.unresolved.sort_by_key(ToString::to_string);
    out.unresolved.dedup();
    out
}
