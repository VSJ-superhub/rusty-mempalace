use std::collections::HashMap;

/// Shannon entropy of a string. Higher = more information-dense.
/// Used at write time to decide whether compression is worthwhile.
pub fn score(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut freq: HashMap<char, usize> = HashMap::new();
    for c in s.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }
    let n = s.chars().count() as f64;
    freq.values()
        .map(|&count| {
            let p = count as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Normalize entropy to [0.0, 1.0] relative to maximum possible for this string.
pub fn score_normalized(s: &str) -> f64 {
    if s.len() < 2 {
        return 0.0;
    }
    let unique: std::collections::HashSet<char> = s.chars().collect();
    let max_entropy = (unique.len() as f64).log2();
    if max_entropy == 0.0 {
        return 0.0;
    }
    score(s) / max_entropy
}

/// Sort fragments by entropy descending. Used to prioritise compression candidates.
pub fn rank<'a>(fragments: &'a [&'a str]) -> Vec<(&'a str, f64)> {
    let mut scored: Vec<(&str, f64)> = fragments.iter().map(|&f| (f, score(f))).collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dense_beats_boilerplate() {
        let s1 = score("pass");
        let s2 = score("fn select_optimal(items: &[Fragment], budget: usize) -> Vec<usize>");
        assert!(s2 > s1);
    }

    #[test]
    fn test_empty() {
        assert_eq!(score(""), 0.0);
        assert_eq!(score_normalized(""), 0.0);
    }

    #[test]
    fn test_rank_orders_correctly() {
        let frags = vec!["pass", "import os", "fn kkt_optimal(budget: usize, items: &[Item]) -> Vec<usize>"];
        let ranked = rank(&frags);
        assert!(ranked[0].1 > ranked[ranked.len() - 1].1);
    }
}
