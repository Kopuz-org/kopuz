package com.temidaradev.kopuz

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Build

object MediaSessionHelper {
    private var session: MediaSession? = null
    private const val CHANNEL_ID = "kopuz_playback"
    const val NOTIF_ID = 42

    @Volatile var pendingNotification: Notification? = null

    // Last reported playing state — read by the focus listener to decide whether a
    // transient loss should auto-resume.
    @Volatile private var wasPlaying = false

    // --- Audio focus -------------------------------------------------------
    // Without this the app keeps playing over phone calls / other media apps and
    // never resumes afterwards — the single biggest "not a real music app" gap.
    private var audioManager: AudioManager? = null
    private var focusRequest: AudioFocusRequest? = null
    private var hasFocus = false
    // Set when a *transient* loss (call, nav prompt) paused us, so we resume on the
    // matching AUDIOFOCUS_GAIN. A permanent loss clears it.
    private var resumeOnFocusGain = false

    private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
        when (change) {
            AudioManager.AUDIOFOCUS_LOSS -> {
                // Another app took over for good — pause, don't auto-resume, and drop
                // our focus so the next manual play re-requests it.
                resumeOnFocusGain = false
                hasFocus = false
                MediaReceiver.nativeOnAction("pause")
            }
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT,
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                // We requested willPauseWhenDucked, so duck collapses into pause.
                resumeOnFocusGain = wasPlaying
                MediaReceiver.nativeOnAction("pause")
            }
            AudioManager.AUDIOFOCUS_GAIN -> {
                if (resumeOnFocusGain) {
                    resumeOnFocusGain = false
                    MediaReceiver.nativeOnAction("play")
                }
            }
        }
    }

    // --- Becoming noisy ----------------------------------------------------
    // Headphones unplugged / Bluetooth disconnected → pause instead of blasting the
    // speaker. Must be context-registered (implicit broadcast, not manifest-declared).
    private var noisyReceiver: BroadcastReceiver? = null

    @JvmStatic
    fun requestPermissions(activity: android.app.Activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (activity.checkSelfPermission(android.Manifest.permission.READ_MEDIA_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                activity.requestPermissions(arrayOf(android.Manifest.permission.READ_MEDIA_AUDIO), 1)
            }
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            if (activity.checkSelfPermission(android.Manifest.permission.READ_EXTERNAL_STORAGE) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                activity.requestPermissions(arrayOf(android.Manifest.permission.READ_EXTERNAL_STORAGE), 1)
            }
        }
    }

    // Synchronized: init can race between MainActivity.onCreate (main thread) and
    // the Rust JNI / artwork threads that lazily call render → init.
    @Synchronized
    @JvmStatic
    fun init(context: Context) {
        if (session != null) return
        // Application context: the session + audio focus outlive any single Activity.
        val app = context.applicationContext

        audioManager = app.getSystemService(Context.AUDIO_SERVICE) as AudioManager

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID, "Now Playing", NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Music playback controls"
                setShowBadge(false)
            }
            nm(app).createNotificationChannel(channel)
        }

        session = MediaSession(app, "kopuz").apply {
            setCallback(object : MediaSession.Callback() {
                override fun onPlay() = MediaReceiver.nativeOnAction("play")
                override fun onPause() = MediaReceiver.nativeOnAction("pause")
                override fun onSkipToNext() = MediaReceiver.nativeOnAction("next")
                override fun onSkipToPrevious() = MediaReceiver.nativeOnAction("prev")
                override fun onStop() = MediaReceiver.nativeOnAction("stop")
            })
            isActive = true
        }
    }

    // Artwork is cached by path so we don't decode on every update.
    // @Volatile because loadArt runs on a background thread (see updateNowPlaying).
    @Volatile private var cachedArtPath: String? = null
    @Volatile private var cachedArtBitmap: Bitmap? = null

    // Bumped on every updateNowPlaying call so a slow background artwork load can
    // tell whether it's still the current track before re-rendering.
    @Volatile private var generation = 0L

    private fun loadArt(path: String?): Bitmap? {
        if (path == null) return null
        if (path == cachedArtPath) return cachedArtBitmap
        val bmp = try {
            if (path.startsWith("http://") || path.startsWith("https://")) {
                val conn = java.net.URL(path).openConnection() as java.net.HttpURLConnection
                conn.connectTimeout = 5_000
                conn.readTimeout = 5_000
                conn.connect()
                BitmapFactory.decodeStream(conn.inputStream).also { conn.disconnect() }
            } else {
                BitmapFactory.decodeFile(path)
            }
        } catch (_: Exception) { null }
        cachedArtPath = path
        cachedArtBitmap = bmp
        return bmp
    }

    @JvmStatic
    fun updateNowPlaying(
        context: Context,
        title: String,
        artist: String,
        album: String,
        durationMs: Long,
        positionMs: Long,
        playing: Boolean,
        artworkPath: String?,
    ) {
        // Grab focus + arm the noisy receiver as soon as we start playing.
        if (playing) {
            if (!hasFocus) hasFocus = requestFocus()
            registerNoisy(context)
        }
        wasPlaying = playing

        val gen = ++generation
        // Render immediately with already-cached art so the calling (player) thread
        // never blocks on artwork I/O. Anything not cached is fetched off-thread and
        // re-rendered — but only if this is still the current track.
        val cached = if (artworkPath != null && artworkPath == cachedArtPath) cachedArtBitmap else null
        render(context, title, artist, album, durationMs, positionMs, playing, cached)
        if (artworkPath != null && artworkPath != cachedArtPath) {
            Thread {
                val art = loadArt(artworkPath)
                if (gen == generation) {
                    render(context, title, artist, album, durationMs, positionMs, playing, art)
                }
            }.apply { isDaemon = true }.start()
        }
    }

    private fun render(
        context: Context,
        title: String,
        artist: String,
        album: String,
        durationMs: Long,
        positionMs: Long,
        playing: Boolean,
        art: Bitmap?,
    ) {
        val s = session ?: run { init(context); session } ?: return

        val metaBuilder = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, durationMs)
        if (art != null) {
            metaBuilder.putBitmap(MediaMetadata.METADATA_KEY_ART, art)
            metaBuilder.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
        }
        s.setMetadata(metaBuilder.build())

        val stateVal = if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED
        s.setPlaybackState(
            PlaybackState.Builder()
                .setState(stateVal, positionMs, if (playing) 1f else 0f)
                .setActions(
                    PlaybackState.ACTION_PLAY or
                    PlaybackState.ACTION_PAUSE or
                    PlaybackState.ACTION_PLAY_PAUSE or
                    PlaybackState.ACTION_STOP or
                    PlaybackState.ACTION_SKIP_TO_NEXT or
                    PlaybackState.ACTION_SKIP_TO_PREVIOUS
                )
                .build()
        )

        val playPauseIcon =
            if (playing) android.R.drawable.ic_media_pause
            else android.R.drawable.ic_media_play
        val playPauseLabel = if (playing) "Pause" else "Play"
        val playPauseAction = if (playing) "pause" else "play"

        val notifBuilder = Notification.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText("$artist — $album")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(playing)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setDeleteIntent(pendingBroadcast(context, "stop"))
        contentIntent(context)?.let { notifBuilder.setContentIntent(it) }
        if (art != null) notifBuilder.setLargeIcon(art)
        val notif = notifBuilder
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(s.sessionToken)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            .addAction(
                Notification.Action.Builder(
                    android.R.drawable.ic_media_previous, "Prev",
                    pendingBroadcast(context, "prev")
                ).build()
            )
            .addAction(
                Notification.Action.Builder(
                    playPauseIcon, playPauseLabel,
                    pendingBroadcast(context, playPauseAction)
                ).build()
            )
            .addAction(
                Notification.Action.Builder(
                    android.R.drawable.ic_media_next, "Next",
                    pendingBroadcast(context, "next")
                ).build()
            )
            .build()

        pendingNotification = notif
        nm(context).notify(NOTIF_ID, notif)

        MusicService.update(context, playing)
    }

    @JvmStatic
    fun wakeMainThread() {
        android.os.Handler(android.os.Looper.getMainLooper()).post {}
    }

    @JvmStatic
    fun stopSession(context: Context) {
        pendingNotification = null
        wasPlaying = false
        resumeOnFocusGain = false
        abandonFocus()
        unregisterNoisy(context)
        MusicService.stop(context)
        nm(context).cancel(NOTIF_ID)
        session?.setPlaybackState(
            PlaybackState.Builder()
                .setState(PlaybackState.STATE_STOPPED, 0, 1f)
                .build()
        )
    }

    @JvmStatic
    fun release(context: Context) {
        abandonFocus()
        unregisterNoisy(context)
        session?.release()
        session = null
        nm(context).cancel(NOTIF_ID)
    }

    private fun requestFocus(): Boolean {
        val am = audioManager ?: return true
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val attrs = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build()
            val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(attrs)
                .setOnAudioFocusChangeListener(focusListener)
                // We can't lower volume from here (cpal writes the device directly),
                // so collapse "duck" into a clean pause.
                .setWillPauseWhenDucked(true)
                .build()
            focusRequest = req
            am.requestAudioFocus(req) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        } else {
            @Suppress("DEPRECATION")
            am.requestAudioFocus(
                focusListener,
                AudioManager.STREAM_MUSIC,
                AudioManager.AUDIOFOCUS_GAIN
            ) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        }
    }

    private fun abandonFocus() {
        val am = audioManager ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            focusRequest?.let { am.abandonAudioFocusRequest(it) }
        } else {
            @Suppress("DEPRECATION")
            am.abandonAudioFocus(focusListener)
        }
        hasFocus = false
    }

    private fun registerNoisy(context: Context) {
        if (noisyReceiver != null) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context?, intent: Intent?) {
                if (intent?.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                    MediaReceiver.nativeOnAction("pause")
                }
            }
        }
        context.applicationContext.registerReceiver(
            receiver, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
        )
        noisyReceiver = receiver
    }

    private fun unregisterNoisy(context: Context) {
        noisyReceiver?.let {
            try { context.applicationContext.unregisterReceiver(it) } catch (_: Exception) {}
        }
        noisyReceiver = null
    }

    // Tapping the notification reopens the app. With launchMode=singleTask + SINGLE_TOP
    // this brings the existing MainActivity forward (onNewIntent) instead of creating a
    // second instance, so native init never re-runs.
    private fun contentIntent(context: Context): PendingIntent? {
        val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?.apply { flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_NEW_TASK }
            ?: return null
        return PendingIntent.getActivity(
            context, 0, launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
    }

    private fun pendingBroadcast(context: Context, action: String): PendingIntent {
        val intent = Intent(context, MediaReceiver::class.java)
            .putExtra("kopuz_action", action)
        return PendingIntent.getBroadcast(
            context,
            action.hashCode(),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
    }

    private fun nm(ctx: Context) =
        ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
}
