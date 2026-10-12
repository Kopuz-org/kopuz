use crate::{Browser, MusicServer, MusicService};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAuth {
    Token {
        token: Option<String>,
        user_id: Option<String>,
    },
    Browser {
        browser: Option<Browser>,
        token: Option<String>,
        user_id: Option<String>,
        anonymous: bool,
    },
}

impl MusicServer {
    pub fn auth(&self) -> ServerAuth {
        if self.service == MusicService::YtMusic || self.service == MusicService::SoundCloud {
            ServerAuth::Browser {
                browser: self.yt_browser,
                token: self.access_token.clone(),
                user_id: self.user_id.clone(),
                anonymous: self.yt_anonymous,
            }
        } else {
            ServerAuth::Token {
                token: self.access_token.clone(),
                user_id: self.user_id.clone(),
            }
        }
    }
}
