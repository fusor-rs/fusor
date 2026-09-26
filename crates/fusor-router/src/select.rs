//! Choosing among one router's routes, independent of the browser.
use crate::{
    AppUrl, UrlError,
    pattern::{Match, Pattern},
};

/// The route a URL selects. A fallback records the path, so each unknown path
/// is a distinct destination.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Selected {
    pub(crate) index: usize,
    pub(crate) matched: Match,
    pub(crate) fallback_path: Option<String>,
}

/// Select the most specific pattern that matches `url` after `prefix`
/// consumed segments, else the fallback route (a `None` pattern), else nothing.
pub(crate) fn select<'a>(
    routes: impl IntoIterator<Item = Option<&'a Pattern>> + Clone,
    url: &AppUrl,
    prefix: usize,
) -> Result<Option<Selected>, UrlError> {
    let segments = url.route_segments()?;
    let best = routes
        .clone()
        .into_iter()
        .enumerate()
        .filter_map(|(index, pattern)| {
            let pattern = pattern?;
            Some((
                pattern.specificity(),
                index,
                pattern.matches(&segments, prefix)?,
            ))
        })
        .max_by_key(|(specificity, ..)| *specificity);
    if let Some((_, index, matched)) = best {
        return Ok(Some(Selected {
            index,
            matched,
            fallback_path: None,
        }));
    }
    let fallback = routes.into_iter().position(|pattern| pattern.is_none());
    Ok(fallback.map(|index| Selected {
        index,
        matched: Match {
            params: Default::default(),
            consumed: segments.len(),
        },
        fallback_path: Some(url.path.clone()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn routes(paths: &[Option<&str>]) -> Vec<Option<Pattern>> {
        paths
            .iter()
            .map(|path| path.map(|path| Pattern::new(path).unwrap()))
            .collect()
    }
    fn selected(routes: &[Option<Pattern>], url: &str, prefix: usize) -> Option<Selected> {
        select(
            routes.iter().map(Option::as_ref),
            &AppUrl::parse(url).unwrap(),
            prefix,
        )
        .unwrap()
    }
    #[test]
    fn the_most_specific_match_wins_regardless_of_order() {
        let routes = routes(&[Some("/articles/:id"), Some("/articles/new"), None]);
        assert_eq!(selected(&routes, "/articles/new", 0).unwrap().index, 1);
        let article = selected(&routes, "/articles/42", 0).unwrap();
        assert_eq!(
            (article.index, article.matched.params["id"].as_str()),
            (0, "42")
        );
    }
    #[test]
    fn unmatched_paths_reach_the_fallback_as_distinct_destinations() {
        let routes = routes(&[Some("/"), None]);
        let missing = selected(&routes, "/missing", 0).unwrap();
        assert_eq!(
            (missing.index, missing.fallback_path.as_deref()),
            (1, Some("/missing"))
        );
        assert_ne!(missing, selected(&routes, "/other", 0).unwrap());
        assert_eq!(selected(&routes, "/?q=1", 0).unwrap().fallback_path, None);
    }
    #[test]
    fn nested_routers_match_after_their_prefix() {
        let routes = routes(&[Some("settings")]);
        assert_eq!(
            selected(&routes, "/teams/a/settings", 2)
                .unwrap()
                .matched
                .consumed,
            3
        );
        assert_eq!(selected(&routes, "/teams/a/other", 2), None);
    }
}
