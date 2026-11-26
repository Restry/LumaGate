//! Public request names are a projection, never a replacement for persisted route IDs.
use std::collections::{HashMap, HashSet};

pub struct RouteName<'a> {
    pub id: &'a str,
    pub model: &'a str,
    pub prefixed: &'a str,
    pub prefer_prefix: bool,
    pub legacy: &'a [String],
}

pub struct Names<'a> {
    routes: Vec<RouteName<'a>>,
    exact: HashMap<&'a str, usize>,
    existing: HashMap<&'a str, Option<usize>>,
    bare_fallback: HashMap<&'a str, Option<usize>>,
}

fn claim<'a>(map: &mut HashMap<&'a str, Option<usize>>, name: &'a str, index: usize) {
    if name.is_empty() {
        return;
    }
    map.entry(name)
        .and_modify(|current| {
            if *current != Some(index) {
                *current = None;
            }
        })
        .or_insert(Some(index));
}

fn bare_name(model: &str) -> &str {
    model
        .strip_prefix("copilot/")
        .or_else(|| model.strip_prefix("codex/"))
        .filter(|s| !s.is_empty())
        .unwrap_or(model)
}

impl<'a> Names<'a> {
    pub fn new(routes: impl IntoIterator<Item = RouteName<'a>>) -> Self {
        let routes: Vec<_> = routes.into_iter().collect();
        let mut exact = HashMap::new();
        let mut existing = HashMap::new();
        let mut bare_fallback = HashMap::new();
        for (i, r) in routes.iter().enumerate() {
            exact.insert(r.id, i);
            claim(&mut existing, r.prefixed, i);
            if !r.prefer_prefix {
                claim(&mut existing, r.model, i);
            } else if r.model != r.prefixed {
                claim(&mut existing, r.model, i);
            }
            for alias in r.legacy {
                claim(&mut existing, alias, i);
            }
            claim(&mut bare_fallback, bare_name(r.model), i);
        }
        Self {
            routes,
            exact,
            existing,
            bare_fallback,
        }
    }

    pub fn resolve(&self, name: &str) -> Option<usize> {
        if let Some(index) = self.exact.get(name) {
            return Some(*index);
        }
        if let Some(index) = self.existing.get(name) {
            return *index;
        }
        // Bare model alias resolves only when unambiguous across all active routes.
        self.bare_fallback.get(name).copied().flatten()
    }

    pub fn preferred(&self, index: usize) -> &'a str {
        let route = &self.routes[index];
        let bare = bare_name(route.model);
        if route.prefer_prefix {
            for name in [route.prefixed, route.model, route.id] {
                if !name.is_empty() && self.resolve(name) == Some(index) {
                    return name;
                }
            }
            return route.id;
        }
        for name in [bare, route.prefixed, route.model, route.id] {
            if !name.is_empty() && self.resolve(name) == Some(index) {
                return name;
            }
        }
        route.id
    }

    pub fn aliases(&self, index: usize) -> Vec<&'a str> {
        let route = &self.routes[index];
        let preferred = self.preferred(index);
        let bare = bare_name(route.model);
        let mut seen = HashSet::new();
        std::iter::once(route.id)
            .chain(std::iter::once(route.prefixed))
            .chain(std::iter::once(route.model))
            .chain(std::iter::once(bare))
            .chain(route.legacy.iter().map(String::as_str))
            .filter(|name| {
                !name.is_empty()
                    && *name != preferred
                    && seen.insert(*name)
                    && self.resolve(name) == Some(index)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route<'a>(id: &'a str, model: &'a str, prefixed: &'a str, prefer_prefix: bool) -> RouteName<'a> {
        RouteName {
            id,
            model,
            prefixed,
            prefer_prefix,
            legacy: &[],
        }
    }

    #[test]
    fn codex_and_copilot_with_same_model_publish_distinct_prefixed_names() {
        let names = Names::new([
            route("codex/gpt-6.1--1", "codex/gpt-6.1", "codex/gpt-6.1", true),
            route("copilot/gpt-6.1--2", "copilot/gpt-6.1", "copilot/gpt-6.1", true),
        ]);
        assert_eq!(names.preferred(0), "codex/gpt-6.1");
        assert_eq!(names.preferred(1), "copilot/gpt-6.1");
        assert_eq!(names.resolve("codex/gpt-6.1"), Some(0));
        assert_eq!(names.resolve("copilot/gpt-6.1"), Some(1));
        assert_eq!(names.resolve("gpt-6.1"), None);
    }

    #[test]
    fn unique_subscription_model_publishes_prefixed_and_accepts_bare_alias() {
        let names = Names::new([route(
            "copilot/gemini-4-neon--1",
            "copilot/gemini-4-neon",
            "copilot/gemini-4-neon",
            true,
        )]);
        assert_eq!(names.preferred(0), "copilot/gemini-4-neon");
        assert_eq!(names.resolve("copilot/gemini-4-neon"), Some(0));
        assert_eq!(names.resolve("gemini-4-neon"), Some(0));
    }

    #[test]
    fn colliding_api_providers_use_provider_prefix() {
        let names = Names::new([
            route("eastus/gpt-5.4--1", "gpt-5.4", "eastus/gpt-5.4", true),
            route("sweden/gpt-5.4--2", "gpt-5.4", "sweden/gpt-5.4", true),
        ]);
        assert_eq!(names.preferred(0), "eastus/gpt-5.4");
        assert_eq!(names.preferred(1), "sweden/gpt-5.4");
        assert_eq!(names.resolve("eastus/gpt-5.4"), Some(0));
        assert_eq!(names.resolve("sweden/gpt-5.4"), Some(1));
        assert_eq!(names.resolve("gpt-5.4"), None);
    }

    #[test]
    fn unique_api_model_keeps_clean_bare_name_and_prefixed_alias() {
        let names = Names::new([route("demo--1", "demo", "example/demo", false)]);
        assert_eq!(names.preferred(0), "demo");
        assert_eq!(names.resolve("demo"), Some(0));
        assert_eq!(names.resolve("example/demo"), Some(0));
    }

    #[test]
    fn stable_ids_win_and_published_names_round_trip_without_collisions() {
        let legacy = vec!["old-route".to_string(), "old-route".to_string()];
        let names = Names::new([
            RouteName {
                id: "stable",
                model: "alpha",
                prefixed: "p1/alpha",
                prefer_prefix: false,
                legacy: &legacy,
            },
            route("beta-stable", "copilot/beta", "copilot/beta", true),
            route("collision-stable", "stable", "p2/stable", false),
        ]);
        assert_eq!(names.resolve("stable"), Some(0));
        assert_eq!(names.resolve("old-route"), Some(0));
        let mut unique = HashSet::new();
        for i in 0..3 {
            let name = names.preferred(i);
            assert!(unique.insert(name));
            assert_eq!(names.resolve(name), Some(i));
        }
    }
}
