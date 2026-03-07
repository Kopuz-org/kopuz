pub mod cover_art;

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{self, Assets, Timestamps},
};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Presence {
    client: Mutex<DiscordIpcClient>,
}

impl Presence {
    pub fn new(client_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut client = DiscordIpcClient::new(client_id);
        client.connect()?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }

    pub fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.client.lock().unwrap().close()?;
        Ok(())
    }

    pub fn set_now_playing(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        elapsed_secs: u64,
        duration_secs: u64,
        cover_url: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let start_time = now - elapsed_secs as i64;
        let end_time = start_time + duration_secs as i64;

        let timestamps = Timestamps::new().start(start_time).end(end_time);

        let state = format!("by {artist}");

        let mut activity = activity::Activity::new()
            .details(title)
            .state(&state)
            .timestamps(timestamps)
            .activity_type(activity::ActivityType::Listening);

        if let Some(url) = cover_url {
            let assets = Assets::new().large_image(url).large_text(album);
            activity = activity.assets(assets);
        }

        self.client.lock().unwrap().set_activity(activity)?;
        Ok(())
    }

    pub fn set_paused(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let state = format!("by {artist} • Paused");
        let mut activity = activity::Activity::new()
            .details(title)
            .state(&state)
            .activity_type(activity::ActivityType::Listening);

        if let Some(url) = cover_url {
            let assets = Assets::new().large_image(url).large_text(album);
            activity = activity.assets(assets);
        }

        self.client.lock().unwrap().set_activity(activity)?;
        Ok(())
    }

    pub fn clear_activity(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.client.lock().unwrap().clear_activity()?;
        Ok(())
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}
