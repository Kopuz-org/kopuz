use api::{ListeningRecap, RecapEntry, RecapPeak, RecapPeriod, RecapUnit};
use components::stat_card::StatCard;
use dioxus::prelude::*;
use hooks::use_db_queries::{use_active_source, use_listening_recap};

fn period_label(period: RecapPeriod) -> String {
    i18n::t(match period {
        RecapPeriod::Day => "recap_period_day",
        RecapPeriod::Week => "recap_period_week",
        RecapPeriod::Month => "recap_period_month",
        RecapPeriod::Year => "recap_period_year",
        RecapPeriod::AllTime => "recap_period_all_time",
    })
}

/// The busiest stretch's label and value, in the unit its period counts in.
fn peak_text(peak: &RecapPeak) -> (String, String) {
    let month = || i18n::t_with("recap_month", &[("month", peak.month.into())]);
    match peak.unit {
        RecapUnit::Hour => (
            i18n::t("recap_busiest_hour"),
            i18n::t_with(
                "recap_peak_hour",
                &[("hour", format!("{:02}", peak.hour).into())],
            ),
        ),
        RecapUnit::Day => (
            i18n::t("recap_busiest_day"),
            i18n::t_with(
                "recap_peak_day",
                &[("month", month().into()), ("day", peak.day.into())],
            ),
        ),
        RecapUnit::Month => (
            i18n::t("recap_busiest_month"),
            i18n::t_with(
                "recap_peak_month",
                &[
                    ("month", month().into()),
                    ("year", peak.year.to_string().into()),
                ],
            ),
        ),
    }
}

/// The listening recap over a period the listener picks, ending now.
#[component]
pub fn ListeningRecapView() -> Element {
    let source = use_active_source();
    let period = use_signal(RecapPeriod::default);
    let recap = use_listening_recap(source, period);

    let body = match recap.read().clone() {
        None => rsx! {
            div { class: "flex items-center justify-center py-12",
                i { class: "fa-solid fa-spinner fa-spin text-3xl text-white/20" }
            }
        },
        Some(Err(error)) => rsx! {
            p { class: "text-slate-400 text-sm py-12 text-center", "{hooks::toast::describe(&error)}" }
        },
        Some(Ok(recap)) => rsx! { RecapBody { recap } },
    };
    rsx! {
        div { class: "flex-1 min-h-0 overflow-y-auto pb-32",
            PeriodPicker { period }
            {body}
        }
    }
}

#[component]
fn PeriodPicker(period: Signal<RecapPeriod>) -> Element {
    rsx! {
        div { class: "flex flex-wrap gap-1 mb-6 p-1 rounded-full bg-white/5 border border-white/10 w-fit",
            for choice in RecapPeriod::ALL {
                button {
                    key: "{choice:?}",
                    class: if period() == choice { "px-3 py-1 rounded-full text-xs font-medium bg-white/10 text-white" } else { "px-3 py-1 rounded-full text-xs font-medium text-slate-400 hover:text-white transition-colors" },
                    onclick: move |_| period.set(choice),
                    "{period_label(choice)}"
                }
            }
        }
    }
}

#[component]
fn RecapBody(recap: ListeningRecap) -> Element {
    if recap.plays == 0 {
        return rsx! {
            div { class: "flex flex-col items-center justify-center py-24 text-slate-500",
                i { class: "fa-solid fa-chart-simple text-4xl mb-4 opacity-50" }
                p { class: "max-w-md text-center", "{i18n::t(\"recap_empty\")}" }
            }
        };
    }

    let minutes = recap.listened_ms / 60_000;
    let (peak_label, peak_value) = recap.peak.as_ref().map(peak_text).unwrap_or_default();
    let streak = i18n::t_with(
        "recap_streak_days",
        &[("count", recap.longest_streak_days.into())],
    );
    rsx! {
        div { class: "grid grid-cols-2 lg:grid-cols-4 gap-4 mb-4",
            StatCard { label: i18n::t("recap_minutes_listened"), value: "{minutes}", icon: "fa-clock" }
            StatCard { label: i18n::t("plays"), value: "{recap.plays}", icon: "fa-play" }
            StatCard { label: peak_label, value: peak_value, icon: "fa-chart-simple" }
            StatCard { label: i18n::t("recap_streak"), value: streak, icon: "fa-fire" }
        }
        div { class: "grid grid-cols-3 gap-4 mb-8",
            StatCard { label: i18n::t("tracks"), value: "{recap.tracks}", icon: "fa-music" }
            StatCard { label: i18n::t("artists"), value: "{recap.artists}", icon: "fa-user" }
            StatCard { label: i18n::t("albums"), value: "{recap.albums}", icon: "fa-compact-disc" }
        }
        div { class: "grid grid-cols-1 lg:grid-cols-3 gap-4",
            TopList { title: i18n::t("recap_top_tracks"), entries: recap.top_tracks.clone(), icon: "fa-music", round: false }
            TopList { title: i18n::t("recap_top_artists"), entries: recap.top_artists.clone(), icon: "fa-user", round: true }
            TopList { title: i18n::t("recap_top_albums"), entries: recap.top_albums.clone(), icon: "fa-compact-disc", round: false }
        }
    }
}

#[component]
fn TopList(title: String, entries: Vec<RecapEntry>, icon: &'static str, round: bool) -> Element {
    let shape = if round { "rounded-full" } else { "rounded-md" };
    rsx! {
        div { class: "border border-white/5 p-5 rounded-xl",
            h3 { class: "text-sm font-semibold text-slate-400 mb-4", "{title}" }
            ol { class: "flex flex-col gap-3",
                for (rank, entry) in entries.into_iter().enumerate() {
                    {
                        let cover = hooks::artwork::url(entry.artwork.as_ref(), hooks::artwork::Size::Thumb);
                        let plays = i18n::t_with("home_plays", &[("count", entry.plays.into())]);
                        let id = entry.key.clone().unwrap_or_else(|| format!("{}\u{1f}{}", entry.title, entry.subtitle));
                        rsx! {
                            li { key: "{id}", class: "flex items-center gap-3 min-w-0",
                                span { class: "w-5 shrink-0 text-right tabular-nums text-white/50 font-medium", "{rank + 1}" }
                                div { class: "w-10 h-10 {shape} bg-white/5 flex items-center justify-center shrink-0 overflow-hidden text-slate-500",
                                    if let Some(url) = cover {
                                        img { src: "{url.as_ref()}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                    } else {
                                        i { class: "fa-solid {icon} text-xs" }
                                    }
                                }
                                div { class: "flex-1 min-w-0",
                                    div { class: "text-white font-medium truncate text-sm", "{entry.title}" }
                                    if !entry.subtitle.is_empty() {
                                        div { class: "text-slate-400 text-xs truncate", "{entry.subtitle}" }
                                    }
                                }
                                span { class: "shrink-0 text-slate-400 text-xs tabular-nums", "{plays}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
