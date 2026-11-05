//! What the source has taken this session: ratings, saves, follows and removed history rows, over the refs listings handed out.

use std::collections::{HashMap, HashSet};

use api::{ApiEvent, CatalogActions, CatalogItem, PlaylistEdit, PlaylistPrivacy, Rating};
use gpui_kit::{App, Context, Entity, Subscription};
use link::Link;

use crate::notices::Notices;

/// A listing's refs, kept by track key so a track row's menu can rate it, and what each verb has since set.
pub struct CatalogMarks {
    known: HashMap<String, CatalogActions>,
    taken: Taken,
    /// The favorites last seen, so a heart flipped anywhere moves a noted song's like with it.
    hearts: Option<HashSet<String>>,
    _subscriptions: [Subscription; 3],
}

/// What the source said yes to, by the ref it was sent with.
#[derive(Default)]
struct Taken {
    rating: HashMap<String, Rating>,
    saved: HashMap<String, bool>,
    followed: HashMap<String, bool>,
    removed: HashSet<String>,
    /// A playlist's description and privacy as last saved, by playlist id.
    details: HashMap<String, (String, PlaylistPrivacy)>,
}

impl Taken {
    fn over(&self, actions: &CatalogActions) -> CatalogActions {
        let mut now = actions.clone();
        if let Some(rating) = actions.rate_ref.as_ref().and_then(|at| self.rating.get(at)) {
            now.rating = Some(*rating);
        }
        if let Some(saved) = actions.save_ref.as_ref().and_then(|at| self.saved.get(at)) {
            now.saved = Some(*saved);
        }
        if let Some(followed) = actions
            .follow_ref
            .as_ref()
            .and_then(|at| self.followed.get(at))
        {
            now.followed = Some(*followed);
        }
        if actions
            .history_token
            .as_ref()
            .is_some_and(|token| self.removed.contains(token))
        {
            now.history_token = None;
        }
        now
    }

    /// Bring a song's rating in line with its heart; whether that changed it.
    fn heart(&mut self, actions: &CatalogActions, favorite: bool) -> bool {
        let Some(at) = &actions.rate_ref else {
            return false;
        };
        let liked = self.over(actions).rating == Some(Rating::Like);
        let rating = match (favorite, liked) {
            (true, false) => Rating::Like,
            (false, true) => Rating::None,
            _ => return false,
        };
        self.rating.insert(at.clone(), rating);
        true
    }
}

impl CatalogMarks {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        // Refs belong to the source that issued them.
        let resync = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| {
            if matches!(event, ApiEvent::Resync) {
                this.forget(cx);
            }
        });
        let switch = cx.subscribe(
            &crate::SourcesModel::global(cx),
            |this, _, _: &crate::SwitchStarted, cx| this.forget(cx),
        );
        let hearts = cx.observe(&crate::FavoritesModel::global(cx), |this, favorites, cx| {
            let favorites = favorites.read(cx);
            let now = favorites
                .is_loaded()
                .then(|| favorites.keys().cloned().collect());
            this.follow_hearts(now, cx)
        });
        Self {
            known: HashMap::new(),
            taken: Taken::default(),
            hearts: None,
            _subscriptions: [resync, switch, hearts],
        }
    }

    fn follow_hearts(&mut self, now: Option<HashSet<String>>, cx: &mut Context<Self>) {
        let Some(now) = now else {
            return;
        };
        let Some(before) = self.hearts.replace(now.clone()) else {
            return;
        };
        let mut changed = false;
        for key in before.symmetric_difference(&now) {
            if let Some(actions) = self.known.get(key) {
                changed |= self.taken.heart(actions, now.contains(key));
            }
        }
        if changed {
            crate::notify(cx);
        }
    }

    fn forget(&mut self, cx: &mut Context<Self>) {
        self.known.clear();
        self.taken = Taken::default();
        self.hearts = None;
        crate::notify(cx);
    }

    /// Remember the refs a shelf's song tiles carry, so their rows can be rated by key.
    pub fn note(&mut self, items: &[CatalogItem]) {
        for item in items {
            if let Some(track) = &item.track {
                self.known.insert(track.key.clone(), item.actions.clone());
            }
        }
    }

    /// The refs noted for a track, as the source now has them.
    pub fn for_track(&self, key: &str) -> Option<CatalogActions> {
        self.known.get(key).map(|actions| self.current(actions))
    }

    /// `actions` as the source now has them, after whatever this session changed.
    pub fn current(&self, actions: &CatalogActions) -> CatalogActions {
        self.taken.over(actions)
    }

    /// Whether a history row was removed, which is a listing's cue to drop it.
    pub fn is_removed(&self, token: &str) -> bool {
        self.taken.removed.contains(token)
    }

    /// A playlist's description and privacy as this session last saved them, over what its page was read with.
    pub fn details(&self, id: &str) -> Option<(&str, PlaylistPrivacy)> {
        let (description, privacy) = self.taken.details.get(id)?;
        Some((description.as_str(), *privacy))
    }

    pub fn edit_details(id: String, description: String, privacy: PlaylistPrivacy, cx: &mut App) {
        let sent = id.clone();
        let edit = PlaylistEdit {
            name: None,
            description: Some(description.clone()),
            privacy: Some(privacy),
        };
        Self::send(
            cx,
            move |api| async move { api.edit_playlist(sent, edit).await },
            move |marks| {
                marks.taken.details.insert(id, (description, privacy));
            },
        );
    }

    /// Like, dislike or clear; a like on a song also sets its favorite at the source.
    pub fn rate(item_ref: String, rating: Rating, cx: &mut App) {
        let sent = item_ref.clone();
        Self::send(
            cx,
            move |api| async move { api.rate(sent, rating).await },
            move |marks| {
                marks.taken.rating.insert(item_ref, rating);
            },
        );
    }

    pub fn save(item_ref: String, saved: bool, cx: &mut App) {
        let sent = item_ref.clone();
        Self::send(
            cx,
            move |api| async move { api.save(sent, saved).await },
            move |marks| {
                marks.taken.saved.insert(item_ref, saved);
            },
        );
    }

    pub fn follow(artist_ref: String, follow: bool, cx: &mut App) {
        let sent = artist_ref.clone();
        Self::send(
            cx,
            move |api| async move { api.follow(sent, follow).await },
            move |marks| {
                marks.taken.followed.insert(artist_ref, follow);
            },
        );
    }

    pub fn remove_from_history(token: String, cx: &mut App) {
        let sent = token.clone();
        Self::send(
            cx,
            move |api| async move { api.remove_from_history(sent).await },
            move |marks| {
                marks.taken.removed.insert(token);
            },
        );
    }

    /// The mark is taken only once the source said yes, so a control flips on success alone.
    fn send<Fut>(
        cx: &mut App,
        call: impl FnOnce(std::sync::Arc<dyn api::KopuzApi>) -> Fut,
        took: impl FnOnce(&mut Self) + 'static,
    ) where
        Fut: std::future::Future<Output = Result<(), api::ApiError>> + Send + 'static,
    {
        let call = Link::global(cx).call(call);
        let marks = Self::global(cx).downgrade();
        cx.spawn(async move |cx| match call.await {
            Ok(()) => {
                marks
                    .update(cx, |marks, cx| {
                        took(marks);
                        crate::notify(cx);
                    })
                    .ok();
            }
            Err(error) => cx.update(|cx| Notices::failed(&error, cx)),
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> CatalogActions {
        CatalogActions {
            rate_ref: Some("r".into()),
            rating: Some(Rating::None),
            save_ref: Some("s".into()),
            saved: Some(false),
            follow_ref: Some("f".into()),
            followed: Some(false),
            history_token: Some("h".into()),
        }
    }

    fn taken() -> Taken {
        Taken {
            rating: HashMap::from([("r".to_string(), Rating::Like)]),
            saved: HashMap::from([("s".to_string(), true)]),
            followed: HashMap::from([("f".to_string(), true)]),
            removed: HashSet::from(["h".to_string()]),
            details: HashMap::new(),
        }
    }

    #[test]
    fn what_the_source_took_overrides_what_the_listing_said() {
        let now = taken().over(&refs());
        assert_eq!(now.rating, Some(Rating::Like));
        assert_eq!(now.saved, Some(true));
        assert_eq!(now.followed, Some(true));
        assert_eq!(now.history_token, None);
    }

    #[test]
    fn a_heart_flipped_elsewhere_moves_the_like_with_it() {
        let mut taken = taken();
        assert!(taken.heart(&refs(), false));
        assert_eq!(taken.over(&refs()).rating, Some(Rating::None));
        assert!(taken.heart(&refs(), true));
        assert_eq!(taken.over(&refs()).rating, Some(Rating::Like));
        assert!(!taken.heart(&refs(), true));
    }

    #[test]
    fn refs_nobody_touched_keep_the_listing_state() {
        let untouched = CatalogActions {
            rate_ref: Some("other".into()),
            rating: Some(Rating::Dislike),
            ..CatalogActions::default()
        };
        assert_eq!(taken().over(&untouched), untouched);
    }
}
