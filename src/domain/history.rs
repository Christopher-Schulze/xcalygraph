//! Git history analyses (`docs/spec.md` section 6, git items): churn and co-change.

use std::collections::BTreeMap;

/// Commits touching more files are bulk changes and do not count for co-change.
pub const BULK_COMMIT: usize = 50;
pub const MIN_CO_CHANGES: u32 = 3;
pub const MIN_JACCARD: f32 = 0.3;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    /// Commits touching each file.
    pub churn: BTreeMap<String, u32>,
    /// File pairs (a < b) changed together, with their shared commit count.
    pub co_change: Vec<(String, String, u32)>,
}

/// Churn and co-change pairs (at least 3 shared commits, Jaccard at least 0.3) of commit file
/// lists.
pub fn analyze(commits: &[Vec<String>]) -> History {
    let mut churn: BTreeMap<String, u32> = BTreeMap::new();
    let mut together: BTreeMap<(String, String), u32> = BTreeMap::new();
    for files in commits {
        let mut files: Vec<&String> = files.iter().collect();
        files.sort_unstable();
        files.dedup();
        for file in &files {
            *churn.entry((*file).clone()).or_default() += 1;
        }
        if files.len() > BULK_COMMIT {
            continue;
        }
        for (index, left) in files.iter().enumerate() {
            for right in &files[index + 1..] {
                *together
                    .entry(((*left).clone(), (*right).clone()))
                    .or_default() += 1;
            }
        }
    }
    let co_change = together
        .into_iter()
        .filter(|((left, right), count)| {
            let union = churn[left] + churn[right] - count;
            *count >= MIN_CO_CHANGES && *count as f32 / union.max(1) as f32 >= MIN_JACCARD
        })
        .map(|((left, right), count)| (left, right, count))
        .collect();
    History { churn, co_change }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(files: &[&str]) -> Vec<String> {
        files.iter().map(|file| (*file).to_owned()).collect()
    }

    #[test]
    fn churn_and_co_change_follow_thresholds() {
        let mut commits = vec![
            commit(&["a.go", "b.go"]),
            commit(&["a.go", "b.go"]),
            commit(&["a.go", "b.go", "c.go"]),
            commit(&["a.go"]),
            commit(&["c.go", "d.go"]),
        ];
        let bulk: Vec<String> = (0..=BULK_COMMIT)
            .map(|index| format!("bulk{index}.go"))
            .chain(["c.go".to_owned(), "d.go".to_owned()])
            .collect();
        commits.push(bulk);
        let history = analyze(&commits);
        assert_eq!(history.churn.get("a.go"), Some(&4));
        assert_eq!(history.churn.get("c.go"), Some(&3));
        // a/b: 3 shared of 4 + 3 - 3 = 4 -> 0.75; c/d: 1 counted (bulk ignored) -> below 3.
        assert_eq!(
            history.co_change,
            [("a.go".to_owned(), "b.go".to_owned(), 3)]
        );
    }
}
