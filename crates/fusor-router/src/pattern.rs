//! URL matching independent of browser history and HTML rendering.
use crate::UrlError;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Parameter(String),
}

/// How specific one part of a pattern is. Later variants win.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Rank {
    /// A trailing `/*` that delegates the rest of the path.
    Rest,
    Parameter,
    Literal,
    /// The end of an exact pattern.
    End,
}

/// An exact route pattern, optionally ending in `/*` to delegate a remainder.
#[derive(Clone, Debug)]
pub struct Pattern {
    segments: Vec<Segment>,
    delegated: bool,
    specificity: Vec<Rank>,
}

/// Decoded captures and the number of path segments consumed by a match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub params: BTreeMap<String, String>,
    pub consumed: usize,
}

impl Pattern {
    pub fn new(path: &str) -> Result<Self, UrlError> {
        if path.starts_with("//") {
            return Err(UrlError("route patterns have at most one leading slash"));
        }
        let path = path.strip_prefix('/').unwrap_or(path);
        if path.contains(['?', '#', '\\']) || path.contains("//") {
            return Err(UrlError("route patterns contain path segments only"));
        }
        let mut parts: Vec<_> = if path.is_empty() {
            vec![]
        } else {
            path.split('/').collect()
        };
        let delegated = parts.last() == Some(&"*");
        if delegated {
            parts.pop();
        }
        if parts.last() == Some(&"") {
            parts.pop();
        }
        let mut names = BTreeSet::new();
        let segments: Vec<_> = parts
            .into_iter()
            .map(|part| parse_segment(part, &mut names))
            .collect::<Result<_, _>>()?;
        let specificity = segments
            .iter()
            .map(|segment| match segment {
                Segment::Literal(_) => Rank::Literal,
                Segment::Parameter(_) => Rank::Parameter,
            })
            .chain([if delegated { Rank::Rest } else { Rank::End }])
            .collect();
        Ok(Self {
            segments,
            delegated,
            specificity,
        })
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Parameter(name) => Some(name.as_str()),
            Segment::Literal(_) => None,
        })
    }
    pub fn matches(&self, segments: &[String], offset: usize) -> Option<Match> {
        let rest = segments.get(offset..)?;
        if rest.len() < self.segments.len()
            || (!self.delegated && rest.len() != self.segments.len())
        {
            return None;
        }
        let mut params = BTreeMap::new();
        for (segment, value) in self.segments.iter().zip(rest) {
            match segment {
                Segment::Literal(literal) if literal != value => return None,
                Segment::Parameter(name) if !value.is_empty() => {
                    params.insert(name.clone(), value.clone());
                }
                Segment::Parameter(_) => return None,
                Segment::Literal(_) => {}
            }
        }
        Some(Match {
            params,
            consumed: offset + self.segments.len(),
        })
    }
    /// Higher values win over less-specific patterns, independent of declaration order.
    pub(crate) fn specificity(&self) -> &[Rank] {
        &self.specificity
    }
    /// Patterns with identical specificity that can match the same URL are
    /// ambiguous. Equal specificity implies equal length.
    pub fn conflicts(&self, other: &Self) -> bool {
        self.specificity == other.specificity
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|pair| match pair {
                    (Segment::Literal(a), Segment::Literal(b)) => a == b,
                    _ => true,
                })
    }
}

/// Whether two routes of one router could both claim a URL: conflicting
/// patterns, or two fallbacks (`None`).
pub fn ambiguous(a: Option<&Pattern>, b: Option<&Pattern>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.conflicts(b),
        (None, None) => true,
        _ => false,
    }
}

fn parse_segment(part: &str, names: &mut BTreeSet<String>) -> Result<Segment, UrlError> {
    if let Some(name) = part.strip_prefix(':') {
        if !is_identifier(name) || !names.insert(name.to_owned()) {
            return Err(UrlError(
                "route parameter names must be distinct identifiers",
            ));
        }
        return Ok(Segment::Parameter(name.into()));
    }
    if part.contains(['*', ':', '%'])
        || part == "."
        || part == ".."
        || part.chars().any(char::is_control)
    {
        return Err(UrlError(
            "use literal segments, :name captures, and an optional trailing /*",
        ));
    }
    Ok(Segment::Literal(part.into()))
}

fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppUrl;
    fn pattern(path: &str) -> Pattern {
        Pattern::new(path).unwrap()
    }
    fn segments(url: &str) -> Vec<String> {
        AppUrl::parse(url).unwrap().route_segments().unwrap()
    }
    #[test]
    fn nested_prefix_and_decoded_captures() {
        let segments = segments("/teams/a%2Fb/settings?tab=x");
        let parent = pattern("/teams/:team/*").matches(&segments, 0).unwrap();
        assert_eq!(parent.params["team"], "a/b");
        assert!(
            pattern("settings")
                .matches(&segments, parent.consumed)
                .is_some()
        );
        assert!(pattern("/teams/:team").matches(&segments, 0).is_none());
        let base = self::segments("/dashboard");
        let matched = pattern("/dashboard/*").matches(&base, 0).unwrap();
        assert!(pattern("").matches(&base, matched.consumed).is_some());
    }
    #[test]
    fn priority_and_ambiguity() {
        assert!(pattern("/articles/new").specificity() > pattern("/articles/:id").specificity());
        assert!(pattern("/articles/:id").conflicts(&pattern("/articles/:slug")));
        assert!(!pattern("/a/:id").conflicts(&pattern("/b/:id")));
        assert!(ambiguous(None, None));
        assert!(!ambiguous(Some(&pattern("/a")), None));
        for invalid in [
            "/:id/:id", "/a/*/b", "/:123", "/a?query", "/../a", "//", "//a",
        ] {
            assert!(Pattern::new(invalid).is_err(), "{invalid}");
        }
    }
}
