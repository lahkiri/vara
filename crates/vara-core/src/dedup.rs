//! Note deduplication — ported from harness v2 (`vara_harness_v2.mjs`).
//!
//! During v1 experiments the harness produced 6-7 copies of the same note,
//! which poisoned both arms. The fix (Jaccard >= 0.72 over token sets) became
//! a permanent rule of the entity.

use std::collections::HashSet;

/// Dedup threshold frozen from harness v2 smoke runs.
pub const DEDUP_THRESHOLD: f64 = 0.72;

/// Tokenize for comparison: lowercase, alphanumeric runs (Arabic included),
/// drop tokens shorter than 3 chars.
pub fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 2)
        .map(|t| t.to_string())
        .collect()
}

pub fn jaccard(a: &str, b: &str) -> f64 {
    let ta = tokens(a);
    let tb = tokens(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    if union == 0 {
        return 0.0;
    }
    inter as f64 / union as f64
}

pub fn is_duplicate(a: &str, b: &str) -> bool {
    jaccard(a, b) >= DEDUP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_duplicate() {
        let t = "GPT-4o mini costs $0.15 per 1M input tokens";
        assert!(is_duplicate(t, t));
    }

    #[test]
    fn near_copy_is_duplicate() {
        let a = "Llama 3 70B scored 82.1 on MMLU benchmark";
        let b = "Llama 3 70B scored 82.1 on the MMLU benchmark";
        assert!(is_duplicate(a, b));
    }

    #[test]
    fn different_is_not_duplicate() {
        assert!(!is_duplicate(
            "Mixtral 8x7B uses sparse mixture of experts routing",
            "The Intel 8086 processor launched in 1978 with 29000 transistors"
        ));
    }

    #[test]
    fn arabic_supported() {
        let a = "تكلفة نموذج Llama 3 في الاستدلال أقل من GPT-4";
        let b = "تكلفة نموذج Llama 3 في الاستدلال أقل من GPT-4";
        assert!(is_duplicate(a, b));
        assert!(!is_duplicate(
            a,
            "الطبقة الكمومية الأولى أطلقتها الصين عام 2016"
        ));
    }

    #[test]
    fn empty_vs_content_is_zero() {
        assert_eq!(jaccard("", "hello world foo"), 0.0);
        assert_eq!(jaccard("", ""), 1.0);
    }
}
