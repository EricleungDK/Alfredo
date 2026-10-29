//! `@path` completion off the key path (issue #524).
//!
//! Listing a repository and vetting candidates costs tens of milliseconds, so the run
//! loop never does it inline. A key hands the draft to [`AtLookup::for_draft`]; the lookup
//! runs on a runtime blocking worker and the loop applies [`AtLookup::poll`]'s answer on
//! the wake that follows. Every request gets a generation: an answer for an older request
//! is dropped, and while one lookup runs only the newest later request is kept. The
//! candidate [`Listing`] is cached for the keystrokes of one `@` word and dropped as soon
//! as the draft stops being one; vetting of the listed paths is never cached.
use crate::chat_mentions::{self, Listing};
use crate::commands::Completion;
use std::{path::Path, path::PathBuf, sync::Arc};
use tokio::{runtime::Runtime, task::JoinHandle};

struct Request {
    generation: u64,
    draft: String,
    fragment: String,
    root: PathBuf,
}

struct Done {
    /// A listing taken by this lookup, to keep for the next key of the word.
    listing: Option<(PathBuf, Arc<Listing>)>,
    paths: Vec<String>,
}

struct Running {
    request: Request,
    handle: JoinHandle<Done>,
}

/// The answer to the newest request: the draft it was for, and the paths to offer.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub draft: String,
    pub paths: Vec<String>,
}

#[derive(Default)]
pub struct AtLookup {
    generation: u64,
    cache: Option<(PathBuf, Arc<Listing>)>,
    running: Option<Running>,
    queued: Option<Request>,
}

impl AtLookup {
    /// The completion for a chat draft after an edit. Slash type-ahead is immediate. An
    /// `@path` word at the end of the draft starts a lookup and shows nothing until
    /// [`poll`](Self::poll) answers; any other draft cancels the lookups of an older one.
    pub fn for_draft(
        &mut self,
        runtime: &Runtime,
        draft: &str,
        cursor: usize,
        workspace: &Path,
    ) -> Option<Completion> {
        if let Some(found) = Completion::typeahead(draft) {
            self.cancel();
            return Some(found);
        }
        match Completion::at_fragment(draft).filter(|_| cursor == draft.len()) {
            Some((_, fragment)) => self.request(runtime, draft, fragment, workspace),
            None => self.cancel(),
        }
        None
    }

    fn request(&mut self, runtime: &Runtime, draft: &str, fragment: &str, root: &Path) {
        self.generation += 1;
        let request = Request {
            generation: self.generation,
            draft: draft.to_string(),
            fragment: fragment.to_string(),
            root: root.to_path_buf(),
        };
        if self.running.is_some() {
            self.queued = Some(request);
        } else {
            self.start(runtime, request);
        }
    }

    /// Drop every pending and queued lookup and the cached listing.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.queued = None;
        self.cache = None;
    }

    /// Whether a lookup is running or queued.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.queued.is_some()
    }

    fn start(&mut self, runtime: &Runtime, request: Request) {
        let cached = self
            .cache
            .as_ref()
            .filter(|(root, _)| *root == request.root)
            .map(|(_, listing)| Arc::clone(listing));
        let (root, fragment) = (request.root.clone(), request.fragment.clone());
        let handle = crate::wake::spawn_blocking(runtime, move || {
            let (listing, fresh) = match cached {
                Some(listing) => (listing, None),
                None => {
                    let listing = Arc::new(chat_mentions::listing(&root));
                    let fresh = (!listing.is_empty()).then(|| (root.clone(), Arc::clone(&listing)));
                    (listing, fresh)
                }
            };
            let paths = chat_mentions::complete_listed(&root, &listing, &fragment);
            Done {
                listing: fresh,
                paths,
            }
        });
        self.running = Some(Running { request, handle });
    }

    /// The answer of a finished lookup, if it is for the newest request. An older
    /// answer is dropped, and the newest queued request then starts.
    pub fn poll(&mut self, runtime: &Runtime) -> Option<Found> {
        if !self.running.as_ref().is_some_and(|run| run.handle.is_finished()) {
            return None;
        }
        let running = self.running.take()?;
        let done = runtime.block_on(running.handle).ok();
        let current = running.request.generation == self.generation;
        if let Some(queued) = self.queued.take() {
            self.start(runtime, queued);
        }
        let done = done?;
        if current {
            if let Some(listing) = done.listing {
                self.cache = Some(listing);
            }
            return Some(Found {
                draft: running.request.draft,
                paths: done.paths,
            });
        }
        None
    }
}
