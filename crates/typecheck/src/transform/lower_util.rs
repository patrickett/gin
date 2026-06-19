//! Fuzzy name-matching utilities used by type-flaw reporting.
//!
//! Provides Levenshtein-based edit distance and `closest_name` lookup
//! for suggesting "did you mean?" corrections when a symbol name is
//! not found.

/// Computes the Levenshtein distance between two strings.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    // Bails early on large length differences.
    let max_dist: usize = 2;
    if a_len.abs_diff(b_len) > max_dist {
        return max_dist + 1;
    }

    // Use two-row technique for O(min(a_len, b_len)) memory.
    let (shorter, longer) = if a_len < b_len { (a, b) } else { (b, a) };
    let s_len = shorter.chars().count();
    let l_chars: Vec<char> = longer.chars().collect();

    let mut prev: Vec<usize> = (0..=s_len).collect();
    let mut curr: Vec<usize> = vec![0; s_len + 1];

    for (i, lc) in l_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, sc) in shorter.chars().enumerate() {
            let cost = if lc == &sc { 0 } else { 1 };
            curr[j + 1] =
                std::cmp::min(std::cmp::min(curr[j] + 1, prev[j + 1] + 1), prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[s_len]
}

/// Returns the closest matching name from `candidates` within edit distance ≤ 2,
/// or `None` if no candidate is close enough.
pub(crate) fn closest_name<'a>(
    target: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        if candidate == target {
            continue;
        }
        let dist = edit_distance(target, candidate);
        if dist <= 2 {
            match best {
                Some((prev_dist, _)) if dist < prev_dist => {
                    best = Some((dist, candidate));
                }
                None => {
                    best = Some((dist, candidate));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, name)| name.to_string())
}
