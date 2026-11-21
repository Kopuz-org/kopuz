//! Android Auto's side of the JNI bridge. `KopuzBrowserService` asks for the
//! rows of a browse screen, a play by media id or a voice search; the frontend
//! registers the handler that answers from the library, since only it holds
//! the API handle.

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Longer than any library read, short enough that a hung answer leaves the car
/// with an empty list instead of a spinner that never ends.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

const FIELD_SEP: char = '\u{1f}';
const RECORD_SEP: char = '\u{1e}';

/// One row of a car browse screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub browsable: bool,
    pub playable: bool,
}

pub enum CarRequest {
    Children {
        parent: String,
        reply: Sender<Vec<BrowseItem>>,
    },
    Play {
        media_id: String,
    },
    PlaySearch {
        query: String,
    },
}

type CarHandler = Box<dyn Fn(CarRequest) + Send + Sync>;

fn handler() -> &'static Mutex<Option<CarHandler>> {
    static HANDLER: OnceLock<Mutex<Option<CarHandler>>> = OnceLock::new();
    HANDLER.get_or_init(|| Mutex::new(None))
}

/// Register what answers the car. Until this runs the browse service reports
/// the app as not ready.
pub fn set_car_handler(handler_fn: impl Fn(CarRequest) + Send + Sync + 'static) {
    let mut guard = handler().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Box::new(handler_fn));
}

/// False when nothing is registered yet.
fn send(request: CarRequest) -> bool {
    let sent = match handler().lock() {
        Ok(guard) => match guard.as_ref() {
            Some(handle) => {
                handle(request);
                true
            }
            None => false,
        },
        Err(_) => false,
    };
    if sent {
        super::android::wake_tokio();
        super::android::wake_run_loop();
    }
    sent
}

fn ask(make: impl FnOnce(Sender<Vec<BrowseItem>>) -> CarRequest) -> Option<Vec<BrowseItem>> {
    let (reply, answer) = channel();
    if !send(make(reply)) {
        return None;
    }
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(items) => Some(items),
        Err(error) => {
            tracing::warn!(%error, "car browse request got no answer");
            Some(Vec::new())
        }
    }
}

fn clean(text: &str) -> String {
    text.replace([FIELD_SEP, RECORD_SEP], " ")
}

/// Rows as plain text: fields split by US, records by RS, so the Kotlin side
/// needs no JSON parser.
fn encode(items: &[BrowseItem]) -> String {
    items
        .iter()
        .map(|item| {
            let flags = match (item.browsable, item.playable) {
                (true, true) => "bp",
                (true, false) => "b",
                (false, true) => "p",
                (false, false) => "",
            };
            [
                clean(&item.id),
                clean(&item.title),
                clean(&item.subtitle),
                flags.to_owned(),
            ]
            .join(&FIELD_SEP.to_string())
        })
        .collect::<Vec<_>>()
        .join(&RECORD_SEP.to_string())
}

fn read(env: &mut JNIEnv, text: &JString) -> Option<String> {
    env.get_string(text).ok().map(Into::into)
}

/// `null` tells the service the app is not up yet.
fn answer(env: &mut JNIEnv, items: Option<Vec<BrowseItem>>) -> jstring {
    let Some(items) = items else {
        return std::ptr::null_mut();
    };
    match env.new_string(encode(&items)) {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_moe_kopuz_kopuz_KopuzBrowserService_nativeChildren(
    mut env: JNIEnv,
    _class: JClass,
    parent: JString,
) -> jstring {
    let Some(parent) = read(&mut env, &parent) else {
        return std::ptr::null_mut();
    };
    let items = ask(|reply| CarRequest::Children { parent, reply });
    answer(&mut env, items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_moe_kopuz_kopuz_KopuzBrowserService_nativePlay(
    mut env: JNIEnv,
    _class: JClass,
    media_id: JString,
) -> jboolean {
    let Some(media_id) = read(&mut env, &media_id) else {
        return 0;
    };
    send(CarRequest::Play { media_id }).into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_moe_kopuz_kopuz_KopuzBrowserService_nativePlaySearch(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
) -> jboolean {
    let Some(query) = read(&mut env, &query) else {
        return 0;
    };
    send(CarRequest::PlaySearch { query }).into()
}
