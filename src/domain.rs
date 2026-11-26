use crate::issue::{Issue, Status};

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
        if issue.closing_summary.is_some() {
            out.push((
                "active issue has a closing summary",
                "remove the Closing Summary section",
            ));
        }
    }
    out
}

#[derive(Debug)]
pub(crate) enum Ineligible {
    State(String),
    Deferred,
    Blocked(Vec<String>),
    Reference(String),
}

impl Ineligible {
    pub(crate) fn message(self, id: &str) -> String {
        match self {
            Self::State(s) => s,
            Self::Deferred => format!("issue {id} is deferred"),
            Self::Blocked(ids) => format!("issue {id} blocked by {}", ids.join(", ")),
            Self::Reference(s) => format!("issue {id} has unresolved {s}"),
        }
    }
}

pub(crate) fn eligibility(
    issue: &Issue,
    today: &str,
    force: bool,
    mut resolve: impl FnMut(&str) -> Result<Issue, String>,
) -> Result<Vec<String>, Ineligible> {
    transition(issue, Action::Claim).map_err(Ineligible::State)?;
    let mut deferred = issue.defer_until.as_deref().is_some_and(|d| d > today);
    let mut parent = issue.parent.clone();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = parent {
        if !seen.insert(id.clone()) {
            return Err(Ineligible::Reference(format!("ancestor cycle at {id}")));
        }
        let ancestor =
            resolve(&id).map_err(|err| Ineligible::Reference(format!("ancestor {id}: {err}")))?;
        deferred |=
            ancestor.status.active() && ancestor.defer_until.as_deref().is_some_and(|d| d > today);
        parent = ancestor.parent;
    }
    let mut blockers = Vec::new();
    for id in &issue.depends_on {
        let blocker =
            resolve(id).map_err(|err| Ineligible::Reference(format!("blocker {id}: {err}")))?;
        if blocker.status.active() {
            blockers.push(id.clone());
        }
    }
    if deferred {
        return Err(Ineligible::Deferred);
    }
    if !force && !blockers.is_empty() {
        return Err(Ineligible::Blocked(blockers));
    }
    Ok(blockers)
}
