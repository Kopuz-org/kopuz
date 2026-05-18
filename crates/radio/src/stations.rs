pub fn stream_url(station_id: &str, stream_id: &str) -> &'static str {
    match station_id {
        "listen_moe" => {
            if stream_id.contains("kpop") {
                "https://listen.moe/kpop/stream"
            } else {
                "https://listen.moe/stream"
            }
        }
        "j1" => {
            if stream_id == "J1GOLD" {
                "https://jenny.torontocast.com:2000/stream/J1GOLD"
            } else {
                "https://jenny.torontocast.com:2000/stream/J1HITS"
            }
        }
        "doujinstyle" => "https://streams.radio.co/s5ff57669c/listen",
        "vocaloid" => "https://vocaloid.radioca.st/stream",
        "asiadreamradio" => match stream_id {
            "japan_hits" => "https://quincy.torontocast.com:2020/;?type=http",
            "natsukashii" => "https://quincy.torontocast.com:2070/;?type=http",
            "jpop_kawaii" => "https://kathy.torontocast.com:3060/;?type=http",
            "jpop_power" => "https://kathy.torontocast.com:3560/;?type=http",
            "jazz_sakura" => "https://kathy.torontocast.com:3330/;?type=http",
            "jrock" => "https://kathy.torontocast.com:3340/;?type=http",
            "jclub_hiphop" => "https://kathy.torontocast.com:3350/;?type=http",
            "bandstand_jazz" => "https://cast1.torontocast.com/bandstand",
            _ => "",
        },
        _ => "",
    }
}
