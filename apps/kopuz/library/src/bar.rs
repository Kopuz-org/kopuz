//! The chrome over the listing: the facet switch, the filter bar and the selection pill.

use api::TrackFilter;
use commands::Command;
use components::format::{t, t_with};
use components::{
    FilterBar, FilterChip, FilterEntry, PageHeader, Segment, Segmented, SortControl, icons, style,
};
use config::{SortCriterion, SortDirection};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
    div,
};
use store::layout::{Density, TileSize, ViewMode};
use store::{Link, Target};

use crate::Library;
use crate::facet::Facet;
use crate::filters::{Chip, Filters};

impl Library {
    /// The heading, the facet switch, the filter bar and the facet's own controls.
    pub(crate) fn chrome(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.total();
        // Favorites is one facet, so there is nothing to switch between.
        let switch = (!self.is_liked()).then(|| self.facet_switch(cx));
        let controls = self.controls(cx);
        let filter_bar = self.filter_bar(cx);
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_4()
            .pb_4()
            .child(
                PageHeader::new(self.title())
                    .count(t_with("library_count", &[("count", count.into())]))
                    .flush()
                    .action(controls),
            )
            .children(switch)
            .child(filter_bar)
            .into_any_element()
            .tap(window)
    }

    /// Tracks, Albums, Artists, Playlists: the same listing, presented four ways.
    fn facet_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.filters().facet;
        let page = cx.entity().downgrade();
        div()
            .flex()
            .self_start()
            .child(
                Segmented::new("library-facet")
                    .segments(Facet::ALL.iter().map(|facet| {
                        Segment::new(facet.name())
                            .icon(facet.icon())
                            .label(t(facet.label_key()))
                    }))
                    .selected(Facet::ALL.iter().position(|facet| *facet == active))
                    .on_select(move |index, _, cx| {
                        let Some(facet) = Facet::ALL.get(*index).copied() else {
                            return;
                        };
                        page.update(cx, |this, cx| {
                            let mut filters = this.filters().clone();
                            filters.facet = facet;
                            this.set_filters(filters, true, cx);
                        })
                        .ok();
                    }),
            )
            .into_any_element()
    }

    /// Type to filter, then the chips that compose on top of what was typed.
    fn filter_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let filters = self.filters().clone();
        // On Favorites the liked filter is the page itself, not a narrowing to clear.
        let narrowed = match self.is_liked() {
            true => filters != Filters::default().held_to_liked(),
            false => filters.is_narrowed(),
        };
        let mut bar = FilterBar::new(&self.search)
            .narrowed(narrowed)
            .on_clear(cx.listener(|this, _, window, cx| {
                let cleared = this.filters().cleared();
                this.search
                    .update(cx, |state, cx| state.set_value("", window, cx));
                this.set_filters(cleared, true, cx);
            }));
        let liked = self.is_liked();
        for chip in Chip::ALL
            .iter()
            .copied()
            .filter(|chip| filters.applies(*chip))
            .filter(|chip| !(liked && *chip == Chip::Favorites))
        {
            bar = bar.chip(self.chip(chip, &filters, cx));
        }
        bar.into_any_element()
    }

    /// One chip: lit when it is narrowing, and carrying its own menu when it picks a value.
    fn chip(&self, chip: Chip, filters: &Filters, cx: &mut Context<Self>) -> FilterChip {
        let (lit, label) = match chip {
            Chip::Favorites => (filters.favorites, t(chip.label_key())),
            Chip::Downloaded => (filters.downloaded, t(chip.label_key())),
            Chip::Genre => match &filters.genre {
                Some(genre) => (true, SharedString::from(genre.clone())),
                None => (false, t(chip.label_key())),
            },
            Chip::Years => match filters.years {
                Some((from, to)) => (true, SharedString::from(format!("{from}–{to}"))),
                None => (false, t(chip.label_key())),
            },
            Chip::Folder => match &filters.folder {
                Some(folder) => (true, SharedString::from(leaf_of(folder))),
                None => (false, t(chip.label_key())),
            },
        };
        let page = cx.entity().downgrade();
        if !chip.has_menu() {
            return FilterChip::toggle(chip.id(), chip.icon(), label, lit, move |_, cx| {
                page.update(cx, |this, cx| {
                    let mut filters = this.filters().clone();
                    match chip {
                        Chip::Favorites => filters.favorites = !filters.favorites,
                        Chip::Downloaded => filters.downloaded = !filters.downloaded,
                        _ => {}
                    }
                    this.set_filters(filters, true, cx);
                })
                .ok();
            });
        }
        let entries = self.chip_entries(chip, cx);
        FilterChip::menu(
            chip.id(),
            chip.icon(),
            label,
            lit,
            entries,
            move |value, _, cx| {
                page.update(cx, |this, cx| {
                    let mut filters = this.filters().clone();
                    apply_chip(&mut filters, chip, value);
                    this.set_filters(filters, true, cx);
                })
                .ok();
            },
        )
    }

    /// What a menu chip offers: the value it would set, what to call it, and whether it is on.
    fn chip_entries(&self, chip: Chip, cx: &App) -> Vec<FilterEntry> {
        let filters = self.filters();
        let entry = |value: Option<String>, label: SharedString, checked: bool| FilterEntry {
            value,
            label,
            checked,
        };
        let mut entries = vec![entry(
            None,
            t("library_chip_any"),
            chip_is_off(filters, chip),
        )];
        match chip {
            Chip::Genre => {
                for genre in self.known_genres(cx) {
                    let checked = filters.genre.as_deref() == Some(genre.as_str());
                    entries.push(entry(
                        Some(genre.clone()),
                        SharedString::from(genre),
                        checked,
                    ));
                }
            }
            Chip::Years => {
                for (from, to) in decades(self.year_span()) {
                    let checked = filters.years == Some((from, to));
                    entries.push(entry(
                        Some(format!("{from}-{to}")),
                        SharedString::from(format!("{from}–{to}")),
                        checked,
                    ));
                }
            }
            Chip::Folder => {
                for root in scan_roots(cx) {
                    let checked = filters.folder.as_deref() == Some(root.as_str());
                    entries.push(entry(
                        Some(root.clone()),
                        SharedString::from(leaf_of(&root)),
                        checked,
                    ));
                }
            }
            _ => {}
        }
        entries
    }

    /// The facet's own knobs: how it is drawn, how it is ordered, and what it groups by.
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let facet = self.filters().facet;
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .when(facet == Facet::Tracks, |row| {
                row.child(self.group_control(cx))
                    .child(self.density_control(cx))
                    .child(self.columns_control(cx))
            })
            .when(facet != Facet::Tracks, |row| {
                row.child(self.view_control(cx))
                    .child(self.tile_control(cx))
                    .child(self.grid_sort_control(cx))
            })
            .into_any_element()
    }

    fn quiet(&self, id: &'static str, icon: &'static str, label: SharedString, cx: &App) -> Button {
        Button::new(id)
            .custom(style::raised_button(style::white(0.7, cx), cx))
            .icon(icons::icon(icon))
            .label(label)
            .h_7()
            .px_3()
            .text_xs()
    }

    fn group_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let held = self.group_by().map(str::to_string);
        let label = match held.as_deref() {
            Some(field) => t(group_label_key(field)),
            None => t("library_group_none"),
        };
        let page = cx.entity().downgrade();
        self.quiet(
            "library-group",
            icons::ROWS,
            SharedString::from(format!("{}: {}", t("library_group_by"), label)),
            cx,
        )
        .dropdown_menu(move |mut menu, _, _| {
            for field in [None, Some("album"), Some("artist"), Some("year")] {
                let page = page.clone();
                let checked = held.as_deref() == field;
                let name = match field {
                    Some(field) => t(group_label_key(field)),
                    None => t("library_group_none"),
                };
                menu = menu.item(PopupMenuItem::new(name).checked(checked).on_click(
                    move |_, _, cx| {
                        page.update(cx, |this, cx| {
                            let group = field.map(str::to_string);
                            // Groups are only contiguous when the listing runs in their order.
                            let sort = group.clone().map(|field| store::layout::Sort {
                                field,
                                descending: false,
                            });
                            this.write_facet(cx, move |facet| {
                                facet.group_by = group;
                                if let Some(sort) = sort {
                                    facet.sort = sort;
                                }
                            });
                        })
                        .ok();
                    },
                ));
            }
            menu
        })
        .into_any_element()
    }

    fn density_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let held = self.density();
        let page = cx.entity().downgrade();
        self.quiet(
            "library-density",
            icons::GRID,
            t(density_label_key(held)),
            cx,
        )
        .dropdown_menu(move |mut menu, _, _| {
            for density in [Density::Comfortable, Density::Compact] {
                let page = page.clone();
                menu = menu.item(
                    PopupMenuItem::new(t(density_label_key(density)))
                        .checked(density == held)
                        .on_click(move |_, _, cx| {
                            page.update(cx, |this, cx| {
                                this.write_facet(cx, move |facet| facet.density = density)
                            })
                            .ok();
                        }),
                );
            }
            menu
        })
        .into_any_element()
    }

    /// Which columns the table draws; the same menu the header's right-click opens.
    fn columns_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity().downgrade();
        let held = self.columns().to_vec();
        self.quiet("library-columns", icons::LIST, t("library_columns"), cx)
            .dropdown_menu(move |menu, window, cx| {
                crate::tracks::columns_menu(page.clone(), &held, menu, window, cx)
            })
            .into_any_element()
    }

    fn view_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity().downgrade();
        components::view_toggle(
            "library-view",
            self.mode == ViewMode::Grid,
            move |grid, cx| {
                let next = match grid {
                    true => ViewMode::Grid,
                    false => ViewMode::Table,
                };
                page.update(cx, |this, cx| {
                    this.write_facet(cx, move |facet| facet.mode = next)
                })
                .ok();
            },
        )
        .into_any_element()
    }

    fn tile_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let held = self.tile;
        let page = cx.entity().downgrade();
        self.quiet("library-tile", icons::EXPAND, t(tile_label_key(held)), cx)
            .dropdown_menu(move |mut menu, _, _| {
                for size in [TileSize::Small, TileSize::Medium, TileSize::Large] {
                    let page = page.clone();
                    menu = menu.item(
                        PopupMenuItem::new(t(tile_label_key(size)))
                            .checked(size == held)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                    this.write_facet(cx, move |facet| facet.tile = size)
                                })
                                .ok();
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// A grid has no column headers to click, so its order lives in a menu.
    fn grid_sort_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let held = self.sort().clone();
        let fields: Vec<(&'static str, &'static str)> = match self.filters().facet {
            Facet::Artists => vec![
                ("title", "sort_field_name"),
                ("tracks", "sort_field_tracks"),
                ("albums", "sort_field_albums"),
            ],
            Facet::Playlists => vec![
                ("title", "sort_field_name"),
                ("tracks", "sort_field_tracks"),
            ],
            _ => vec![
                ("title", "sort_field_title"),
                ("artist", "sort_field_artist"),
                ("year", "sort_field_year"),
            ],
        };
        // The listing holds one criterion; the shared control edits a stack, so it is a stack of one.
        let criteria: Vec<SortCriterion<&'static str>> = fields
            .iter()
            .find(|(id, _)| *id == held.field)
            .map(|(id, _)| {
                let direction = match held.descending {
                    true => SortDirection::Desc,
                    false => SortDirection::Asc,
                };
                vec![SortCriterion::new(*id, direction)]
            })
            .unwrap_or_default();
        let page = cx.entity().downgrade();
        SortControl::new("library-grid-sort", criteria)
            .fields(fields.into_iter().map(|(id, key)| (id, t(key))))
            .clearable(false)
            .on_change(move |criteria, cx| {
                let Some(first) = criteria.first() else {
                    return;
                };
                let next = store::layout::Sort {
                    field: first.field.to_string(),
                    descending: first.direction == SortDirection::Desc,
                };
                page.update(cx, |this, cx| {
                    this.write_facet(cx, move |facet| facet.sort = next)
                })
                .ok();
            })
            .into_any_element()
    }

    /// The pill over the listing while anything is picked. It never says nothing is.
    pub(crate) fn selection_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.selection.is_active() {
            return None;
        }
        let count = self.selection.count(self.total());
        if count == 0 {
            return None;
        }
        let target = self.pill_target();
        let run = cx.entity().downgrade();
        let cancel = cx.entity().downgrade();
        Some(
            commands::selection_bar(
                "library-selection",
                count,
                target,
                move |command, _, window, cx| {
                    run.update(cx, |this, cx| this.run_on_selection(command, window, cx))
                        .ok();
                },
                cx,
            )
            .on_cancel(move |_, cx| {
                cancel
                    .update(cx, |this, cx| {
                        this.selection.clear();
                        cx.notify();
                    })
                    .ok();
            })
            .into_any_element(),
        )
    }

    /// What the pill's labels describe; under select-all the keys are fetched when a verb is pressed.
    fn pill_target(&self) -> Target {
        if let Some(target) = self.picked_target() {
            return target;
        }
        Target::tracks(Vec::new(), t("library_whole_selection"))
    }

    /// The target for what was actually clicked, when the keys are already in hand.
    fn picked_target(&self) -> Option<Target> {
        if self.selection.is_everything() {
            return None;
        }
        let picked = self.selection.picked();
        if picked.is_empty() {
            return None;
        }
        if self.filters().facet == Facet::Tracks {
            let keys: Vec<SharedString> = picked
                .iter()
                .map(|key| SharedString::from(key.clone()))
                .collect();
            return Some(Target::selection(keys, self.first_title()));
        }
        if picked.len() == 1 {
            return self
                .tiles
                .iter()
                .find(|tile| tile.id.as_ref() == picked[0])
                .map(|tile| tile.target.clone());
        }
        None
    }

    /// The title of the row picked first, which names the whole selection.
    fn first_title(&self) -> String {
        self.selection
            .picked()
            .first()
            .and_then(|key| {
                self.rows
                    .iter()
                    .find(|row| &row.key == key)
                    .map(|row| row.title.clone())
            })
            .unwrap_or_default()
    }

    /// Run a verb on the selection, fetching the keys first when the selection is a filter.
    pub(crate) fn run_on_selection(
        &mut self,
        command: &'static Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.picked_target() {
            command.perform(target, window, cx);
            self.selection.clear();
            cx.notify();
            return;
        }
        let label = t("library_whole_selection");
        let resolve = self.resolve_selection(cx);
        self._resolve = Some(cx.spawn_in(window, async move |this, cx| {
            let keys = resolve.await;
            if keys.is_empty() {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                command.perform(Target::tracks(keys, label.clone()), window, cx);
                this.selection.clear();
                cx.notify();
            })
            .ok();
        }));
    }

    /// The keys a filter-scoped selection stands for, read once, at the moment they are needed.
    fn resolve_selection(
        &self,
        cx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Vec<SharedString>> + use<> {
        let filter = self.filters().track_filter(self.sort());
        let picked: Vec<String> = self.selection.picked().to_vec();
        let facet = self.filters().facet;
        let playlists: Vec<(String, Vec<String>)> = match facet {
            Facet::Playlists => store::PlaylistsModel::global(cx)
                .read(cx)
                .playlists()
                .iter()
                .map(|playlist| (playlist.id.clone(), playlist.track_keys.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let call = Link::global(cx).call(move |api| {
            let filter = filter.clone();
            let picked = picked.clone();
            let playlists = playlists.clone();
            async move { gather(api, facet, filter, picked, playlists).await }
        });
        async move {
            match call.await {
                Ok(keys) => keys.into_iter().map(SharedString::from).collect(),
                Err(error) => {
                    tracing::debug!(%error, "could not resolve the selection");
                    Vec::new()
                }
            }
        }
    }
}

/// Every key the selection stands for, whichever facet it was made on.
async fn gather(
    api: std::sync::Arc<dyn api::KopuzApi>,
    facet: Facet,
    filter: TrackFilter,
    picked: Vec<String>,
    playlists: Vec<(String, Vec<String>)>,
) -> Result<Vec<String>, api::ApiError> {
    if picked.is_empty() {
        // Select-all: the filtered listing entire, read once rather than held as keys.
        let page = api.tracks(filter, store::all()).await?;
        return Ok(page.items.into_iter().map(|track| track.key).collect());
    }
    let mut keys = Vec::new();
    for id in picked {
        match facet {
            Facet::Tracks => keys.push(id),
            Facet::Albums => {
                let page = api.album_tracks(id, store::all()).await?;
                keys.extend(page.items.into_iter().map(|track| track.key));
            }
            Facet::Artists => {
                let page = api.artist_tracks(id, store::all()).await?;
                keys.extend(page.items.into_iter().map(|track| track.key));
            }
            Facet::Playlists => {
                if let Some((_, held)) = playlists.iter().find(|(held, _)| held == &id) {
                    keys.extend(held.iter().cloned());
                }
            }
        }
    }
    Ok(keys)
}

fn chip_is_off(filters: &Filters, chip: Chip) -> bool {
    match chip {
        Chip::Genre => filters.genre.is_none(),
        Chip::Years => filters.years.is_none(),
        Chip::Folder => filters.folder.is_none(),
        Chip::Favorites => !filters.favorites,
        Chip::Downloaded => !filters.downloaded,
    }
}

fn apply_chip(filters: &mut Filters, chip: Chip, value: Option<String>) {
    match chip {
        Chip::Genre => filters.genre = value,
        Chip::Folder => filters.folder = value,
        Chip::Years => {
            filters.years = value.and_then(|value| {
                let (from, to) = value.split_once('-')?;
                Some((from.parse().ok()?, to.parse().ok()?))
            })
        }
        _ => {}
    }
}

/// The decades a library actually spans, so the menu never offers an empty one.
fn decades(span: Option<(u16, u16)>) -> Vec<(u16, u16)> {
    let Some((first, last)) = span else {
        return Vec::new();
    };
    let start = first / 10 * 10;
    let end = last / 10 * 10;
    (0..=((end.saturating_sub(start)) / 10))
        .map(|step| {
            let from = start + step * 10;
            (from, from + 9)
        })
        .collect()
}

/// The last path segment, so a chip reads "Music" rather than a whole home directory.
fn leaf_of(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// The folders the active source scans, which is what a folder chip can narrow to.
fn scan_roots(cx: &App) -> Vec<String> {
    store::SourcesModel::global(cx)
        .read(cx)
        .active()
        .and_then(|source| api::schema::spec_value(&source.settings, "directories"))
        .map(api::schema::decode_directories)
        .unwrap_or_default()
}

fn group_label_key(field: &str) -> &'static str {
    match field {
        "album" => "album",
        "artist" => "artist",
        "year" => "sort_field_year",
        _ => "library_group_none",
    }
}

fn density_label_key(density: Density) -> &'static str {
    match density {
        Density::Compact => "library_density_compact",
        Density::Comfortable => "library_density_comfortable",
    }
}

fn tile_label_key(size: TileSize) -> &'static str {
    match size {
        TileSize::Small => "library_tile_small",
        TileSize::Medium => "library_tile_medium",
        TileSize::Large => "library_tile_large",
    }
}

/// `AnyElement` has no builder tail; this keeps `chrome` reading as one expression.
trait Tap: Sized {
    fn tap(self, _: &mut Window) -> Self {
        self
    }
}

impl Tap for AnyElement {}

#[cfg(test)]
mod tests {
    use super::{decades, leaf_of};

    #[test]
    fn the_year_menu_offers_only_the_decades_the_library_spans() {
        assert_eq!(
            decades(Some((1983, 2004))),
            [(1980, 1989), (1990, 1999), (2000, 2009)]
        );
        assert!(decades(None).is_empty());
    }

    #[test]
    fn a_folder_chip_reads_as_its_last_segment() {
        assert_eq!(leaf_of("/home/someone/Music/"), "Music");
        assert_eq!(leaf_of("Music"), "Music");
    }
}
