use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::Instant;

pub struct CachedFileCompletions {
    pub cached_at: Instant,
    pub files: Arc<Vec<String>>,
}

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

    let filename = candidate.rsplit('/').next().unwrap_or(candidate);
    let filename_lower = filename.to_lowercase();

    if filename_lower == q {
        return Some(100_000);
    }

    if filename_lower.starts_with(&q) {
        let filename_len = i32::try_from(filename.len()).ok()?;
        return Some(50_000 - filename_len);
    }

    if let Some(idx) = filename_lower.find(&q) {
        let idx = i32::try_from(idx).ok()?;
        let filename_len = i32::try_from(filename.len()).ok()?;
        return Some(20_000 - idx * 10 - filename_len);
    }

    let path_parts: Vec<&str> = candidate.split('/').collect();
    if path_parts.len() > 1 {
        for (i, part) in path_parts.iter().enumerate() {
            if i == path_parts.len() - 1 {
                continue;
            }
            let part_lower = part.to_lowercase();
            if let Some(idx) = part_lower.find(&q) {
                let idx = i32::try_from(idx).ok()?;
                let path_len = i32::try_from(candidate.len()).ok()?;
                return Some(5_000 - idx * 10 - path_len);
            }
        }
    }

    if let Some(idx) = cand.find(&q) {
        let idx = i32::try_from(idx).ok()?;
        let path_len = i32::try_from(candidate.len()).ok()?;
        return Some(2_000 - idx * 10 - path_len);
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
    fn ranks_filename_matches_highest() {
        let paths = vec![
            "src/pages/SessionPage.tsx".to_string(),
            "src/pages/WorkbenchPage.tsx".to_string(),
            "README.md".to_string(),
        ];

        let out = filter_and_rank_paths(&paths, "sess", 10);
        assert_eq!(
            out.first().map(|s| s.as_str()),
            Some("src/pages/SessionPage.tsx")
        );
    }

    #[test]
    fn no_fuzzy_subsequence_matching() {
        let paths = vec![
            "scripts/supabase_local_start.sh".to_string(),
            "src/earth_model.ts".to_string(),
        ];

        let out = filter_and_rank_paths(&paths, "earth", 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out.first().map(|s| s.as_str()), Some("src/earth_model.ts"));
    }

    #[test]
    fn returns_deterministic_order_for_empty_query() {
        let paths = vec![
            "b.txt".to_string(),
            "a.txt".to_string(),
            "c.txt".to_string(),
        ];
        let out = filter_and_rank_paths(&paths, "", 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filters_out_non_matches() {
        let paths = vec!["src/main.rs".to_string(), "Cargo.toml".to_string()];
        let out = filter_and_rank_paths(&paths, "does-not-exist", 10);
        assert!(out.is_empty());
    }
}
