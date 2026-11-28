use crate::issue::{self, Issue, Reference};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct Root {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct Names {
    pub(crate) prefix: String,
    numbers: BTreeMap<String, u64>,
    roots: BTreeMap<u64, Vec<Root>>,
    reserved: Vec<u64>,
}

pub(crate) enum Resolved {
    Key(String),
    Missing(String),
    Ambiguous(String, Vec<Root>),
}

impl Names {
    pub(crate) fn new(prefix: &str) -> Names {
        Names {
            prefix: prefix.to_string(),
            numbers: BTreeMap::new(),
            roots: BTreeMap::new(),
            reserved: Vec::new(),
        }
    }

    pub(crate) fn add(&mut self, issue: &Issue, path: PathBuf) {
        if let Some(number) = issue.number {
            self.numbers.insert(issue.id.clone(), number);
            let roots = self.roots.entry(number).or_default();
            if roots.iter().all(|root| root.key != issue.id) {
                roots.push(Root {
                    key: issue.id.clone(),
                    title: issue.title.clone(),
                    path,
                });
            }
        }
    }

    pub(crate) fn name(&self, key: &str) -> String {
        match self.numbers.get(issue::root_key(key)) {
            Some(number) => issue::handle(&self.prefix, *number, key),
            None => key.to_string(),
        }
    }

    pub(crate) fn reserve(&mut self, number: u64) {
        self.reserved.push(number);
    }

    pub(crate) fn next_number(&self) -> u64 {
        self.roots
            .keys()
            .chain(self.reserved.iter())
            .max()
            .map_or(1, |last| last + 1)
    }

    pub(crate) fn taken(&self, number: u64) -> bool {
        self.roots.contains_key(&number) || self.reserved.contains(&number)
    }

    pub(crate) fn duplicates(&self) -> impl Iterator<Item = (u64, &[Root])> {
        self.roots
            .iter()
            .filter(|(_, roots)| roots.len() > 1)
            .map(|(number, roots)| (*number, roots.as_slice()))
    }

    pub(crate) fn resolve(&self, reference: &Reference) -> Resolved {
        match reference {
            Reference::Key(key) => Resolved::Key(key.clone()),
            Reference::Number { number, suffix } => {
                let shown = format!("{}{number}{suffix}", self.prefix);
                match self.roots.get(number).map(Vec::as_slice) {
                    None | Some([]) => Resolved::Missing(shown),
                    Some([root]) => Resolved::Key(format!("{}{suffix}", root.key)),
                    Some(many) => Resolved::Ambiguous(shown, many.to_vec()),
                }
            }
        }
    }
}
