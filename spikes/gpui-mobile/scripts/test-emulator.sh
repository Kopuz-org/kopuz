#!/bin/sh
# One self-contained run of the spike on the emulator that is ALREADY running (for
# example in Android Studio's Running Devices pane). This script never starts or stops
# an emulator. It builds kopuzd/kopuzctl for the host (once, into target/host), starts
# an isolated kopuzd on 127.0.0.1:$PORT with a scanned folder of test tones, installs
# build/kopuz-gpui-spike.apk, drives the screen with adb input and prints
# PASS/FAIL/UNKNOWN per check. On exit it stops kopuzd and the spike app, removes its
# adb reverse, and restores the display density and rotation it changed.
# Outputs: build/logs/, build/shots/.
#
# usage: scripts/test-emulator.sh [--uninstall]
#   --uninstall   also uninstall the spike APK at the end (default: leave it installed)
# Run scripts/build-apk.sh first; the APK is built here only when it is missing.
# Exit: 0 when the checks ran (whatever they found), 2 when the environment is
# missing pieces, 3 when setup broke (build, daemon, no running emulator).
#
# Env knobs: ANDROID_SERIAL (default: the first emulator-* in `adb devices`),
# PORT (50777), N_TRACKS (24), TOTAL_LIMIT (1500 s, after the host build),
# BUILD_LIMIT (2400 s per build), REBUILD_HOST (0), and tap points as "x%,y%" of the
# portrait screen, matched to src/ui.rs (status bar inset, a 64 px header with
# Play/Pause on the right, 64 px rows): TAP_ROW (30,15), TAP_TOGGLE (88,8),
# TAP_FILTER (30,8, the header's left side, where a Filter field would sit);
# SWIPE "x,y1,y2" (50,75,30).
set -u

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SPIKE_DIR=$(dirname -- "$SCRIPT_DIR")
REPO_DIR=$(CDPATH='' cd -- "$SPIKE_DIR/../.." && pwd)
. "$SCRIPT_DIR/env.sh" || exit 2

UNINSTALL=0
for arg in "$@"; do
    case $arg in
        --uninstall) UNINSTALL=1 ;;
        -h | --help) sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "test-emulator.sh: unknown argument $arg" >&2; exit 2 ;;
    esac
done

: "${PORT:=50777}" "${N_TRACKS:=24}" "${TOTAL_LIMIT:=1500}" "${BUILD_LIMIT:=2400}" "${REBUILD_HOST:=0}"
: "${TAP_ROW:=30,15}" "${TAP_TOGGLE:=88,8}" "${TAP_FILTER:=30,8}" "${SWIPE:=50,75,30}"

PKG=moe.kopuz.gpuispike
ACTIVITY="$PKG/android.app.NativeActivity"
BUILD="$SPIKE_DIR/build"
LOGS="$BUILD/logs"
SHOTS="$BUILD/shots"
MUSIC="$BUILD/test-music"
DH="$BUILD/daemon-home"
APK="$BUILD/kopuz-gpui-spike.apk"
HOST_TARGET="$SPIKE_DIR/target/host"
KOPUZD="$HOST_TARGET/debug/kopuzd"
KOPUZCTL="$HOST_TARGET/debug/kopuzctl"
SUMMARY="$LOGS/summary.txt"
CHECKS="apk_install connected tracks_loaded play_tap scroll toggle lifecycle density keyboard rotation no_crash renderer"

SERIAL=
KOPUZD_PID=
WATCHDOG_PID=
REVERSED=0
LAUNCHED=0
ORIG_DENSITY=
ORIG_ACC=
ORIG_ROT=
DENSITY_CHANGED=0
ROTATION_CHANGED=0
SINCE=

say() { printf '== %s\n' "$*"; }
die() { printf 'SETUP FAILED: %s\n' "$*" >&2; exit 3; }
# Run an external command with a time limit; no stray timer process is left behind.
tmo() { _t=$1; shift; perl -e 'alarm shift; exec @ARGV or die "exec $ARGV[0]: $!\n"' "$_t" "$@"; }
# adb against the chosen emulator, never reading stdin: `a` with a 30 s limit, `at SECS` otherwise.
at() { _s=$1; shift; tmo "$_s" "$ADB" -s "$SERIAL" "$@" </dev/null; }
a() { at 30 "$@"; }
sh_() { a shell "$@" | tr -d '\r'; }
result() { printf '%-14s %-7s %s\n' "$1" "$2" "$3" >>"$SUMMARY"; printf -- '-- %s: %s %s\n' "$1" "$2" "$3"; }
now() { date +%s; }
shot() { at 20 exec-out screencap -p >"$SHOTS/$1.png" 2>/dev/null || echo "screencap $1 failed"; }
# The app's lines since this run launched it (logcat is never cleared: it is the person's device).
spike_log() { at 20 logcat -d -v brief -T "$SINCE" -s 'kopuz-spike:*' 2>/dev/null | tr -d '\r'; }
spike_count() { spike_log | grep -c -- "$1"; }
# wait_log PATTERN BASELINE SECONDS: until more than BASELINE lines match.
wait_log() {
    _end=$(( $(now) + $3 ))
    while [ "$(now)" -lt "$_end" ]; do
        [ "$(spike_count "$1")" -gt "$2" ] && return 0
        sleep 1
    done
    return 1
}
last_line() { spike_log | grep -- "$1" | tail -1 | sed 's/^[^:]*: //'; }
app_pid() { sh_ pidof "$PKG" 2>/dev/null; }
# The spike has input focus; one am start to bring it back if not.
focused() { sh_ 'dumpsys window 2>/dev/null | grep mCurrentFocus' | head -1 | grep -q "$PKG"; }
front() {
    focused && return 0
    sh_ am start -W -n "$ACTIVITY" >/dev/null; sleep 3
    focused
}
# tap "x%,y%"
tap() {
    _x=$(( SCREEN_W * ${1%,*} / 100 )); _y=$(( SCREEN_H * ${1#*,} / 100 ))
    echo "tap $_x,$_y"
    sh_ input tap "$_x" "$_y"
}
ctl() { tmo 40 env -i PATH="$PATH" HOME="$DH" NO_COLOR=1 "$KOPUZCTL" --address "127.0.0.1:$PORT" --timeout 30 "$@" </dev/null; }
restore_density() {
    case $ORIG_DENSITY in
        '') sh_ wm density reset ;;
        *) sh_ wm density "$ORIG_DENSITY" ;;
    esac
}
restore_rotation() {
    sh_ settings put system user_rotation "$ORIG_ROT"
    sh_ settings put system accelerometer_rotation "$ORIG_ACC"
}

summary() {
    [ -f "$SUMMARY" ] || return 0
    echo
    echo "================ SUMMARY (build/logs/summary.txt) ================"
    for c in $CHECKS; do
        line=$(grep "^$c " "$SUMMARY" | tail -1)
        [ -n "$line" ] && echo "$line" || printf '%-14s %-7s %s\n' "$c" UNKNOWN "not reached"
    done
    echo "shots: $SHOTS   logs: $LOGS"
}

cleanup() {
    rc=$?
    trap - EXIT INT TERM
    set +e
    echo
    say "cleanup (the emulator itself is left running)"
    [ -n "$WATCHDOG_PID" ] && kill "$WATCHDOG_PID" 2>/dev/null
    if [ -n "$SERIAL" ]; then
        [ "$DENSITY_CHANGED" = 1 ] && restore_density && echo "density restored"
        [ "$ROTATION_CHANGED" = 1 ] && restore_rotation && echo "rotation restored"
        [ "$LAUNCHED" = 1 ] && sh_ am force-stop "$PKG" && echo "spike app stopped"
        [ "$REVERSED" = 1 ] && at 10 reverse --remove "tcp:$PORT" >/dev/null 2>&1 && echo "adb reverse tcp:$PORT removed"
        [ "$UNINSTALL" = 1 ] && at 60 uninstall "$PKG" >/dev/null 2>&1 && echo "spike APK uninstalled"
    fi
    if [ -n "$KOPUZD_PID" ] && kill -0 "$KOPUZD_PID" 2>/dev/null; then
        kill "$KOPUZD_PID"
        i=0
        while kill -0 "$KOPUZD_PID" 2>/dev/null && [ $i -lt 15 ]; do sleep 1; i=$((i + 1)); done
        kill -9 "$KOPUZD_PID" 2>/dev/null
        echo "kopuzd stopped"
    fi
    summary
    exit $rc
}
trap cleanup EXIT
trap 'echo "interrupted"; exit 130' INT
trap 'echo "stopped (time limit or TERM)"; exit 143' TERM

for tool in perl nc cmp; do command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }; done
[ -x "$ADB" ] || { echo "missing $ADB" >&2; exit 2; }

# The running emulator: ANDROID_SERIAL, else the first emulator-* that is online.
SERIAL=${ANDROID_SERIAL:-$(tmo 20 "$ADB" devices </dev/null | awk '$1 ~ /^emulator-/ && $2 == "device" { print $1; exit }')}
[ -n "$SERIAL" ] || { SERIAL=; die "no running emulator in 'adb devices'; start the Pixel 8 Pro in Android Studio (or set ANDROID_SERIAL)"; }
[ "$(at 10 shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] \
    || { s=$SERIAL; SERIAL=; die "$s is not booted"; }

mkdir -p "$BUILD" "$LOGS" "$SHOTS"
[ -f "$BUILD/.gitignore" ] || printf '*\n' >"$BUILD/.gitignore"
rm -f "$LOGS"/* "$SHOTS"/*
: >"$SUMMARY"
START=$(now)

# ---- a. host daemon and CLI, once, into a target dir of the spike
if [ "$REBUILD_HOST" = 1 ] || [ ! -x "$KOPUZD" ] || [ ! -x "$KOPUZCTL" ]; then
    say "cargo build kopuzd + kopuzctl (host, CARGO_TARGET_DIR=target/host, limit ${BUILD_LIMIT}s)"
    (cd "$REPO_DIR" && CARGO_TARGET_DIR="$HOST_TARGET" tmo "$BUILD_LIMIT" \
        cargo build -p kopuz-kopuzd --bin kopuzd -p kopuz-ctl --bin kopuzctl) || die "host build failed"
fi
if [ ! -f "$APK" ]; then
    say "no APK yet: scripts/build-apk.sh (limit ${BUILD_LIMIT}s)"
    tmo "$BUILD_LIMIT" sh "$SCRIPT_DIR/build-apk.sh" || die "APK build failed"
fi
stale=$(find "$SPIKE_DIR/src" "$SPIKE_DIR/Cargo.toml" "$SPIKE_DIR/android" -newer "$APK" -type f 2>/dev/null | head -3)
[ -n "$stale" ] && printf 'WARNING: the APK is older than %s; rerun scripts/build-apk.sh\n' "$stale"
say "builds took $(( $(now) - START ))s; test phase limit ${TOTAL_LIMIT}s"
perl -e 'sleep shift; kill "TERM", shift' "$TOTAL_LIMIT" $$ &
WATCHDOG_PID=$!

# ---- b. isolated daemon home: HOME (which `directories` resolves every macOS path
# from), XDG_*, TMPDIR and the KOPUZ_* path overrides all point inside build/daemon-home.
nc -z 127.0.0.1 "$PORT" 2>/dev/null && die "127.0.0.1:$PORT is already in use"
rm -rf "$DH"
mkdir -p "$DH/data" "$DH/run" "$DH/tmp" "$DH/share" "$DH/xdg/config" "$DH/xdg/data" "$DH/xdg/cache" "$DH/xdg/state"

# ---- c. test tones
if [ "$(find "$MUSIC" -type f 2>/dev/null | wc -l | tr -d ' ')" -lt "$N_TRACKS" ]; then
    say "making $N_TRACKS test tones in build/test-music"
    rm -rf "$MUSIC"; mkdir -p "$MUSIC"
    if command -v ffmpeg >/dev/null; then
        i=1
        while [ $i -le "$N_TRACKS" ]; do
            n=$(printf '%02d' $i)
            tmo 60 ffmpeg -nostdin -hide_banner -loglevel error \
                -f lavfi -i "sine=frequency=$((220 + i * 20)):sample_rate=22050:duration=30" \
                -af volume=0.2 -ac 1 -c:a flac \
                -metadata title="Spike Tone $n" -metadata artist="Kopuz Spike" \
                -metadata album="Test Tones" -metadata track="$i" \
                "$MUSIC/$n Spike Tone $n.flac" || die "ffmpeg failed"
            i=$((i + 1))
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
fi
ls "$MUSIC" | head -3; echo "... $(ls "$MUSIC" | wc -l | tr -d ' ') files"

# ---- d. kopuzd --listen, with nothing of the person's environment (env -i)
say "starting kopuzd on 127.0.0.1:$PORT (home build/daemon-home)"
cd "$DH" || die "no $DH"
env -i PATH="$PATH" USER="${USER:-spike}" LOGNAME="${USER:-spike}" LANG="${LANG:-en_US.UTF-8}" \
    HOME="$DH" TMPDIR="$DH/tmp/" \
    XDG_CONFIG_HOME="$DH/xdg/config" XDG_DATA_HOME="$DH/xdg/data" XDG_CACHE_HOME="$DH/xdg/cache" \
    XDG_STATE_HOME="$DH/xdg/state" XDG_RUNTIME_DIR="$DH/run" XDG_DATA_DIRS="$DH/share" \
    XDG_CONFIG_DIRS="$DH/xdg/config" \
    KOPUZ_DB_PATH="$DH/data/kopuz.db" KOPUZ_CONFIG_PATH="$DH/data/settings.toml" \
    KOPUZ_PLUGIN_DIR="$DH/data/plugins" KOPUZ_PLUGIN_PATH="$DH/share/kopuz-plugins" \
    KOPUZ_RUNTIME_DIR="$DH/run" KOPUZ_CONFIG_SYSTEM__AUTO_CHECK_UPDATES=false \
    NO_COLOR=1 RUST_LOG="${RUST_LOG:-info}" \
    "$KOPUZD" --socket run/kopuzd.sock --listen "127.0.0.1:$PORT" --db-path "$DH/data/kopuz.db" \
    >"$LOGS/kopuzd.log" 2>&1 </dev/null &
KOPUZD_PID=$!
cd "$SPIKE_DIR" || exit 3
end=$(( $(now) + 90 ))
until ctl status >"$LOGS/kopuzd-status.txt" 2>&1; do
    kill -0 "$KOPUZD_PID" 2>/dev/null || { tail -20 "$LOGS/kopuzd.log"; die "kopuzd exited"; }
    [ "$(now)" -lt "$end" ] || { tail -20 "$LOGS/kopuzd.log"; die "kopuzd did not answer on 127.0.0.1:$PORT"; }
    sleep 2
done
cat "$LOGS/kopuzd-status.txt"
say "isolation: paths kopuzd reports"
grep -E 'daemon log|opening library database|kopuzd listening' "$LOGS/kopuzd.log" | sed 's/^.*INFO //'
grep 'opening library database' "$LOGS/kopuzd.log" | grep -qF "$DH/data/kopuz.db" \
    || die "kopuzd did not open the isolated database; see build/logs/kopuzd.log"
grep 'daemon log' "$LOGS/kopuzd.log" | grep -qF "$DH/" \
    || die "kopuzd logs outside build/daemon-home; see build/logs/kopuzd.log"
say "isolation: files under build/daemon-home"
find "$DH" -maxdepth 6 \( -type f -o -type s \) | sed "s|^$SPIKE_DIR/||" | head -40

ctl volume 10 >/dev/null 2>&1 || echo "could not lower the volume"
say "adding build/test-music as a folder source and scanning"
if ! ctl source add folders --name "Spike test music" --field "directories=$MUSIC" --activate; then
    echo "source add failed; falling back to the default folder source (\$HOME/Music)"
    ln -s "$MUSIC" "$DH/Music"
    ctl source use local || true
fi
ctl source list || true
ctl jobs start scan || true
end=$(( $(now) + 120 )); n=0; next_scan=$(( $(now) + 20 ))
while [ "$(now)" -lt "$end" ]; do
    n=$(ctl --json search "" --limit 200 2>/dev/null | grep -o '"key"' | wc -l | tr -d ' ')
    [ "$n" -ge "$N_TRACKS" ] && break
    if [ "$(now)" -ge "$next_scan" ]; then ctl jobs start scan >/dev/null 2>&1; next_scan=$(( $(now) + 20 )); fi
    sleep 2
done
[ "$n" -gt 0 ] || { ctl jobs; die "no tracks listed after the scan"; }
echo "kopuzctl search lists $n tracks"

# ---- e. the running emulator (read its state; remember what we will change)
say "device $SERIAL"
size=$(sh_ wm size | grep -v Override | tail -1 | sed 's/.*: //')
SCREEN_W=${size%x*}; SCREEN_H=${size#*x}
ORIG_DENSITY=$(sh_ wm density | sed -n 's/^Override density: //p')
ORIG_ACC=$(sh_ settings get system accelerometer_rotation); case $ORIG_ACC in [01]) ;; *) ORIG_ACC=1 ;; esac
ORIG_ROT=$(sh_ settings get system user_rotation); case $ORIG_ROT in [0-3]) ;; *) ORIG_ROT=0 ;; esac
{
    echo "serial=$SERIAL sdk=$(sh_ getprop ro.build.version.sdk) abi=$(sh_ getprop ro.product.cpu.abi)"
    echo "vulkan=$(sh_ getprop ro.hardware.vulkan) egl=$(sh_ getprop ro.hardware.egl) page=$(sh_ getconf PAGE_SIZE)"
    echo "screen=$size $(sh_ wm density | tr '\n' ' ')"
    echo "before: accelerometer_rotation=$ORIG_ACC user_rotation=$ORIG_ROT density_override=${ORIG_DENSITY:-none}"
} | tee "$LOGS/device.txt"
sh_ input keyevent KEYCODE_WAKEUP

# ---- f. install and launch
say "install, adb reverse tcp:$PORT, launch"
if at 180 install -r "$APK" >"$LOGS/install.txt" 2>&1; then
    result apk_install PASS "$(tail -1 "$LOGS/install.txt")"
else
    result apk_install FAIL "$(tail -1 "$LOGS/install.txt")"
    exit 0
fi
a reverse "tcp:$PORT" "tcp:$PORT" >/dev/null || die "adb reverse failed"
REVERSED=1
sh_ am force-stop "$PKG"
SINCE="$(sh_ date +%s).000"
LAUNCHED=1
sh_ am start -W -n "$ACTIVITY"
sleep 15
shot 01-launch
if wait_log "SPIKE connected" 0 15; then result connected PASS "$(last_line 'SPIKE connected')"
else result connected FAIL "no 'SPIKE connected' line; $(last_line 'SPIKE error')"; fi
if wait_log "SPIKE tracks loaded" 0 15; then
    line=$(last_line 'SPIKE tracks loaded'); count=$(echo "$line" | sed -n 's/.*n=\([0-9]*\).*/\1/p')
    [ "${count:-0}" -gt 0 ] && result tracks_loaded PASS "$line (kopuzctl: $n)" || result tracks_loaded FAIL "$line"
else result tracks_loaded FAIL "no 'SPIKE tracks loaded' line"; fi
[ -n "$(app_pid)" ] || echo "WARNING: $PKG is not running after launch"

# ---- g. interactions (each only while the spike has focus)
say "tap the first row ($TAP_ROW)"
if front; then
    b=$(spike_count "SPIKE play ok"); tap "$TAP_ROW"
    if wait_log "SPIKE play ok" "$b" 10; then result play_tap PASS "$(last_line 'SPIKE play ok'); daemon: $(ctl status 2>/dev/null | grep -i phase | tr -s ' ')"
    else result play_tap FAIL "no new 'SPIKE play ok'; $(last_line 'SPIKE error')"; fi
    sleep 2; shot 02-after-play
else result play_tap UNKNOWN "the spike did not have focus"; fi

say "swipe up on the list ($SWIPE)"
if front; then
    shot 03-scroll-before; sleep 2; shot 03-scroll-before-2
    x=${SWIPE%%,*}; rest=${SWIPE#*,}; y1=${rest%,*}; y2=${rest#*,}
    sh_ input swipe $((SCREEN_W * x / 100)) $((SCREEN_H * y1 / 100)) $((SCREEN_W * x / 100)) $((SCREEN_H * y2 / 100)) 300
    sleep 3; shot 04-scroll-after
    if ! cmp -s "$SHOTS/03-scroll-before.png" "$SHOTS/03-scroll-before-2.png"; then
        result scroll UNKNOWN "the screen changes on its own; compare 03/04 shots by eye"
    elif cmp -s "$SHOTS/03-scroll-before-2.png" "$SHOTS/04-scroll-after.png"; then
        result scroll FAIL "screen unchanged after the swipe"
    else result scroll PASS "screen changed after the swipe (see 03/04 shots)"; fi
else result scroll UNKNOWN "the spike did not have focus"; fi

say "tap Play/Pause ($TAP_TOGGLE)"
if front; then
    b=$(spike_count "SPIKE toggle ok"); tap "$TAP_TOGGLE"
    if wait_log "SPIKE toggle ok" "$b" 10; then result toggle PASS "$(last_line 'SPIKE toggle ok')"
    else result toggle FAIL "no new 'SPIKE toggle ok'; $(last_line 'SPIKE error')"; fi
    sleep 1; shot 05-after-toggle
else result toggle UNKNOWN "the spike did not have focus"; fi

say "suspend (HOME) and resume (am start)"
pid1=$(app_pid); b=$(spike_count "SPIKE lifecycle")
sh_ input keyevent KEYCODE_HOME; sleep 5; shot 06-home
sh_ am start -W -n "$ACTIVITY" >/dev/null; sleep 6; shot 07-resumed
pid2=$(app_pid); l=$(( $(spike_count "SPIKE lifecycle") - b ))
if [ -n "$pid1" ] && [ "$pid1" = "$pid2" ] && [ "$l" -gt 0 ]; then
    result lifecycle PASS "pid $pid1 kept, $l lifecycle lines, last: $(last_line 'SPIKE lifecycle')"
else result lifecycle FAIL "pid before=${pid1:-none} after=${pid2:-none}, $l lifecycle lines"; fi

say "display scaling: wm density 280"
b=$(spike_count "SPIKE scale="); DENSITY_CHANGED=1; sh_ wm density 280
sleep 8; shot 08-density-280
if wait_log "SPIKE scale=" "$b" 5; then result density PASS "$(last_line 'SPIKE scale='), pid ${pid2:-?} -> $(app_pid)"
else result density FAIL "no new 'SPIKE scale=' line; pid now $(app_pid)"; fi
restore_density; DENSITY_CHANGED=0; sleep 5

say "soft keyboard: tap the Filter field ($TAP_FILTER)"
if front; then
    tap "$TAP_FILTER"; sleep 3
    shown=$(sh_ dumpsys input_method | grep -o 'mInputShown=[a-z]*' | head -1)
    # Type only into the spike, never into whatever else might have focus.
    if front; then sh_ input text abc; typed="typed abc"; else typed="did not type: focus left the spike"; fi
    sleep 2; shot 09-keyboard-abc
    case $shown in
        *true) result keyboard PASS "$shown after the tap; $typed (see 09 shot)" ;;
        *) result keyboard FAIL "${shown:-mInputShown unknown} after the tap; $typed (see 09 shot)" ;;
    esac
else result keyboard UNKNOWN "the spike did not have focus"; fi

say "rotation: user_rotation 1"
b=$(spike_count "SPIKE scale="); ROTATION_CHANGED=1
sh_ settings put system accelerometer_rotation 0; sh_ settings put system user_rotation 1
sleep 8; shot 10-rotated
if wait_log "SPIKE scale=" "$b" 5; then
    line=$(last_line 'SPIKE scale='); wh=$(echo "$line" | sed -n 's/.*size=\([0-9.]*\)x\([0-9.]*\).*/\1 \2/p')
    if [ -n "$wh" ] && awk -v w="${wh% *}" -v h="${wh#* }" 'BEGIN { exit !(w > h) }'; then
        result rotation PASS "$line"
    else result rotation FAIL "new line is not landscape: $line"; fi
else result rotation FAIL "no new 'SPIKE scale=' line; pid now $(app_pid)"; fi
restore_rotation; ROTATION_CHANGED=0; sleep 5; shot 11-restored

# ---- h. logs (only what was logged since the launch)
say "collecting logs"
at 30 logcat -d -v threadtime -T "$SINCE" -s 'kopuz-spike:*' 'AndroidRuntime:E' 'DEBUG:*' 'wgpu*:*' 'RustStdoutStderr:*' \
    >"$LOGS/logcat-spike.txt" 2>&1
at 30 logcat -d -v threadtime -T "$SINCE" >"$LOGS/logcat-all.txt" 2>&1
at 30 logcat -d -b crash -v threadtime -T "$SINCE" >"$LOGS/logcat-crash.txt" 2>&1
pid_end=$(app_pid)
if grep -qE "$PKG|kopuz" "$LOGS/logcat-crash.txt" || grep -qE "Fatal signal.*(gpuispike|kopuz)" "$LOGS/logcat-all.txt"; then
    result no_crash FAIL "crash records in logcat-crash.txt / logcat-all.txt; pid now ${pid_end:-none}"
elif [ -z "$pid_end" ]; then result no_crash FAIL "no crash record, but $PKG is not running"
else result no_crash PASS "no crash records, pid $pid_end alive"; fi
# The backend: wgpu/gpui log through the `log` crate, so under the app's own tag.
GFX='adapter|backend|vulkan|gles|opengl|(^|[^a-z])gl([^a-z]|$)'
grep " kopuz-spike: " "$LOGS/logcat-all.txt" | grep -iE "$GFX" >"$LOGS/renderer.txt"
pids=" $pid1 $pid2 $pid_end "
awk -v p="$pids" 'index(p, " " $3 " ") > 0' "$LOGS/logcat-all.txt" | grep -iE "$GFX" >"$LOGS/renderer-app-pid.txt"
if grep -qi 'vulkan' "$LOGS/renderer.txt"; then
    result renderer PASS "Vulkan: $(grep -i vulkan "$LOGS/renderer.txt" | head -1 | cut -c1-160)"
elif grep -qiE 'gles|opengl|(^|[^a-z])gl([^a-z]|$)' "$LOGS/renderer.txt"; then
    result renderer FAIL "GL fallback: $(grep -iE 'gles|opengl|gl' "$LOGS/renderer.txt" | head -1 | cut -c1-160)"
else
    result renderer UNKNOWN "no backend line under kopuz-spike; $(wc -l <"$LOGS/renderer-app-pid.txt" | tr -d ' ') graphics lines from the app pid in renderer-app-pid.txt"
fi
grep -c 'SPIKE error' "$LOGS/logcat-spike.txt" | sed 's/^/SPIKE error lines: /'
ctl status >"$LOGS/kopuzd-status-end.txt" 2>&1
say "done in $(( $(now) - START ))s"
exit 0
