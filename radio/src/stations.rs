pub fn stream_url(station_id: &str, stream_id: &str) -> &'static str {
    match station_id {
        "listen_moe" => if stream_id.contains("kpop") {
            "https://listen.moe/kpop/stream"
        } else {
            "https://listen.moe/stream"
        },
        "j1" => if stream_id == "J1GOLD" {
            "https://jenny.torontocast.com:2000/stream/J1GOLD"
        } else {
            "https://jenny.torontocast.com:2000/stream/J1HITS"
        },
        "doujinstyle" => "https://streams.radio.co/s5ff57669c/listen",
        "vocaloid"    => "https://vocaloid.radioca.st/stream",
        _             => "",
    }
}
