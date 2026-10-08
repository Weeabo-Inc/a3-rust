//! Wildcard patterns over VFS paths.

use a3_core::VfsPath;

pub(crate) struct Pattern {
    parts: Vec<Part>,
}

enum Part {
    /// `**`: any number of whole components.
    AnyComponents,
    /// One component, possibly with `*` and `?`.
    Component(Vec<char>),
}

impl Pattern {
    pub(crate) fn new(pattern: &str) -> Self {
        let normalised = VfsPath::new(pattern);
        let parts = normalised
            .components()
            .map(|c| match c {
                "**" => Part::AnyComponents,
                c => Part::Component(c.chars().collect()),
            })
            .collect();
        Self { parts }
    }

    /// The longest leading run of wildcard-free components, excluding the last component:
    /// every match lies below it.
    pub(crate) fn literal_base(&self) -> VfsPath {
        let mut base = VfsPath::root();
        for part in &self.parts[..self.parts.len().saturating_sub(1)] {
            match part {
                Part::Component(chars) if !chars.iter().any(|c| matches!(c, '*' | '?')) => {
                    base = base.join(&chars.iter().collect::<String>());
                }
                _ => break,
            }
        }
        base
    }

    pub(crate) fn matches(&self, path: &VfsPath) -> bool {
        let components: Vec<&str> = path.components().collect();
        match_parts(&self.parts, &components)
    }
}

fn match_parts(parts: &[Part], components: &[&str]) -> bool {
    match parts.split_first() {
        None => components.is_empty(),
        Some((Part::AnyComponents, rest)) => {
            (0..=components.len()).any(|skip| match_parts(rest, &components[skip..]))
        }
        Some((Part::Component(pattern), rest)) => match components.split_first() {
            Some((first, others)) => {
                let text: Vec<char> = first.chars().collect();
                match_component(pattern, &text) && match_parts(rest, others)
            }
            None => false,
        },
    }
}

/// Matches one component with `*` (any run) and `?` (one character), by backtracking.
fn match_component(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        Pattern::new(pattern).matches(&VfsPath::new(path))
    }

    #[test]
    fn star_and_question_mark_stay_within_a_component() {
        assert!(matches(r"a\*.paa", r"a\x_co.paa"));
        assert!(matches(r"a\?.paa", r"a\x.paa"));
        assert!(!matches(r"a\*.paa", r"a\b\x.paa"));
        assert!(!matches(r"a\?.paa", r"a\xy.paa"));
        assert!(matches(r"a\*_co*", r"a\tex_co.paa"));
    }

    #[test]
    fn double_star_matches_any_number_of_components() {
        assert!(matches(r"a\**\x.paa", r"a\x.paa"));
        assert!(matches(r"a\**\x.paa", r"a\b\c\x.paa"));
        assert!(matches(r"**", r"a\b"));
        assert!(!matches(r"a\**\x.paa", r"b\x.paa"));
    }

    #[test]
    fn literal_base_stops_at_the_first_wildcard() {
        assert_eq!(Pattern::new(r"a\b\*\c.p3d").literal_base().as_str(), r"a\b");
        assert_eq!(Pattern::new(r"a\b\c.p3d").literal_base().as_str(), r"a\b");
        assert_eq!(Pattern::new(r"*.p3d").literal_base().as_str(), "");
    }
}
