//! An example integration: asks an address for a command every half second
//! and carries it out.

use kopuz_plugin_sdk::SourceError;
use kopuz_plugin_sdk::http::Client;
use kopuz_plugin_sdk::integration::library;
use kopuz_plugin_sdk::integration::playback::{self, Command, QueueMode};
use kopuz_plugin_sdk::integration::{
    ActionTarget, Block, Integration, PanelAction, PanelTarget, Settings,
};

pub struct Remote {
    http: Client,
    url: String,
}

/// One line of the address's answer, as what to do with it.
enum Order {
    Command(Command),
    /// A track key, which may hold spaces, so it is the rest of its line.
    Queue(String, QueueMode),
    /// Words to search the library for, whose tracks then play.
    Find(String),
}

/// How many of a search's tracks are queued.
const FOUND: u32 = 50;

fn order(line: &str) -> Option<Order> {
    let line = line.trim();
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    let word = word.to_ascii_lowercase();
    let number = || rest.parse::<f64>().ok();
    Some(Order::Command(match word.as_str() {
        "play" if !rest.is_empty() => return Some(Order::Queue(rest.into(), QueueMode::Replace)),
        "queue" if !rest.is_empty() => return Some(Order::Queue(rest.into(), QueueMode::Append)),
        "find" if !rest.is_empty() => return Some(Order::Find(rest.into())),
        "play" => Command::Play,
        "pause" => Command::Pause,
        "toggle" => Command::Toggle,
        "next" => Command::Next,
        "previous" => Command::Previous,
        "stop" => Command::Stop,
        "seek" => Command::Seek((number()? * 1000.0) as u64),
        "volume" => Command::Volume((number()? / 100.0).clamp(0.0, 1.0) as f32),
        _ => return None,
    }))
}

impl Integration for Remote {
    fn open(settings: Settings) -> Self {
        Remote {
            http: Client::new(),
            url: settings.get("url").unwrap_or_default().to_string(),
        }
    }

    fn validate(&self) -> Result<(), SourceError> {
        let response = self.http.get(&self.url).send()?;
        match response.is_success() {
            true => Ok(()),
            false => Err(SourceError::from_status(response.status(), "the remote")),
        }
    }

    fn on_tick(&self) {
        let Ok(response) = self.http.get(&self.url).send() else {
            return;
        };
        if !response.is_success() {
            return;
        }
        for line in response.text().lines() {
            let done = match order(line) {
                Some(Order::Command(command)) => playback::control(command),
                Some(Order::Queue(key, mode)) => playback::queue(&[key], mode),
                Some(Order::Find(words)) => find(&words),
                None if line.trim().is_empty() => Ok(()),
                None => Err(format!("not a command: {line}")),
            };
            if let Err(error) = done {
                kopuz_plugin_sdk::warn!("the remote's command failed: {error}");
            }
        }
    }

    fn run_action(
        &self,
        id: &str,
        target: ActionTarget,
        input: Option<String>,
    ) -> Result<Option<String>, String> {
        match (id, target) {
            ("find", _) => {
                let words = input.filter(|words| !words.trim().is_empty());
                let words = words.ok_or("type something to search for")?;
                find(&words)?;
                Ok(Some(format!("Playing what the library has for {words}")))
            }
            ("more-by-artist", target) => {
                let artist = artist_of(&target)?;
                find(&artist)?;
                Ok(Some(format!("Playing more by {artist}")))
            }
            _ => Err(format!("the remote has no action {id} for that")),
        }
    }

    fn render_panel(&self, id: &str, target: PanelTarget) -> Result<Vec<Block>, String> {
        if id != "more-by-artist" {
            return Err(format!("the remote has no panel {id}"));
        }
        let (target, own) = match target {
            PanelTarget::Album(album) => {
                let own: Vec<String> = library::album_tracks(&album)?
                    .into_iter()
                    .map(|track| track.key)
                    .collect();
                (ActionTarget::Album(album), own)
            }
            PanelTarget::Artist(artist) => (ActionTarget::Artist(artist), Vec::new()),
        };
        let artist = artist_of(&target)?;
        let keys: Vec<String> = library::search(&artist, FOUND)?
            .into_iter()
            .map(|track| track.key)
            .filter(|key| !own.contains(key))
            .take(10)
            .collect();
        let play = Block::Action(PanelAction {
            label: "Play everything by them".into(),
            action: "more-by-artist".into(),
        });
        if keys.is_empty() {
            return Ok(vec![
                Block::Text(format!("Nothing else by {artist} in the library yet.")),
                play,
            ]);
        }
        Ok(vec![
            Block::Text(format!("What else the library has by {artist}")),
            Block::Tracks(keys),
            play,
        ])
    }
}

/// The artist a track, an album or an artist page is by.
fn artist_of(target: &ActionTarget) -> Result<String, String> {
    let first = match target {
        ActionTarget::Tracks(keys) => library::tracks(&keys[..keys.len().min(1)])?,
        ActionTarget::Album(album) => library::album_tracks(album)?,
        ActionTarget::Artist(artist) => library::artist_tracks(artist)?,
        ActionTarget::None => return Err("nothing was picked".into()),
    };
    first
        .first()
        .map(|track| track.artist.clone())
        .filter(|artist| !artist.is_empty())
        .ok_or_else(|| "the track has no artist".into())
}

/// Play what a library search for `words` finds.
fn find(words: &str) -> Result<(), String> {
    let keys: Vec<String> = library::search(words, FOUND)?
        .into_iter()
        .map(|track| track.key)
        .collect();
    if keys.is_empty() {
        return Err(format!("the library has nothing for {words}"));
    }
    playback::queue(&keys, QueueMode::Replace)
}

kopuz_plugin_sdk::export_integration!(Remote);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_as_a_command() {
        assert!(matches!(order("next"), Some(Order::Command(Command::Next))));
        assert!(matches!(
            order("  Pause "),
            Some(Order::Command(Command::Pause))
        ));
        assert!(matches!(
            order("seek 30"),
            Some(Order::Command(Command::Seek(30_000)))
        ));
        assert!(matches!(
            order("volume 50"),
            Some(Order::Command(Command::Volume(volume))) if (volume - 0.5).abs() < 1e-6
        ));
        assert!(matches!(
            order("play /music/Kind of Blue/01.flac"),
            Some(Order::Queue(key, QueueMode::Replace)) if key == "/music/Kind of Blue/01.flac"
        ));
        assert!(matches!(
            order("queue b"),
            Some(Order::Queue(key, QueueMode::Append)) if key == "b"
        ));
        assert!(
            matches!(order("find blue in green"), Some(Order::Find(words)) if words == "blue in green")
        );
        assert!(order("find").is_none());
        assert!(order("seek soon").is_none());
        assert!(order("dance").is_none());
        assert!(order("").is_none());
    }
}
