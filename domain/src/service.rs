use std::sync::atomic::{AtomicU64, Ordering};

use crate::validate::{validate_custom_slug, validate_original_url};
use crate::{Clock, CoreError, LinkRepository, NewLink, ShortLink, Slug, SlugGenerator};

/// Application service orchestrating creation and resolution of short links.
///
/// It remains generic over repository, slug generator, and clock, and keeps a
/// simple internal monotonically increasing counter to provide IDs for slug
/// generation in the absence of a database-backed counter. This keeps the
/// domain testable without external dependencies.
pub struct LinkService<R: LinkRepository, G: SlugGenerator, C: Clock> {
    repo: R,
    slugger: G,
    clock: C,
    next_id: AtomicU64,
}

impl<R: LinkRepository, G: SlugGenerator, C: Clock> LinkService<R, G, C> {
    pub fn new(repo: R, slugger: G, clock: C) -> Self {
        Self {
            repo,
            slugger,
            clock,
            next_id: AtomicU64::new(0),
        }
    }

    fn reserve_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Create a new short link.
    pub fn create(&self, input: NewLink) -> Result<ShortLink, CoreError> {
        // Validate inputs
        validate_original_url(&input.original_url)?;
        if let Some(ref custom) = input.custom_slug {
            validate_custom_slug(custom.as_str())?;
            if self.repo.get(custom)?.is_some() {
                return Err(CoreError::AlreadyExists);
            }
            return self.persist_with_slug(custom.clone(), input);
        }

        // Generate slug from an internal increasing id; retry on unlikely collision
        for _ in 0..100 {
            // hard cap to avoid infinite loop in degenerate cases
            let id = self.reserve_id();
            let slug = self.slugger.next_slug(id);
            if self.repo.get(&slug)?.is_none() {
                return self.persist_with_slug(slug, input);
            }
        }
        Err(CoreError::Repository(
            "failed to generate unique slug".into(),
        ))
    }

    fn persist_with_slug(&self, slug: Slug, input: NewLink) -> Result<ShortLink, CoreError> {
        let link = ShortLink::new(slug, input.original_url, self.clock.now(), input.user_email);
        self.repo.put(link.clone())?;
        Ok(link)
    }

    /// Resolve a slug to its original URL.
    pub fn resolve(&self, slug: &Slug) -> Result<String, CoreError> {
        match self.repo.get(slug)? {
            Some(link) => Ok(link.original_url),
            None => Err(CoreError::NotFound),
        }
    }

    /// List short links up to the given limit.
    pub fn list(&self, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        self.repo.list(limit)
    }

    /// Update an existing link (original_url, is_active).
    pub fn update(&self, link: &ShortLink) -> Result<(), CoreError> {
        self.repo.update(link)
    }

    /// Atomically increment the click count for a link.
    pub fn increment_click(&self, slug: &Slug) -> Result<(), CoreError> {
        self.repo.increment_click(slug)
    }

    /// List links created by a specific user.
    pub fn list_by_creator(
        &self,
        email: &crate::UserEmail,
        limit: usize,
    ) -> Result<Vec<ShortLink>, CoreError> {
        self.repo.list_by_creator(email, limit)
    }

    /// Get a link by slug (exposes repo.get for update workflows).
    pub fn get(&self, slug: &Slug) -> Result<Option<ShortLink>, CoreError> {
        self.repo.get(slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_repo::InMemoryRepo;
    use crate::slug::Base62SlugGenerator;
    use crate::{Slug, UserEmail};
    use std::time::SystemTime;

    struct TestClock;
    impl Clock for TestClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }
    }

    #[test]
    fn create_auto_generates_and_resolves() {
        let svc = LinkService::new(InMemoryRepo::new(), Base62SlugGenerator::new(1), TestClock);
        let input = NewLink {
            original_url: "https://example.com".to_string(),
            custom_slug: None,
            user_email: UserEmail::new("user@example.com").unwrap(),
        };
        let created = svc.create(input).expect("created");
        let url = svc.resolve(&created.slug).unwrap();
        assert_eq!(url, "https://example.com");
    }

    #[test]
    fn create_with_custom_slug_and_collision() {
        let svc = LinkService::new(InMemoryRepo::new(), Base62SlugGenerator::new(1), TestClock);
        let custom = Slug::new("custom1").unwrap();
        let a = NewLink {
            original_url: "https://one".to_string(),
            custom_slug: Some(custom.clone()),
            user_email: UserEmail::new("a@e.com").unwrap(),
        };
        let _ = svc.create(a).unwrap();

        let b = NewLink {
            original_url: "https://two".to_string(),
            custom_slug: Some(custom.clone()),
            user_email: UserEmail::new("b@e.com").unwrap(),
        };
        let err = svc.create(b).unwrap_err();
        assert!(matches!(err, CoreError::AlreadyExists));
    }

    #[test]
    fn resolve_not_found() {
        let svc = LinkService::new(InMemoryRepo::new(), Base62SlugGenerator::new(1), TestClock);
        let missing = Slug::new("missing").unwrap();
        let err = svc.resolve(&missing).unwrap_err();
        assert!(matches!(err, CoreError::NotFound));
    }

    #[test]
    fn list_returns_items() {
        let svc = LinkService::new(InMemoryRepo::new(), Base62SlugGenerator::new(1), TestClock);
        for i in 0..3 {
            let _ = svc.create(NewLink {
                original_url: format!("https://e/{}", i),
                custom_slug: None,
                user_email: UserEmail::new("u@e.com").unwrap(),
            });
        }
        let items = svc.list(2).unwrap();
        assert_eq!(items.len(), 2);
    }
}
