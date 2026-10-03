//! Public request names are a projection, never a replacement for persisted route IDs.
use std::collections::{HashMap, HashSet};
pub struct RouteName<'a> {
    pub id: &'a str,
    pub model: &'a str,
    pub legacy: &'a [String],
}
pub struct Names<'a> {
    routes: Vec<RouteName<'a>>,
    exact: HashMap<&'a str, usize>,
    existing: HashMap<&'a str, Option<usize>>,
    copilot: HashMap<&'a str, Option<usize>>,
}
fn claim<'a>(map: &mut HashMap<&'a str, Option<usize>>, name: &'a str, index: usize) {
    map.entry(name)
        .and_modify(|current| {
            if *current != Some(index) {
                *current = None;
            }
        })
        .or_insert(Some(index));
}
impl<'a> Names<'a> {
    pub fn new(routes: impl IntoIterator<Item = RouteName<'a>>) -> Self {
        let routes: Vec<_> = routes.into_iter().collect();
        let mut exact = HashMap::new();
        let mut existing = HashMap::new();
        let mut copilot = HashMap::new();
        for (i, r) in routes.iter().enumerate() {
            exact.insert(r.id, i);
            claim(&mut existing, r.model, i);
            for alias in r.legacy {
                claim(&mut existing, alias, i);
            }
            if let Some(bare) = r.model.strip_prefix("copilot/").filter(|s| !s.is_empty()) {
                claim(&mut copilot, bare, i);
            }
        }
        Self {
            routes,
            exact,
            existing,
            copilot,
        }
    }
    pub fn resolve(&self, name: &str) -> Option<usize> {
        if let Some(index) = self.exact.get(name) {
            return Some(*index);
        }
        if let Some(index) = self.existing.get(name) {
            return *index;
        }
        // A bare Copilot alias is fallback-only. Never hijack an existing API route
        // or escape its ambiguity/blocked state by selecting a different source.
        self.copilot.get(name).copied().flatten()
    }
    pub fn preferred(&self, index: usize) -> &'a str {
        let route = &self.routes[index];
        let bare = route.model.strip_prefix("copilot/").unwrap_or(route.model);
        for name in [bare, route.model, route.id] {
            if self.resolve(name) == Some(index) {
                return name;
            }
        }
        route.id
    }
    pub fn aliases(&self, index: usize) -> Vec<&'a str> {
        let route = &self.routes[index];
        let preferred = self.preferred(index);
        let mut seen = HashSet::new();
        std::iter::once(route.id)
            .chain(std::iter::once(route.model))
            .chain(route.legacy.iter().map(String::as_str))
            .filter(|name| {
                *name != preferred && seen.insert(*name) && self.resolve(name) == Some(index)
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn route<'a>(id: &'a str, model: &'a str) -> RouteName<'a> {
        RouteName {
            id,
            model,
            legacy: &[],
        }
    }
    #[test]
    fn copilot_only_accepts_the_normal_model_name() {
        let names = Names::new([route("copilot/gpt-5.4--stable", "copilot/gpt-5.4")]);
        assert_eq!(names.resolve("gpt-5.4"), Some(0));
        assert_eq!(names.preferred(0), "gpt-5.4");
        assert_eq!(names.resolve("copilot/gpt-5.4"), Some(0));
        assert_eq!(names.resolve("copilot/gpt-5.4--stable"), Some(0));
    }
    #[test]
    fn unrelated_api_providers_do_not_prevent_a_copilot_bare_name() {
        let names = Names::new([
            route("other--stable", "other"),
            route("copilot/gpt-5.4--stable", "copilot/gpt-5.4"),
        ]);
        assert_eq!(names.resolve("gpt-5.4"), Some(1));
    }
    #[test]
    fn an_existing_ordinary_route_keeps_its_ownership() {
        let names = Names::new([
            route("ordinary--stable", "gpt-5.4"),
            route("copilot--stable", "copilot/gpt-5.4"),
        ]);
        assert_eq!(names.resolve("gpt-5.4"), Some(0));
        assert_eq!(names.preferred(1), "copilot/gpt-5.4");
    }
    #[test]
    fn ambiguous_existing_names_do_not_fall_through_to_another_source() {
        let names = Names::new([
            route("api-chat", "gpt-5.4"),
            route("api-responses", "gpt-5.4"),
            route("copilot--stable", "copilot/gpt-5.4"),
        ]);
        assert_eq!(names.resolve("gpt-5.4"), None);
        assert_eq!(names.preferred(0), "api-chat");
    }
    #[test]
    fn duplicate_copilot_protocols_are_not_guessed() {
        let names = Names::new([
            route("copilot-chat", "copilot/gpt-5.4"),
            route("copilot-responses", "copilot/gpt-5.4"),
        ]);
        assert_eq!(names.resolve("gpt-5.4"), None);
        assert_eq!(names.preferred(0), "copilot-chat");
    }
    #[test]
    fn stable_ids_win_and_published_names_round_trip_without_collisions() {
        let legacy = vec!["old-route".to_string(), "old-route".to_string()];
        let names = Names::new([
            RouteName {
                id: "stable",
                model: "alpha",
                legacy: &legacy,
            },
            route("beta-stable", "copilot/beta"),
            route("collision-stable", "stable"),
        ]);
        assert_eq!(names.resolve("stable"), Some(0));
        assert_eq!(names.resolve("old-route"), Some(0));
        let mut unique = HashSet::new();
        for i in 0..3 {
            let name = names.preferred(i);
            assert!(unique.insert(name));
            assert_eq!(names.resolve(name), Some(i));
            for alias in names.aliases(i) {
                assert_eq!(names.resolve(alias), Some(i));
            }
        }
    }
}
