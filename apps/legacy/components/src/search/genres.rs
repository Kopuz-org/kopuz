use dioxus::prelude::*;

#[component]
pub fn SearchGenres(
    genres: Vec<(String, Option<utils::CoverUrl>)>,
    on_select_genre: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "mt-12",
            h2 { class: "text-xl font-semibold text-white/80 mb-4", "{i18n::t(\"browse_genres\")}" }
            if genres.is_empty() {
                p { class: "text-slate-500 italic", "{i18n::t(\"no_genres_found\")}" }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-4 gap-4",
                    for (genre, cover_url) in genres {
                        div {
                            key: "{genre}",
                            class: "aspect-video bg-white/5 border border-white/10 rounded-xl p-4 cursor-pointer hover:bg-white/10 transition-all flex items-end relative overflow-hidden group content-visibility-auto",
                            onclick: {
                                let genre = genre.clone();
                                move |_| on_select_genre.call(genre.clone())
                            },
                            if let Some(url) = cover_url {
                                img {
                                    src: "{url}",
                                    class: "absolute inset-0 w-full h-full object-cover group-hover:scale-110 transition-transform duration-500 will-change-transform",
                                    loading: "lazy",
                                    decoding: "async",
                                }
                                div { class: "absolute inset-0 bg-gradient-to-t from-black/80 via-transparent to-transparent" }
                            }
                            span { class: "text-lg font-bold text-white relative z-10", "{genre}" }
                        }
                    }
                }
            }
        }
    }
}
