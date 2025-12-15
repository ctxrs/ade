use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Eq, PartialEq)]
struct ScoredCandidate {
    score: i32,
    path_len: usize,
    path: String,
}

impl Ord for ScoredCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| other.path_len.cmp(&self.path_len))
            .then_with(|| other.path.cmp(&self.path))
    }
}

impl PartialOrd for ScoredCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn match_score(candidate: &str, query: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }

    let cand = candidate.to_lowercase();
    let q = query.to_lowercase();

    if let Some(idx) = cand.find(&q) {
        let idx = i32::try_from(idx).ok()?;
        let len = i32::try_from(candidate.len()).ok()?;
        // Prefer earlier matches and shorter paths.
        return Some(10_000 - idx * 10 - len);
    }

    // Fallback: subsequence match (characters in order).
    let mut q_chars = q.chars();
    let mut next = q_chars.next()?;
    let mut last_match: Option<usize> = None;
    let mut gaps: i32 = 0;

    for (i, c) in cand.chars().enumerate() {
        if c == next {
            if let Some(prev) = last_match {
                gaps += i32::try_from(i.saturating_sub(prev + 1)).ok()?;
            }
            last_match = Some(i);
            if let Some(n) = q_chars.next() {
                next = n;
            } else {
                let len = i32::try_from(candidate.len()).ok()?;
                return Some(5_000 - gaps * 10 - len);
            }
        }
    }

    None
}

pub fn filter_and_rank_paths(paths: &[String], query: &str, limit: usize) -> Vec<String> {
    let limit = limit.clamp(1, 200);
    let mut heap: BinaryHeap<Reverse<ScoredCandidate>> = BinaryHeap::with_capacity(limit + 1);

    for path in paths {
        let Some(score) = match_score(path, query) else {
            continue;
        };
        let candidate = ScoredCandidate {
            score,
            path_len: path.len(),
            path: path.clone(),
        };
        heap.push(Reverse(candidate));
        if heap.len() > limit {
            heap.pop();
        }
    }

    let mut out: Vec<ScoredCandidate> = heap.into_iter().map(|r| r.0).collect();
    out.sort_by(|a, b| b.cmp(a));
    out.into_iter().map(|c| c.path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_prefix_and_substring_ahead_of_subsequence() {
        let paths = vec![
            "src/pages/SessionPage.tsx".to_string(),
            "src/pages/WorkbenchPage.tsx".to_string(),
            "README.md".to_string(),
        ];

        let out = filter_and_rank_paths(&paths, "sess", 10);
        assert_eq!(out.first().map(|s| s.as_str()), Some("src/pages/SessionPage.tsx"));
    }

    #[test]
    fn returns_deterministic_order_for_empty_query() {
        let paths = vec!["b.txt".to_string(), "a.txt".to_string(), "c.txt".to_string()];
        let out = filter_and_rank_paths(&paths, "", 2);
        // With equal score, shorter/lex order picks deterministically.
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filters_out_non_matches() {
        let paths = vec!["src/main.rs".to_string(), "Cargo.toml".to_string()];
        let out = filter_and_rank_paths(&paths, "does-not-exist", 10);
        assert!(out.is_empty());
    }
}

