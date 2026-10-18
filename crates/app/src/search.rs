//! Fuzzy search over the scan (spec §3.10). Matches names, not full paths, so no path index
//! is needed; `/` in the query matches its earlier parts against the ancestors, in order.

use rayon::prelude::*;
use rsdisk_core::tree::{NodeId, Tree, flag};

/// The best `limit` matches for `query`, best first, ties going to the larger node.
pub fn search(tree: &Tree, query: &str, limit: usize) -> Vec<NodeId> {
    let parts: Vec<Vec<u8>> = query
        .split('/')
        .map(|p| p.trim().to_ascii_lowercase().into_bytes())
        .filter(|p| !p.is_empty())
        .collect();
    let Some((name, ancestors)) = parts.split_last() else {
        return Vec::new();
    };
    let mut hits: Vec<(i32, u64, NodeId)> = (1..tree.len() as NodeId)
        .into_par_iter()
        .filter_map(|id| {
            let node = tree.node(id);
            let score = score(node.name.as_bytes(), name)?;
            let bonus = match_ancestors(tree, id, ancestors)?;
            live(tree, id).then_some((score + bonus, node.size, id))
        })
        .collect();
    hits.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    hits.into_iter().take(limit).map(|(_, _, id)| id).collect()
}

/// Whether neither `id` nor anything above it was removed.
fn live(tree: &Tree, id: NodeId) -> bool {
    let mut at = Some(id);
    while let Some(i) = at {
        let n = tree.node(i);
        if n.has(flag::REMOVED) {
            return false;
        }
        at = n.parent();
    }
    true
}

/// Matches the ancestor parts, last part nearest, each against a strictly higher ancestor.
/// Returns their combined score, or `None` when one doesn't match.
fn match_ancestors(tree: &Tree, id: NodeId, parts: &[Vec<u8>]) -> Option<i32> {
    let mut total = 0;
    let mut at = tree.node(id).parent();
    for part in parts.iter().rev() {
        loop {
            let p = at?;
            // The root's name is a full path; only its last component counts.
            let name = tree.node(p).name.rsplit('/').next().unwrap_or_default();
            at = tree.node(p).parent();
            if let Some(s) = score(name.as_bytes(), part) {
                total += s / 2;
                break;
            }
        }
    }
    Some(total)
}

fn is_word_start(text: &[u8], i: usize) -> bool {
    i == 0
        || !text[i - 1].is_ascii_alphanumeric()
        || (text[i].is_ascii_uppercase() && text[i - 1].is_ascii_lowercase())
}

/// A case-insensitive subsequence match of `pattern` (lowercase) in `text`, scored so that
/// word starts and unbroken runs beat scattered letters, and a name that starts with the
/// pattern and has little else beats a long one that merely contains it. `None` when it
/// doesn't match.
pub fn score(text: &[u8], pattern: &[u8]) -> Option<i32> {
    if pattern.is_empty() || pattern.len() > text.len() {
        return None;
    }
    let prefix = text[..pattern.len()].eq_ignore_ascii_case(pattern);
    let base = if prefix { 20 } else { 0 } - ((text.len() - pattern.len()) as i32 / 4).min(10);
    // Greedy from each place the first letter occurs, keeping the best: a later start can be
    // a word start (`Data` in `DerivedData`) where the first one isn't.
    (0..text.len())
        .filter(|&i| text[i].to_ascii_lowercase() == pattern[0])
        .filter_map(|start| greedy(text, pattern, start))
        .max()
        .map(|s| base + s)
}

/// Matches `pattern` in `text` taking each letter as early as possible from `start`.
fn greedy(text: &[u8], pattern: &[u8], start: usize) -> Option<i32> {
    let mut score = 0;
    let mut p = 0;
    let mut last: Option<usize> = None;
    for (i, c) in text.iter().enumerate().skip(start) {
        if p == pattern.len() {
            break;
        }
        if c.to_ascii_lowercase() != pattern[p] {
            continue;
        }
        score += 10;
        if is_word_start(text, i) {
            score += 15;
        }
        match last {
            Some(l) if l + 1 == i => score += 10,
            Some(l) => score -= ((i - l - 1) as i32).min(10),
            None => score -= (i as i32).min(10),
        }
        last = Some(i);
        p += 1;
    }
    (p == pattern.len()).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsdisk_core::kind::Kind;
    use rsdisk_core::tree::{Node, TreeBuilder};

    fn dir(name: &str) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Other, 0)
    }

    /// /Users/tim { Library { Caches { big:500 } }, repos { Caches { small:5 }, cache.db:50 },
    ///             notes.txt:1 }
    fn tree() -> (Tree, [NodeId; 6]) {
        let mut b = TreeBuilder::new(dir("/Users/tim"));
        let lib = b.add_children(0, [dir("Library"), dir("repos"), file("notes.txt", 1)]);
        let repos = lib + 1;
        let lib_caches = b.add_children(lib, [dir("Caches")]);
        let repo_caches = b.add_children(repos, [dir("Caches"), file("cache.db", 50)]);
        let cache_db = repo_caches + 1;
        b.add_children(lib_caches, [file("big", 500)]);
        b.add_children(repo_caches, [file("small", 5)]);
        (
            b.finish(),
            [lib, repos, lib_caches, repo_caches, cache_db, lib + 2],
        )
    }

    fn names(tree: &Tree, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .map(|id| tree.path(*id).display().to_string())
            .collect()
    }

    #[test]
    fn subsequences_match_case_insensitively() {
        assert!(score(b"Library", b"lbr").is_some());
        assert!(score(b"Library", b"LIB".to_ascii_lowercase().as_slice()).is_some());
        assert!(score(b"Library", b"yl").is_none());
    }

    #[test]
    fn word_starts_and_runs_beat_scattered_letters() {
        let run = score(b"node_modules", b"mod").unwrap();
        let scattered = score(b"my_old_directory", b"mod").unwrap();
        assert!(run > scattered, "{run} {scattered}");
        let start = score(b"DerivedData", b"data").unwrap();
        let middle = score(b"metadatafile", b"data").unwrap();
        assert!(start > middle, "{start} {middle}");
    }

    #[test]
    fn a_name_starting_with_the_query_beats_a_long_one_containing_it() {
        let caches = score(b"Caches", b"cach").unwrap();
        let long = score(b"ChaCha20_Poly1305.cpython-314.pyc", b"cach").unwrap();
        assert!(caches > long, "{caches} {long}");
    }

    #[test]
    fn ties_go_to_the_larger_node() {
        let (t, [_, _, lib_caches, repo_caches, ..]) = tree();
        assert_eq!(search(&t, "caches", 10), [lib_caches, repo_caches]);
    }

    #[test]
    fn slashes_match_ancestors() {
        let (t, [_, _, lib_caches, ..]) = tree();
        assert_eq!(search(&t, "lib/cach", 10), [lib_caches]);
        assert_eq!(names(&t, &search(&t, "repos/cache", 10)).len(), 2);
        assert!(search(&t, "nowhere/cach", 10).is_empty());
    }

    #[test]
    fn removed_nodes_never_match() {
        let (mut t, [lib, ..]) = tree();
        t.remove(lib);
        let found = names(&t, &search(&t, "caches", 10));
        assert_eq!(found, ["/Users/tim/repos/Caches"]);
        assert!(search(&t, "big", 10).is_empty());
    }

    #[test]
    fn empty_queries_return_nothing() {
        let (t, _) = tree();
        assert!(search(&t, "", 10).is_empty());
        assert!(search(&t, " / ", 10).is_empty());
    }

    #[test]
    fn the_limit_is_respected() {
        let (t, _) = tree();
        assert_eq!(search(&t, "c", 2).len(), 2);
    }
}
