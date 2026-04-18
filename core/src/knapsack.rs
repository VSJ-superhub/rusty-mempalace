pub struct Fragment<'a> {
    pub text: &'a str,
    pub tokens: usize,
    pub entropy: f64,
}

/// Select the subset of fragments with maximum total entropy within `budget` tokens.
pub fn select_optimal<'a>(fragments: &'a [Fragment<'a>], budget: usize) -> Vec<&'a Fragment<'a>> {
    let n = fragments.len();
    if n == 0 || budget == 0 {
        return vec![];
    }

    let scaled: Vec<usize> = fragments
        .iter()
        .map(|f| (f.entropy * 1000.0) as usize)
        .collect();

    let mut dp: Vec<Vec<usize>> = vec![vec![0; budget + 1]; n + 1];

    for i in 1..=n {
        let item = &fragments[i - 1];
        let weight = item.tokens;
        let value = scaled[i - 1];

        for w in 0..=budget {
            dp[i][w] = dp[i - 1][w];
            if weight <= w {
                let with_item = dp[i - 1][w - weight] + value;
                if with_item > dp[i][w] {
                    dp[i][w] = with_item;
                }
            }
        }
    }

    let mut selected: Vec<&Fragment> = Vec::new();
    let mut remaining = budget;

    for i in (1..=n).rev() {
        if dp[i][remaining] != dp[i - 1][remaining] {
            selected.push(&fragments[i - 1]);
            remaining -= fragments[i - 1].tokens;
        }
    }

    selected.reverse();
    selected
}

/// Greedy approximation — sort by entropy-per-token, take greedily.
pub fn select_greedy<'a>(fragments: &'a [Fragment<'a>], budget: usize) -> Vec<&'a Fragment<'a>> {
    let mut indexed: Vec<(usize, f64)> = fragments
        .iter()
        .enumerate()
        .map(|(i, f)| (i, if f.tokens > 0 { f.entropy / f.tokens as f64 } else { 0.0 }))
        .collect();

    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut result = Vec::new();
    let mut used = 0;

    for (idx, _) in indexed {
        let frag = &fragments[idx];
        if used + frag.tokens <= budget {
            result.push(frag);
            used += frag.tokens;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fragments<'a>(data: &[(&'a str, usize, f64)]) -> Vec<Fragment<'a>> {
        data.iter()
            .map(|&(text, tokens, entropy)| Fragment { text, tokens, entropy })
            .collect()
    }

    #[test]
    fn test_dp_vs_greedy() {
        let data = vec![
            ("import os", 2, 0.5),
            ("fn complex(x: &HashMap<u32,f64>)", 8, 4.1),
            ("let mut scores = Vec::new();", 5, 2.8),
            ("/// docs", 2, 1.2),
        ];
        let frags = make_fragments(&data);
        let dp_result = select_optimal(&frags, 10);
        let dp_entropy: f64 = dp_result.iter().map(|f| f.entropy).sum();
        let greedy_result = select_greedy(&frags, 10);
        let greedy_entropy: f64 = greedy_result.iter().map(|f| f.entropy).sum();
        assert!(dp_entropy >= greedy_entropy - 0.001);
    }

    #[test]
    fn test_budget_respected() {
        let data = vec![
            ("fn a()", 5, 2.0),
            ("fn b(x: usize)", 6, 3.0),
            ("let x = 1;", 3, 1.0),
        ];
        let frags = make_fragments(&data);
        let selected = select_optimal(&frags, 8);
        let total: usize = selected.iter().map(|f| f.tokens).sum();
        assert!(total <= 8);
    }

    #[test]
    fn test_empty() {
        let frags: Vec<Fragment> = vec![];
        assert!(select_optimal(&frags, 100).is_empty());
        assert!(select_greedy(&frags, 100).is_empty());
    }
}
