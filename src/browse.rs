//! Shared debounced search and cursor state for embedded catalogs.
#[derive(Default)]
pub struct Browse {
    signature: Option<String>,
    pending_at: Option<f64>,
    revision: u64,
    pub offset: usize,
    pub resolved_query: Option<String>,
    pub loading: bool,
    pub more: bool,
    pub failed: bool,
    pub reset_scroll: bool,
}
impl Browse {
    pub fn sync(&mut self, signature: String, now: f64, populated: bool) -> bool {
        if self.signature.as_ref() == Some(&signature) {
            return false;
        }
        let initial = self.signature.is_none();
        self.signature = Some(signature);
        if initial && populated {
            self.more = false;
            return false;
        }
        self.reset(now + if initial { 0. } else { 0.35 });
        true
    }
    pub fn invalidate(&mut self) {
        self.reset(0.);
        self.signature = None;
        self.pending_at = None;
    }
    pub fn reset(&mut self, at: f64) {
        self.revision = self.revision.wrapping_add(1);
        self.pending_at = Some(at);
        self.offset = 0;
        self.resolved_query = None;
        self.loading = false;
        self.more = true;
        self.failed = false;
        self.reset_scroll = true;
    }
    pub fn due(&self, now: f64) -> bool {
        !self.loading && self.pending_at.is_some_and(|at| now >= at)
    }
    pub fn pending(&self) -> bool {
        self.pending_at.is_some()
    }
    pub fn begin(&mut self) -> Option<(u64, usize)> {
        if self.loading {
            return None;
        }
        self.loading = true;
        self.failed = false;
        self.pending_at = None;
        Some((self.revision, self.offset))
    }
    pub fn finish(&mut self, revision: u64, offset: usize, count: Option<usize>) -> bool {
        if revision != self.revision || offset != self.offset || !self.loading {
            return false;
        }
        self.loading = false;
        self.failed = count.is_none();
        if let Some(count) = count {
            self.offset += count;
            self.more = count == 20;
        }
        true
    }
}
pub fn query_variants(input: &str) -> Vec<String> {
    let input = input.trim();
    let input = ["https://modrinth.com/", "http://modrinth.com/"]
        .iter()
        .find_map(|prefix| input.strip_prefix(prefix))
        .and_then(|path| {
            let mut parts = path.split('/');
            let kind = parts.next()?;
            matches!(kind, "mod" | "modpack" | "resourcepack").then_some(())?;
            parts
                .next()
                .map(|slug| slug.split(['?', '#']).next().unwrap_or(slug))
        })
        .unwrap_or(input);
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowercase = normalized.to_lowercase();
    let mut variants = vec![normalized];
    let latin = "qwertyuiop[]asdfghjkl;'zxcvbnm,.";
    let cyrillic = "йцукенгшщзхъфывапролджэячсмитьбю";
    let has_cyrillic = lowercase
        .chars()
        .any(|c| ('а'..='я').contains(&c) || c == 'ё');
    let (from, to) = if has_cyrillic {
        (cyrillic, latin)
    } else {
        (latin, cyrillic)
    };
    let layout: String = lowercase
        .chars()
        .map(|c| {
            from.chars()
                .position(|key| key == c)
                .and_then(|index| to.chars().nth(index))
                .unwrap_or(c)
        })
        .collect();
    let transliterated: String = lowercase
        .chars()
        .map(|c| {
            match c {
                'а' => "a",
                'б' => "b",
                'в' => "v",
                'г' => "g",
                'д' => "d",
                'е' | 'э' => "e",
                'ё' => "yo",
                'ж' => "zh",
                'з' => "z",
                'и' | 'й' => "i",
                'к' => "k",
                'л' => "l",
                'м' => "m",
                'н' => "n",
                'о' => "o",
                'п' => "p",
                'р' => "r",
                'с' => "s",
                'т' => "t",
                'у' => "u",
                'ф' => "f",
                'х' => "kh",
                'ц' => "ts",
                'ч' => "ch",
                'ш' => "sh",
                'щ' => "shch",
                'ъ' | 'ь' => "",
                'ы' => "y",
                'ю' => "yu",
                'я' => "ya",
                _ => return c.to_string(),
            }
            .to_owned()
        })
        .collect();
    let translated = match lowercase.as_str() {
        "натрий" | "содиум" => "sodium",
        "радужка" | "ирис" => "iris",
        "оптимизация" => "optimization",
        "анимации" => "animations",
        "текстуры" => "textures",
        "карта" => "map",
        "шейдеры" => "shaders",
        _ => "",
    };
    let spaced = lowercase.replace(['-', '_'], " ");
    let compact: String = lowercase
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect();
    for candidate in [
        translated.to_owned(),
        layout,
        transliterated,
        spaced,
        compact,
    ] {
        if !candidate.is_empty() && !variants.iter().any(|v| v.to_lowercase() == candidate) {
            variants.push(candidate);
        }
    }
    variants
}

pub fn search_variants<T>(
    query: &str,
    resolved: Option<String>,
    mut search: impl FnMut(&str) -> anyhow::Result<Vec<T>>,
) -> anyhow::Result<(String, Vec<T>)> {
    let candidates = resolved
        .map(|q| vec![q])
        .unwrap_or_else(|| query_variants(query));
    for candidate in candidates {
        let results = search(&candidate)?;
        if !results.is_empty() {
            return Ok((candidate, results));
        }
    }
    Ok((query.to_owned(), Vec::new()))
}

pub fn near_end(offset: f32, viewport: f32, content: f32) -> bool {
    offset + viewport + 160. >= content
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrected_search_keeps_its_query_on_later_pages() {
        let (query, hits) = search_variants("   ЫЩВШГЬ   ", None, |query| {
            Ok(if query == "sodium" { vec![1] } else { vec![] })
        })
        .unwrap();
        assert_eq!(query, "sodium");
        assert_eq!(hits, vec![1]);
        search_variants("ыщвшгь", Some(query), |query| {
            assert_eq!(query, "sodium");
            Ok(vec![2])
        })
        .unwrap();
        assert_eq!(query_variants("  Sodium     Extra  ")[0], "Sodium Extra");
        assert!(query_variants("Натрий").contains(&"sodium".into()));
    }
    #[test]
    #[ignore = "Uses live catalog APIs and the locally configured CurseForge key"]
    fn live_named_mod_search() {
        use crate::model::{Config, SearchOptions, Source};
        let mut profile = Config::default().profiles.remove(0);
        profile.game_version = "1.12.2".into();
        profile.loader = "forge".into();
        let api = crate::api::Api::new(String::new()).unwrap();
        let (_, hits) = search_variants("  ЫЩВШГЬ  ", None, |q| {
            api.search_filtered(Source::Modrinth, q, &profile, 0, &SearchOptions::default())
        })
        .unwrap();
        assert!(hits.iter().any(|p| p.name == "Sodium"));
        println!("Modrinth: Sodium found from wrong layout in a Forge 1.12.2 profile");
        let root = directories::ProjectDirs::from("dev", "MLauncher", "MLauncher").unwrap();
        let (_, stored) = crate::secrets::load_api_keys(root.data_local_dir());
        let key = std::env::var("CURSEFORGE_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(stored);
        if key.trim().is_empty() {
            println!("CurseForge: no configured key available");
            return;
        }
        let api = crate::api::Api::new(key).unwrap();
        let hits = api.search(Source::CurseForge, "JEI", &profile, 0).unwrap();
        assert!(hits.iter().any(|p| p.id == "238222"));
        println!("CurseForge: JEI found with configured key");
    }
    #[test]
    #[ignore = "Uses the public Modrinth API"]
    fn live_catalog_pagination() {
        let api = crate::api::Api::new(String::new()).unwrap();
        let mut profile = crate::model::Config::default().profiles.remove(0);
        profile.loader = "fabric".into();
        let first = api
            .search(crate::model::Source::Modrinth, "", &profile, 0)
            .unwrap();
        let next = api
            .search(crate::model::Source::Modrinth, "", &profile, 20)
            .unwrap();
        assert_eq!(first.len(), 20);
        assert_eq!(next.len(), 20);
        assert!(next.iter().all(|p| first.iter().all(|old| old.id != p.id)));
        println!("Mods: two disjoint pages of 20 OK");
        let service = crate::packs::Service::new(String::new()).unwrap();
        let first = service
            .search_page(
                crate::packs::Provider::Modrinth,
                "",
                true,
                "1.21.1",
                crate::model::SortOrder::Downloads,
                0,
            )
            .unwrap();
        let next = service
            .search_page(
                crate::packs::Provider::Modrinth,
                "",
                true,
                "1.21.1",
                crate::model::SortOrder::Downloads,
                20,
            )
            .unwrap();
        assert_eq!(first.len(), 20);
        assert_eq!(next.len(), 20);
        assert!(next.iter().all(|p| first.iter().all(|old| old.id != p.id)));
        println!("Resourcepacks: two disjoint pages of 20 OK");
    }
    #[test]
    fn debounce_invalidates_old_responses_and_preserves_cursor_on_retry() {
        let mut b = Browse::default();
        assert!(b.sync("old".into(), 0., false));
        let old = b.begin().unwrap();
        b.sync("new".into(), 1., false);
        assert!(!b.finish(old.0, old.1, Some(20)));
        assert!(!b.due(1.2));
        assert!(b.due(1.4));
        let first = b.begin().unwrap();
        assert!(b.begin().is_none());
        assert!(b.finish(first.0, first.1, Some(20)));
        let next = b.begin().unwrap();
        assert_eq!(next.1, 20);
        b.finish(next.0, next.1, None);
        assert!(b.failed);
        assert_eq!(b.offset, 20);
        let retry = b.begin().unwrap();
        b.finish(retry.0, retry.1, Some(3));
        assert!(!b.more);
        assert_eq!(b.offset, 23);
    }
}
