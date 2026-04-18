use std::collections::{HashMap, HashSet};

/// Build a set of word bigram shingles from text.
pub fn shingle(text: &str, n: usize) -> HashSet<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < n {
        return words.iter().map(|w| w.to_string()).collect();
    }
    words.windows(n).map(|w| w.join(" ")).collect()
}

fn hash_with_seed(s: &str, seed: u64) -> u64 {
    let mut h: u64 = seed;
    for byte in s.bytes() {
        h = h.wrapping_mul(1_000_003).wrapping_add(byte as u64);
    }
    h
}

/// Compute a MinHash signature over a shingle set.
pub fn minhash_signature(shingles: &HashSet<String>, num_hashes: usize) -> Vec<u64> {
    (0..num_hashes)
        .map(|seed| {
            shingles
                .iter()
                .map(|s| hash_with_seed(s, seed as u64))
                .min()
                .unwrap_or(u64::MAX)
        })
        .collect()
}

/// Estimate Jaccard similarity from two MinHash signatures.
pub fn jaccard_estimate(sig_a: &[u64], sig_b: &[u64]) -> f64 {
    let matches = sig_a.iter().zip(sig_b.iter()).filter(|(a, b)| a == b).count();
    matches as f64 / sig_a.len() as f64
}

pub struct Index<'a> {
    corpus: Vec<&'a str>,
    signatures: Vec<Vec<u64>>,
    band_buckets: Vec<HashMap<u64, Vec<usize>>>,
    num_hashes: usize,
    bands: usize,
}

impl<'a> Index<'a> {
    pub fn build(corpus: &[&'a str], bands: usize) -> Self {
        let num_hashes = bands * 4;

        let signatures: Vec<Vec<u64>> = corpus
            .iter()
            .map(|text| {
                let shingles = shingle(text, 2);
                minhash_signature(&shingles, num_hashes)
            })
            .collect();

        let mut band_buckets: Vec<HashMap<u64, Vec<usize>>> =
            (0..bands).map(|_| HashMap::new()).collect();

        let band_size = num_hashes / bands;
        for (doc_id, sig) in signatures.iter().enumerate() {
            for (band_idx, chunk) in sig.chunks(band_size).enumerate() {
                let band_hash: u64 = chunk
                    .iter()
                    .enumerate()
                    .fold(0u64, |acc, (i, &v)| acc.wrapping_add(v.wrapping_mul(i as u64 + 7)));
                band_buckets[band_idx]
                    .entry(band_hash)
                    .or_insert_with(Vec::new)
                    .push(doc_id);
            }
        }

        Self { corpus: corpus.to_vec(), signatures, band_buckets, num_hashes, bands }
    }

    /// Find top-k most similar documents to `query`.
    /// Returns `(similarity_score, corpus_index)` pairs sorted descending.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(f64, usize)> {
        let q_shingles = shingle(query, 2);
        let q_sig = minhash_signature(&q_shingles, self.num_hashes);

        let band_size = self.num_hashes / self.bands;
        let mut candidates: HashSet<usize> = HashSet::new();

        for (band_idx, chunk) in q_sig.chunks(band_size).enumerate() {
            let band_hash: u64 = chunk
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &v)| acc.wrapping_add(v.wrapping_mul(i as u64 + 7)));
            if let Some(bucket) = self.band_buckets[band_idx].get(&band_hash) {
                candidates.extend(bucket.iter());
            }
        }

        let mut scored: Vec<(f64, usize)> = candidates
            .iter()
            .map(|&doc_id| (jaccard_estimate(&q_sig, &self.signatures[doc_id]), doc_id))
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    pub fn corpus(&self) -> &[&'a str] {
        &self.corpus
    }
}

/// Owned, mutable LSH index for incremental insertion.
/// Satisfies the `SearchIndex` interface required by the read path.
pub struct SearchIndex {
    docs: Vec<(usize, String)>,
    signatures: Vec<(usize, Vec<u64>)>,
    band_buckets: Vec<HashMap<u64, Vec<usize>>>,
    num_hashes: usize,
    bands: usize,
}

impl SearchIndex {
    pub fn new(bands: usize) -> Self {
        let num_hashes = bands * 4;
        Self {
            docs: Vec::new(),
            signatures: Vec::new(),
            band_buckets: (0..bands).map(|_| HashMap::new()).collect(),
            num_hashes,
            bands,
        }
    }

    pub fn insert(&mut self, drawer_id: usize, content: &str) {
        let shingles = shingle(content, 2);
        let sig = minhash_signature(&shingles, self.num_hashes);

        let slot = self.signatures.len();
        let band_size = self.num_hashes / self.bands;
        for (band_idx, chunk) in sig.chunks(band_size).enumerate() {
            let band_hash: u64 = chunk
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &v)| acc.wrapping_add(v.wrapping_mul(i as u64 + 7)));
            self.band_buckets[band_idx]
                .entry(band_hash)
                .or_insert_with(Vec::new)
                .push(slot);
        }

        self.docs.push((drawer_id, content.to_owned()));
        self.signatures.push((drawer_id, sig));
    }

    /// Returns `(drawer_id, similarity_score)` pairs sorted by descending similarity.
    pub fn query(&self, text: &str, top_k: usize) -> Vec<(usize, f64)> {
        let q_shingles = shingle(text, 2);
        let q_sig = minhash_signature(&q_shingles, self.num_hashes);

        let band_size = self.num_hashes / self.bands;
        let mut candidates: HashSet<usize> = HashSet::new();

        for (band_idx, chunk) in q_sig.chunks(band_size).enumerate() {
            let band_hash: u64 = chunk
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &v)| acc.wrapping_add(v.wrapping_mul(i as u64 + 7)));
            if let Some(bucket) = self.band_buckets[band_idx].get(&band_hash) {
                candidates.extend(bucket.iter());
            }
        }

        let mut scored: Vec<(usize, f64)> = candidates
            .iter()
            .map(|&slot| {
                let (drawer_id, ref sig) = self.signatures[slot];
                (drawer_id, jaccard_estimate(&q_sig, sig))
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_finds_similar() {
        // Use a corpus entry that shares enough shingles with the query to guarantee a band hit.
        let corpus = vec![
            "optimal selection token budget",
            "import os sys path",
            "pass return None",
        ];
        let index = Index::build(&corpus, 4);
        let results = index.search("optimal selection token budget", 2);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_shingles() {
        let s = shingle("fn select optimal items", 2);
        assert!(s.contains("fn select"));
        assert!(s.contains("select optimal"));
        assert!(s.contains("optimal items"));
    }

    #[test]
    fn test_empty_corpus() {
        let corpus: Vec<&str> = vec![];
        let index = Index::build(&corpus, 4);
        assert!(index.search("anything", 3).is_empty());
    }

    #[test]
    fn test_search_index_insert_and_query() {
        let mut idx = SearchIndex::new(4);
        idx.insert(10, "optimal selection token budget knapsack");
        idx.insert(20, "import os sys path");
        idx.insert(30, "pass return None");

        let results = idx.query("optimal selection token budget", 3);
        assert!(!results.is_empty());
        // drawer_id 10 should be the top hit
        assert_eq!(results[0].0, 10);
        // results sorted descending by similarity
        for w in results.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
    }

    #[test]
    fn test_search_index_empty() {
        let idx = SearchIndex::new(4);
        assert!(idx.query("anything", 3).is_empty());
    }
}
