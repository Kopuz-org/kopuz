# Shared pieces of test-emulator.sh and run-dev.sh (POSIX sh, sourced after env.sh):
# small adb helpers for the emulator that is already running, and an isolated host
# kopuzd (built once into target/host, run with env -i and HOME, XDG_*, TMPDIR and
# the database under build/daemon-home, serving gRPC on 127.0.0.1:$PORT) with
# build/test-music as its library.
#
# The caller sets SPIKE_DIR, REPO_DIR and LOGS (where kopuzd.log goes) before
# sourcing, and a cleanup trap that calls kd_stop. Knobs: PORT (50777), N_TRACKS
# (24), BUILD_LIMIT (2400 s per build), REBUILD_HOST (0).

: "${PORT:=50777}" "${N_TRACKS:=24}" "${BUILD_LIMIT:=2400}" "${REBUILD_HOST:=0}"

PKG=moe.kopuz.gpuispike
ACTIVITY="$PKG/android.app.NativeActivity"
# The spike reads this system property at start: empty means the in-process
# core, an address means kopuzd over TCP (here through adb reverse).
ADDR_PROP=debug.kopuz.spike.addr
BUILD="$SPIKE_DIR/build"
MUSIC="$BUILD/test-music"
DH="$BUILD/daemon-home"
APK="$BUILD/kopuz-gpui-spike.apk"
HOST_TARGET="$SPIKE_DIR/target/host"
KOPUZD="$HOST_TARGET/debug/kopuzd"
KOPUZCTL="$HOST_TARGET/debug/kopuzctl"

SERIAL=
KOPUZD_PID=

say() { printf '== %s\n' "$*"; }
die() { printf 'SETUP FAILED: %s\n' "$*" >&2; exit 3; }
now() { date +%s; }
# Run an external command with a time limit; no stray timer process is left behind.
tmo() { _t=$1; shift; perl -e 'alarm shift; exec @ARGV or die "exec $ARGV[0]: $!\n"' "$_t" "$@"; }
# adb against the chosen emulator, never reading stdin: `a` with a 30 s limit, `at SECS` otherwise.
at() { _s=$1; shift; tmo "$_s" "$ADB" -s "$SERIAL" "$@" </dev/null; }
a() { at 30 "$@"; }
sh_() { a shell "$@" | tr -d '\r'; }

# ---- the running emulator (never started or stopped here)

# SERIAL: ANDROID_SERIAL, else the first emulator-* that is online; it must be booted.
kd_pick_emulator() {
    SERIAL=${ANDROID_SERIAL:-$(tmo 20 "$ADB" devices </dev/null | awk '$1 ~ /^emulator-/ && $2 == "device" { print $1; exit }')}
    [ -n "$SERIAL" ] || { SERIAL=; die "no running emulator in 'adb devices'; start the Pixel 8 Pro in Android Studio (or set ANDROID_SERIAL)"; }
    [ "$(at 10 shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] \
        || { _s=$SERIAL; SERIAL=; die "$_s is not booted"; }
}
prop_get() { sh_ getprop "$ADDR_PROP"; }
prop_set() { sh_ setprop "$ADDR_PROP" "$1"; }
# adb shell joins its arguments into one device command line, so the empty value is quoted for the device shell.
prop_clear() { sh_ "setprop $ADDR_PROP ''"; }
reverse_listed() { a reverse --list 2>/dev/null | tr -d '\r' | grep -q "tcp:$PORT "; }
installed() { sh_ pm path "$PKG" 2>/dev/null | grep -q '^package:'; }
# Start the spike the way the launcher does (a running task is brought to the front).
launch() {
    _out=$(sh_ monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 2>&1)
    case $_out in
        *'Events injected: 1'*) return 0 ;;
        *) printf 'launcher start failed: %s\n' "$(printf '%s' "$_out" | tail -2 | tr '\n' ' ')"; return 1 ;;
    esac
}

# ---- host kopuzd and kopuzctl

kd_build() {
    if [ "$REBUILD_HOST" = 1 ] || [ ! -x "$KOPUZD" ] || [ ! -x "$KOPUZCTL" ]; then
        say "cargo build kopuzd + kopuzctl (host, CARGO_TARGET_DIR=target/host, limit ${BUILD_LIMIT}s)"
        (cd "$REPO_DIR" && CARGO_TARGET_DIR="$HOST_TARGET" tmo "$BUILD_LIMIT" \
            cargo build -p kopuz-kopuzd --bin kopuzd -p kopuz-ctl --bin kopuzctl) || die "host build failed"
    fi
}

ctl() { tmo 40 env -i PATH="$PATH" HOME="$DH" NO_COLOR=1 "$KOPUZCTL" --address "127.0.0.1:$PORT" --timeout 30 "$@" </dev/null; }
kd_track_count() { ctl --json search "" --limit 200 2>/dev/null | grep -o '"key"' | wc -l | tr -d ' '; }

# N_TRACKS quiet test tones in build/test-music (ffmpeg FLAC, else python3 WAV).
kd_make_music() {
    [ "$(find "$MUSIC" -type f 2>/dev/null | wc -l | tr -d ' ')" -ge "$N_TRACKS" ] && return 0
    say "making $N_TRACKS test tones in build/test-music"
    rm -rf "$MUSIC"; mkdir -p "$MUSIC"
    if command -v ffmpeg >/dev/null; then
        _i=1
        while [ $_i -le "$N_TRACKS" ]; do
            _n=$(printf '%02d' $_i)
            tmo 60 ffmpeg -nostdin -hide_banner -loglevel error \
                -f lavfi -i "sine=frequency=$((220 + _i * 20)):sample_rate=22050:duration=30" \
                -af volume=0.2 -ac 1 -c:a flac \
                -metadata title="Spike Tone $_n" -metadata artist="Kopuz Spike" \
                -metadata album="Test Tones" -metadata track="$_i" \
                "$MUSIC/$_n Spike Tone $_n.flac" || die "ffmpeg failed"
            _i=$((_i + 1))
        done
    else
        command -v python3 >/dev/null || die "neither ffmpeg nor python3 to make test audio"
        tmo 120 python3 - "$MUSIC" "$N_TRACKS" <<'PY' || die "python3 test audio failed"
import math, struct, sys, os
out, count, rate, secs = sys.argv[1], int(sys.argv[2]), 22050, 30
def chunk(tag, data):
    return tag + struct.pack('<I', len(data)) + data + (b'\0' if len(data) % 2 else b'')
for i in range(1, count + 1):
    f = 220 + 20 * i
    pcm = b''.join(struct.pack('<h', int(6000 * math.sin(2 * math.pi * f * t / rate))) for t in range(rate * secs))
    info = b'INFO' + b''.join(chunk(k, v.encode() + b'\0') for k, v in
        ((b'INAM', 'Spike Tone %02d' % i), (b'IART', 'Kopuz Spike'), (b'IPRD', 'Test Tones'), (b'ITRK', str(i))))
    fmt = struct.pack('<HHIIHH', 1, 1, rate, rate * 2, 2, 16)
    body = b'WAVE' + chunk(b'fmt ', fmt) + chunk(b'data', pcm) + chunk(b'LIST', info)
    with open(os.path.join(out, '%02d Spike Tone %02d.wav' % (i, i)), 'wb') as fh:
        fh.write(b'RIFF' + struct.pack('<I', len(body)) + body)
PY
    fi
    ls "$MUSIC" | head -3; echo "... $(ls "$MUSIC" | wc -l | tr -d ' ') files"
}

# kd_start fresh|keep: kopuzd --listen 127.0.0.1:$PORT with nothing of the person's
# environment (env -i); HOME, XDG_*, TMPDIR and the KOPUZ_* path overrides all point
# inside build/daemon-home ("fresh" wipes it first). Aborts if kopuzd reports a
# database or log path outside it.
kd_start() {
    nc -z 127.0.0.1 "$PORT" 2>/dev/null && die "127.0.0.1:$PORT is already in use (another kopuzd, run-dev.sh or test-emulator.sh?)"
    [ "$1" = fresh ] && rm -rf "$DH"
    # A socket left by a kopuzd that did not exit cleanly (none of ours is running: the port is free).
    rm -f "$DH/run/kopuzd.sock"
    mkdir -p "$LOGS" "$DH/data" "$DH/run" "$DH/tmp" "$DH/share" \
        "$DH/xdg/config" "$DH/xdg/data" "$DH/xdg/cache" "$DH/xdg/state"
    say "starting kopuzd on 127.0.0.1:$PORT (home build/daemon-home)"
    # The subshell and env both exec, so $! is kopuzd itself.
    (cd "$DH" && exec env -i PATH="$PATH" USER="${USER:-spike}" LOGNAME="${USER:-spike}" LANG="${LANG:-en_US.UTF-8}" \
        HOME="$DH" TMPDIR="$DH/tmp/" \
        XDG_CONFIG_HOME="$DH/xdg/config" XDG_DATA_HOME="$DH/xdg/data" XDG_CACHE_HOME="$DH/xdg/cache" \
        XDG_STATE_HOME="$DH/xdg/state" XDG_RUNTIME_DIR="$DH/run" XDG_DATA_DIRS="$DH/share" \
        XDG_CONFIG_DIRS="$DH/xdg/config" \
        KOPUZ_DB_PATH="$DH/data/kopuz.db" KOPUZ_CONFIG_PATH="$DH/data/settings.toml" \
        KOPUZ_PLUGIN_DIR="$DH/data/plugins" KOPUZ_PLUGIN_PATH="$DH/share/kopuz-plugins" \
        KOPUZ_RUNTIME_DIR="$DH/run" KOPUZ_CONFIG_SYSTEM__AUTO_CHECK_UPDATES=false \
        NO_COLOR=1 RUST_LOG="${RUST_LOG:-info}" \
        "$KOPUZD" --socket run/kopuzd.sock --listen "127.0.0.1:$PORT" --db-path "$DH/data/kopuz.db") \
        >"$LOGS/kopuzd.log" 2>&1 </dev/null &
    KOPUZD_PID=$!
    _end=$(( $(now) + 90 ))
    until ctl status >"$LOGS/kopuzd-status.txt" 2>&1; do
        kill -0 "$KOPUZD_PID" 2>/dev/null || { tail -20 "$LOGS/kopuzd.log"; die "kopuzd exited"; }
        [ "$(now)" -lt "$_end" ] || { tail -20 "$LOGS/kopuzd.log"; die "kopuzd did not answer on 127.0.0.1:$PORT"; }
        sleep 2
    done
    cat "$LOGS/kopuzd-status.txt"
    say "isolation: paths kopuzd reports"
    grep -E 'daemon log|opening library database|kopuzd listening' "$LOGS/kopuzd.log" | sed 's/^.*INFO //'
    grep 'opening library database' "$LOGS/kopuzd.log" | grep -qF "$DH/data/kopuz.db" \
        || die "kopuzd did not open the isolated database; see $LOGS/kopuzd.log"
    grep 'daemon log' "$LOGS/kopuzd.log" | grep -qF "$DH/" \
        || die "kopuzd logs outside build/daemon-home; see $LOGS/kopuzd.log"
    say "isolation: files under build/daemon-home"
    find "$DH" -maxdepth 6 \( -type f -o -type s \) | sed "s|^$SPIKE_DIR/||" | head -40
    ctl volume 10 >/dev/null 2>&1 || echo "could not lower the volume"
}

# When the library is empty: add build/test-music as a folder source (once) and scan.
# Sets KD_TRACKS to the number of tracks kopuzctl lists.
kd_seed() {
    KD_TRACKS=$(kd_track_count)
    if [ "${KD_TRACKS:-0}" -gt 0 ]; then echo "the library already has $KD_TRACKS tracks"; return 0; fi
    if ctl source list 2>/dev/null | grep -q 'Spike test music'; then
        say "rescanning build/test-music"
    else
        say "adding build/test-music as a folder source and scanning"
        if ! ctl source add folders --name "Spike test music" --field "directories=$MUSIC" --activate; then
            echo "source add failed; falling back to the default folder source (\$HOME/Music)"
            [ -e "$DH/Music" ] || ln -s "$MUSIC" "$DH/Music"
            ctl source use local || true
        fi
    fi
    ctl source list || true
    ctl jobs start scan || true
    _end=$(( $(now) + 120 )); KD_TRACKS=0; _next=$(( $(now) + 20 ))
    while [ "$(now)" -lt "$_end" ]; do
        KD_TRACKS=$(kd_track_count)
        [ "$KD_TRACKS" -ge "$N_TRACKS" ] && break
        if [ "$(now)" -ge "$_next" ]; then ctl jobs start scan >/dev/null 2>&1; _next=$(( $(now) + 20 )); fi
        sleep 2
    done
    [ "$KD_TRACKS" -gt 0 ] || { ctl jobs; die "no tracks listed after the scan"; }
    echo "kopuzctl search lists $KD_TRACKS tracks"
}

# Stop kopuzd (TERM, then KILL after 15 s); safe to call twice.
kd_stop() {
    [ -n "$KOPUZD_PID" ] || return 0
    if kill -0 "$KOPUZD_PID" 2>/dev/null; then
        kill "$KOPUZD_PID"
        _i=0
        while kill -0 "$KOPUZD_PID" 2>/dev/null && [ $_i -lt 15 ]; do sleep 1; _i=$((_i + 1)); done
        kill -9 "$KOPUZD_PID" 2>/dev/null
        echo "kopuzd stopped"
    fi
    KOPUZD_PID=
}
