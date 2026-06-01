use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// Set from the JNI thread when the hardware/gesture back is pressed; drained on the
// runtime by take_back_pressed(). Decoupled because dioxus signals can only be touched
// from the runtime thread, not the JNI thread.
static BACK_PENDING: AtomicBool = AtomicBool::new(false);

/// Returns true once per back press, clearing the pending flag.
pub fn take_back_pressed() -> bool {
    BACK_PENDING.swap(false, Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Stop,
}

static JVM: OnceLock<JavaVM> = OnceLock::new();
// App classloader cached from main thread so FindClass works from any thread.
static CLASSLOADER: OnceLock<GlobalRef> = OnceLock::new();
static BACKGROUND_HANDLER: OnceLock<Arc<Mutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>>> =
    OnceLock::new();

fn get_bg_handler() -> Arc<Mutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>> {
    BACKGROUND_HANDLER
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

pub fn set_background_handler(handler: impl Fn(SystemEvent) + Send + Sync + 'static) {
    let binding = get_bg_handler();
    let mut guard = binding.lock().unwrap();
    *guard = Some(Box::new(handler));
}

fn dispatch_event(event: SystemEvent) {
    if let Ok(guard) = get_bg_handler().lock() {
        if let Some(ref handler) = *guard {
            handler(event);
        }
    }
}

pub fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let ctx = ndk_context::android_context();
        let vm_ptr = ctx.vm();
        if vm_ptr.is_null() {
            return;
        }
        match unsafe { JavaVM::from_raw(vm_ptr.cast()) } {
            Ok(vm) => {
                let _ = JVM.set(vm);
                cache_classloader();
                init_media_session();
            }
            Err(e) => eprintln!("[android] Failed to capture JVM: {}", e),
        }
    });
}

// Cache the app classloader from the activity so FindClass works from background threads.
fn cache_classloader() {
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let ctx = ndk_context::android_context();
    let raw = ctx.context();
    if raw.is_null() {
        eprintln!("[android] null activity context; skipping classloader cache");
        return;
    }
    // Transient local only — we immediately turn the resolved classloader into a
    // GlobalRef below and never retain this raw activity pointer.
    let activity = unsafe { JObject::from_raw(raw.cast()) };
    let result: Result<(), jni::errors::Error> = (|| {
        let cl = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let global = env.new_global_ref(&cl)?;
        let _ = CLASSLOADER.set(global);
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[android] Failed to cache classloader: {}", e);
    }
}

// Resolve an app class using the cached classloader, falling back to FindClass.
fn find_app_class<'a>(env: &mut JNIEnv<'a>, name: &str) -> Result<JClass<'a>, jni::errors::Error> {
    if let Some(cl) = CLASSLOADER.get() {
        let dot_name = env.new_string(name.replace('/', "."))?;
        let class_obj = env
            .call_method(
                cl.as_obj(),
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&dot_name)],
            )?
            .l()?;
        Ok(JClass::from(class_obj))
    } else {
        env.find_class(name)
    }
}

fn init_media_session() {
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[android] attach_current_thread failed: {}", e);
            return;
        }
    };
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result: Result<(), jni::errors::Error> = (|| {
        let class = find_app_class(&mut env, "com/temidaradev/kopuz/MediaSessionHelper")?;
        env.call_static_method(
            &class,
            "init",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&activity)],
        )?
        .v()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[android] MediaSessionHelper.init failed: {}", e);
        clear_jni_exception(&mut env);
    }
}

fn dir_via_jni(method: &str) -> Option<String> {
    init();
    let vm = JVM.get()?;
    let mut env = vm.attach_current_thread().ok()?;
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let r: Result<String, jni::errors::Error> = (|| {
        let file = env
            .call_method(&activity, method, "()Ljava/io/File;", &[])?
            .l()?;
        let path = env
            .call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        Ok(env.get_string(&JString::from(path))?.into())
    })();
    r.map_err(|_| {
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
    })
    .ok()
}

pub fn get_files_dir() -> Option<String> {
    dir_via_jni("getFilesDir").or_else(|| {
        std::env::var("FILES_DIR").ok().or_else(|| {
            let home = std::env::var("HOME").ok()?;
            if home.contains("com.temidaradev.kopuz") {
                Some(format!("{}/files", home))
            } else {
                None
            }
        })
    })
}

pub fn get_android_music_dir() -> Option<String> {
    init();
    let vm = JVM.get()?;
    let mut env = vm.attach_current_thread().ok()?;
    let result: Result<String, jni::errors::Error> = (|env: &mut JNIEnv| {
        let env_class = env.find_class("android/os/Environment")?;
        let dir_type = env.new_string("Music")?;
        let file = env
            .call_static_method(
                env_class,
                "getExternalStoragePublicDirectory",
                "(Ljava/lang/String;)Ljava/io/File;",
                &[JValue::Object(&dir_type)],
            )?
            .l()?;
        let path = env
            .call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        Ok(env.get_string(&JString::from(path))?.into())
    })(&mut env);
    if let Err(e) = result {
        eprintln!("[android] get_android_music_dir failed: {}", e);
        clear_jni_exception(&mut env);
        None
    } else {
        result.ok()
    }
}

/// Normalises an artwork URL to something Kotlin can consume:
/// - `artwork://local?p=…` → decoded absolute file path
/// - `http(s)://…`         → passed through as-is for Kotlin to download
/// - anything else         → None
fn normalize_artwork(url: &str) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(url.to_string());
    }
    let query = url.strip_prefix("artwork://local?")?;
    let encoded = query.split('&').find_map(|kv| {
        let mut parts = kv.splitn(2, '=');
        if parts.next() == Some("p") {
            parts.next()
        } else {
            None
        }
    })?;
    let decoded = percent_decode(encoded);
    let path = if decoded.starts_with("/~") {
        std::env::var("HOME")
            .ok()
            .map(|h| decoded.replacen("/~", &h, 1))
            .unwrap_or(decoded)
    } else if decoded.starts_with('~') {
        std::env::var("HOME")
            .ok()
            .map(|h| decoded.replacen('~', &h, 1))
            .unwrap_or(decoded)
    } else {
        decoded
    };
    Some(path)
}

/// Cache of the last decoded `data:` artwork: (content hash, written file path).
/// The player re-sends the same artwork every position tick (~1s); without this
/// we'd base64-decode and rewrite the file each tick.
static LAST_DATA_ART: Mutex<Option<(u64, String)>> = Mutex::new(None);

/// Resolve an artwork URL to something `MediaSessionHelper` can load: an http(s)
/// URL, a local file path, or — for the base64 `data:` URLs the Android UI uses —
/// a file decoded into app storage (the notification can't render a data URL).
fn resolve_artwork(url: &str) -> Option<String> {
    let resolved = if url.starts_with("data:") {
        data_url_to_file(url)
    } else if let Some(stripped) = url.strip_prefix("file://") {
        Some(stripped.to_string())
    } else if url.starts_with('/') {
        // Bare absolute path (e.g. a downloaded server cover in the temp dir).
        Some(url.to_string())
    } else {
        normalize_artwork(url)
    };
    eprintln!(
        "[android] resolve_artwork in={} -> {:?}",
        &url[..url.len().min(48)],
        resolved
    );
    resolved
}

/// Decode a `data:<mime>;base64,<payload>` URL to a file under the app's files dir
/// and return its path. Cached by content hash so repeated identical updates reuse
/// the same file instead of rewriting it.
fn data_url_to_file(url: &str) -> Option<String> {
    use base64::{Engine as _, engine::general_purpose};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let meta = url.strip_prefix("data:")?;
    let comma = meta.find(',')?;
    let header = &meta[..comma];
    let payload = &meta[comma + 1..];
    if !header.contains("base64") {
        return None;
    }
    let ext = if header.contains("image/png") {
        "png"
    } else if header.contains("image/webp") {
        "webp"
    } else if header.contains("image/gif") {
        "gif"
    } else {
        "jpg"
    };

    let mut hasher = DefaultHasher::new();
    payload.hash(&mut hasher);
    let hash = hasher.finish();

    // Hash is part of the filename so a new track yields a new path — the Kotlin
    // side caches its decoded bitmap by path and would otherwise keep showing the
    // previous track's art when the filename stayed constant.
    if let Ok(guard) = LAST_DATA_ART.lock() {
        if let Some((last_hash, path)) = guard.as_ref() {
            if *last_hash == hash && std::path::Path::new(path).exists() {
                return Some(path.clone());
            }
        }
    }

    let files_dir = get_files_dir()?;
    let path = format!("{files_dir}/np_art_{hash}.{ext}");
    let bytes = general_purpose::STANDARD.decode(payload).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    if let Ok(mut guard) = LAST_DATA_ART.lock() {
        // Remove the previously written art file so they don't accumulate.
        if let Some((_, old_path)) = guard.as_ref() {
            if old_path != &path {
                let _ = std::fs::remove_file(old_path);
            }
        }
        *guard = Some((hash, path.clone()));
    }
    Some(path)
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.bytes().peekable();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next().map(hex_val).unwrap_or(0);
            let h2 = chars.next().map(hex_val).unwrap_or(0);
            out.push(char::from(h1 << 4 | h2));
        } else if b == b'+' {
            out.push(' ');
        } else {
            out.push(char::from(b));
        }
    }
    out
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
    position: f64,
    playing: bool,
    artwork_path: Option<&str>,
) {
    init();
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let duration_ms = (duration * 1000.0) as i64;
    let position_ms = (position * 1000.0) as i64;
    let resolved_art = artwork_path.and_then(resolve_artwork);
    let result: Result<(), jni::errors::Error> = (|| {
        let class = find_app_class(&mut env, "com/temidaradev/kopuz/MediaSessionHelper")?;
        let j_title = env.new_string(title)?;
        let j_artist = env.new_string(artist)?;
        let j_album = env.new_string(album)?;
        let null_obj = JObject::null();
        let j_art_owned;
        let j_art: &JObject = if let Some(ref path) = resolved_art {
            j_art_owned = env.new_string(path)?;
            &*j_art_owned
        } else {
            &null_obj
        };
        env.call_static_method(
            &class,
            "updateNowPlaying",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZLjava/lang/String;)V",
            &[
                JValue::Object(&activity),
                JValue::Object(&j_title),
                JValue::Object(&j_artist),
                JValue::Object(&j_album),
                JValue::Long(duration_ms),
                JValue::Long(position_ms),
                JValue::Bool(playing as u8),
                JValue::Object(j_art),
            ],
        )?
        .v()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!(
            "[android] MediaSessionHelper.updateNowPlaying failed: {}",
            e
        );
        clear_jni_exception(&mut env);
    }
}

pub fn wake_run_loop() {
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let result: Result<(), jni::errors::Error> = (|| {
        let class = find_app_class(&mut env, "com/temidaradev/kopuz/MediaSessionHelper")?;
        env.call_static_method(&class, "wakeMainThread", "()V", &[])?
            .v()?;
        Ok(())
    })();
    if let Err(_) = result {
        clear_jni_exception(&mut env);
    }
}

pub fn stop_session() {
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result: Result<(), jni::errors::Error> = (|| {
        let class = find_app_class(&mut env, "com/temidaradev/kopuz/MediaSessionHelper")?;
        env.call_static_method(
            &class,
            "stopSession",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&activity)],
        )?
        .v()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[android] MediaSessionHelper.stopSession failed: {}", e);
        clear_jni_exception(&mut env);
    }
}

pub fn request_permissions() {
    init();
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let ctx = ndk_context::android_context();
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result: Result<(), jni::errors::Error> = (|env: &mut JNIEnv| {
        let class = find_app_class(env, "com/temidaradev/kopuz/MediaSessionHelper")?;
        env.call_static_method(
            &class,
            "requestPermissions",
            "(Landroid/app/Activity;)V",
            &[JValue::Object(&activity)],
        )?
        .v()?;
        Ok(())
    })(&mut env);
    if let Err(e) = result {
        eprintln!(
            "[android] MediaSessionHelper.requestPermissions failed: {}",
            e
        );
        clear_jni_exception(&mut env);
    }
}

fn clear_jni_exception(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

// Called from Kotlin: MediaReceiver.nativeOnAction(String) — routes notification button taps to Rust
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_temidaradev_kopuz_MediaReceiver_nativeOnAction(
    mut env: JNIEnv,
    _class: JClass,
    action: JString,
) {
    let action_str: String = match env.get_string(&action) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    match action_str.as_str() {
        "play" => dispatch_event(SystemEvent::Play),
        "pause" => dispatch_event(SystemEvent::Pause),
        "toggle" => dispatch_event(SystemEvent::Toggle),
        "next" => dispatch_event(SystemEvent::Next),
        "prev" => dispatch_event(SystemEvent::Prev),
        "stop" => dispatch_event(SystemEvent::Stop),
        // Hardware/gesture back — handled by the app router, not a media command.
        "back" => {
            BACK_PENDING.store(true, Ordering::SeqCst);
            super::back_wake();
        }
        _ => {}
    }
}

/// Send the app to the background (like Home) instead of finishing it, so playback
/// survives. Delegates to MainActivity.moveToBack(), which marshals onto the UI thread.
pub fn move_task_to_back() {
    let vm = match JVM.get() {
        Some(v) => v,
        None => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let result: Result<(), jni::errors::Error> = (|| {
        let class = find_app_class(&mut env, "dev/dioxus/main/MainActivity")?;
        env.call_static_method(&class, "moveToBack", "()V", &[])?
            .v()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[android] MainActivity.moveToBack failed: {}", e);
        clear_jni_exception(&mut env);
    }
}
