#!/bin/sh
# One self-contained run of the spike on the emulator that is ALREADY running (for
# example in Android Studio's Running Devices pane). This script never starts or stops
# an emulator.
#
# Default: the app as a user meets it. The in-process kopuz core with its seeded test
# tones, started from the launcher, with no host kopuzd, no adb reverse and the
# debug.kopuz.spike.addr property cleared. It installs build/kopuz-gpui-spike.apk,
# clears the spike's own data once (pm clear, so the first-run seeding runs), drives
# the screen with adb input, checks the resume and density screenshots for a black
# window, relaunches from the launcher (the library must persist, with no new seeding)
# and prints PASS/FAIL/UNKNOWN per check.
# --host-daemon: the earlier flow. It builds kopuzd/kopuzctl for the host (once, into
# target/host), starts an isolated kopuzd on 127.0.0.1:$PORT with a scanned folder of
# test tones, runs adb reverse tcp:$PORT and sets debug.kopuz.spike.addr to
# 127.0.0.1:$PORT, so the app connects over TCP.
# On exit it stops the spike app (and kopuzd), removes its adb reverse, clears the
# property it set, and restores the display density and rotation it changed.
# Outputs: build/logs/, build/shots/.
#
# usage: scripts/test-emulator.sh [--host-daemon] [--uninstall]
#        scripts/test-emulator.sh --png-stats FILE...
#   --host-daemon  test against an isolated host kopuzd over TCP (see above)
#   --uninstall    also uninstall the spike APK at the end (default: leave it installed)
#   --png-stats    only print how much of each PNG is exact black (exit 1 if one is mostly black)
# Run scripts/build-apk.sh first; the APK is built here only when it is missing.
# Exit: 0 when the checks ran (whatever they found), 2 when the environment is
# missing pieces, 3 when setup broke (build, daemon, no running emulator).
#
# Env knobs: ANDROID_SERIAL (default: the first emulator-* in `adb devices`),
# PORT (50777), N_TRACKS (24, host daemon), TOTAL_LIMIT (1500 s, after the builds),
# LOAD_LIMIT (120 s for the library to load), BUILD_LIMIT (2400 s per build),
# REBUILD_HOST (0), and tap points as "x%,y%" of the portrait screen, matched to
# src/ui.rs (status bar inset, a 64 px header with Play/Pause on the right, 64 px
# rows): TAP_ROW (30,15), TAP_TOGGLE (88,8), TAP_FILTER (30,8, the header's left
# side, where a Filter field would sit); SWIPE "x,y1,y2" (50,75,30).
set -u

# png_stats FILE...: how much of a screenshot is exact black (000000; the app's
# background is 121212). Reads the 8-bit RGB/RGBA non-interlaced PNGs screencap -p
# writes, with the Python standard library only (zlib; filter types 0-4). Samples a
# 40x40 grid from 8% to 95% of the height (below the status bar, above the gesture
# bar) and 4% to 96% of the width.
# Prints "FILE black=F mean_luma=L samples=N size=WxH" per file.
# Exit: 0 when no file is mostly black, 1 when one is (black > 0.5), 2 on a read error.
png_stats() {
    python3 - "$@" <<'PY'
import struct, sys, zlib
from itertools import accumulate

GRID, TOP, BOTTOM, SIDE = 40, 8, 5, 4


def read_png(path):
    with open(path, 'rb') as fh:
        data = fh.read()
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError('not a PNG')
    pos, idat, ihdr = 8, [], None
    while pos + 8 <= len(data):
        ln, typ = struct.unpack('>I4s', data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + ln]
        pos += 12 + ln
        if typ == b'IHDR':
            ihdr = struct.unpack('>IIBBBBB', body)
        elif typ == b'IDAT':
            idat.append(body)
        elif typ == b'IEND':
            break
    if ihdr is None:
        raise ValueError('no IHDR')
    w, h, depth, ctype, _, _, interlace = ihdr
    if depth != 8 or ctype not in (2, 6) or interlace:
        raise ValueError('unsupported PNG: depth %d, colour type %d, interlace %d' % (depth, ctype, interlace))
    bpp = 4 if ctype == 6 else 3
    raw = zlib.decompress(b''.join(idat))
    if len(raw) < h * (w * bpp + 1):
        raise ValueError('short image data')
    return w, h, bpp, w * bpp, raw


def unfilter(ft, line, prev, bpp, stride):
    if ft == 0:
        return line
    if ft == 2:  # Up: bytewise add without carries, on whole-row integers
        a, b = int.from_bytes(line, 'big'), int.from_bytes(prev, 'big')
        m7, m8 = int.from_bytes(b'\x7f' * stride, 'big'), int.from_bytes(b'\x80' * stride, 'big')
        return (((a & m7) + (b & m7)) ^ ((a ^ b) & m8)).to_bytes(stride, 'big')
    out = bytearray(line)
    if ft == 1:  # Sub: a running sum per channel
        for c in range(bpp):
            out[c::bpp] = bytes(map((255).__and__, accumulate(line[c::bpp])))
    elif ft == 3:  # Average
        for i in range(stride):
            left = out[i - bpp] if i >= bpp else 0
            out[i] = (out[i] + ((left + prev[i]) >> 1)) & 255
    elif ft == 4:  # Paeth
        for i in range(stride):
            a = out[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            out[i] = (out[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    else:
        raise ValueError('bad filter type %d' % ft)
    return bytes(out)


def stats(path):
    w, h, bpp, stride, raw = read_png(path)
    rows = sorted({min(h - 1, int(h * (TOP + (100 - TOP - BOTTOM) * k / (GRID - 1)) / 100)) for k in range(GRID)})
    cols = sorted({min(w - 1, int(w * (SIDE + (100 - 2 * SIDE) * k / (GRID - 1)) / 100)) for k in range(GRID)})
    ftypes = [raw[r * (stride + 1)] for r in range(h)]
    # A row with filter None or Sub does not depend on the row above, so decoding
    # can restart at the nearest such row at or above the next wanted row.
    prev, cur, black, luma, n = bytes(stride), -1, 0, 0.0, 0
    for r in rows:
        k = r
        while k > cur + 1 and ftypes[k] not in (0, 1):
            k -= 1
        if k > cur + 1:
            cur, prev = k - 1, bytes(stride)
        for rr in range(cur + 1, r + 1):
            off = rr * (stride + 1)
            prev = unfilter(ftypes[rr], raw[off + 1:off + 1 + stride], prev, bpp, stride)
        cur = r
        for x in cols:
            px = prev[x * bpp:x * bpp + 3]
            n += 1
            black += px == b'\0\0\0'
            luma += 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2]
    return black / n, luma / n, n, w, h


rc = 0
for path in sys.argv[1:]:
    try:
        frac, luma, n, w, h = stats(path)
    except (OSError, ValueError, IndexError, zlib.error) as err:
        print('%s error=%s' % (path, err))
        rc = 2
        continue
    print('%s black=%.2f mean_luma=%.1f samples=%d size=%dx%d' % (path, frac, luma, n, w, h))
    if frac > 0.5 and rc == 0:
        rc = 1
sys.exit(rc)
PY
}

if [ "${1:-}" = --png-stats ]; then
    shift
    [ $# -gt 0 ] || { echo "usage: $0 --png-stats FILE..." >&2; exit 2; }
    png_stats "$@"; exit
fi
HOST=0
UNINSTALL=0
for arg in "$@"; do
    case $arg in
        --host-daemon) HOST=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h | --help) sed -n '2,/^set -u/p' "$0" | sed '$d'; exit 0 ;;
        *) echo "test-emulator.sh: unknown argument $arg" >&2; exit 2 ;;
    esac
done

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SPIKE_DIR=$(dirname -- "$SCRIPT_DIR")
REPO_DIR=$(CDPATH='' cd -- "$SPIKE_DIR/../.." && pwd)
LOGS="$SPIKE_DIR/build/logs"
. "$SCRIPT_DIR/env.sh" || exit 2
. "$SCRIPT_DIR/lib-host-daemon.sh"

: "${TOTAL_LIMIT:=1500}" "${LOAD_LIMIT:=120}"
: "${TAP_ROW:=30,15}" "${TAP_TOGGLE:=88,8}" "${TAP_FILTER:=30,8}" "${SWIPE:=50,75,30}"

SHOTS="$BUILD/shots"
SUMMARY="$LOGS/summary.txt"
if [ "$HOST" = 1 ]; then
    MODE="host kopuzd over TCP (127.0.0.1:$PORT through adb reverse)"
    CHECKS="apk_install backend connected tracks_loaded"
else
    MODE="in-process core, started from the launcher, no host daemon"
    CHECKS="apk_install backend seeded tracks_loaded"
fi
CHECKS="$CHECKS play_tap scroll toggle lifecycle resume_shot density density_shot keyboard rotation relaunch no_crash renderer"

WATCHDOG_PID=
REVERSED=0
PROP_SET=0
LAUNCHED=0
ORIG_DENSITY=
ORIG_ACC=
ORIG_ROT=
DENSITY_CHANGED=0
ROTATION_CHANGED=0
SINCE=
n=

result() { printf '%-14s %-7s %s\n' "$1" "$2" "$3" >>"$SUMMARY"; printf -- '-- %s: %s %s\n' "$1" "$2" "$3"; }
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
# The number after "n=" in the last line matching PATTERN (0 if none).
last_n() { _v=$(last_line "$1" | sed -n 's/.*n=\([0-9]*\).*/\1/p'); echo "${_v:-0}"; }
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
# shot_check CHECK SHOT [NOTE]: PASS unless build/shots/SHOT.png is mostly exact black.
shot_check() {
    command -v python3 >/dev/null || { result "$1" UNKNOWN "no python3 to read $2.png${3:+; $3}"; return 0; }
    _st=$(png_stats "$SHOTS/$2.png" 2>&1); _rc=$?
    _st=${_st#"$SHOTS/"}
    case $_rc in
        0) result "$1" PASS "not black: $_st${3:+; $3}" ;;
        1) result "$1" FAIL "mostly black: $_st${3:+; $3}" ;;
        *) result "$1" UNKNOWN "$(printf '%s' "$_st" | tail -1)${3:+; $3}" ;;
    esac
}
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
    echo "mode: $MODE"
    for c in $CHECKS; do
        line=$(grep "^$c " "$SUMMARY" | tail -1)
        [ -n "$line" ] && echo "$line" || printf '%-14s %-7s %s\n' "$c" UNKNOWN "not reached"
    done
    echo "shots: $SHOTS   logs: $LOGS"
}

cleanup() {
    rc=$?
    trap - EXIT INT TERM HUP
    set +e
    echo
    say "cleanup (the emulator itself is left running)"
    [ -n "$WATCHDOG_PID" ] && kill "$WATCHDOG_PID" 2>/dev/null
    if [ -n "$SERIAL" ]; then
        [ "$DENSITY_CHANGED" = 1 ] && restore_density && echo "density restored"
        [ "$ROTATION_CHANGED" = 1 ] && restore_rotation && echo "rotation restored"
        [ "$LAUNCHED" = 1 ] && sh_ am force-stop "$PKG" && echo "spike app stopped"
        [ "$PROP_SET" = 1 ] && prop_clear && echo "$ADDR_PROP cleared (the next launch is in-process)"
        [ "$REVERSED" = 1 ] && at 10 reverse --remove "tcp:$PORT" >/dev/null 2>&1 && echo "adb reverse tcp:$PORT removed"
        [ "$UNINSTALL" = 1 ] && at 60 uninstall "$PKG" >/dev/null 2>&1 && echo "spike APK uninstalled"
    fi
    kd_stop
    summary
    exit $rc
}
trap cleanup EXIT
trap 'echo "interrupted"; exit 130' INT
trap 'echo "hangup"; exit 129' HUP
trap 'echo "stopped (time limit or TERM)"; exit 143' TERM

for tool in perl nc cmp; do command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }; done
[ -x "$ADB" ] || { echo "missing $ADB" >&2; exit 2; }
command -v python3 >/dev/null || echo "WARNING: no python3; the screenshot checks will be UNKNOWN"

kd_pick_emulator

mkdir -p "$BUILD" "$LOGS" "$SHOTS"
[ -f "$BUILD/.gitignore" ] || printf '*\n' >"$BUILD/.gitignore"
rm -f "$LOGS"/* "$SHOTS"/*
: >"$SUMMARY"
START=$(now)
say "mode: $MODE"

# ---- a. builds: the host daemon and CLI (host flow only), and the APK if missing
[ "$HOST" = 1 ] && kd_build
if [ ! -f "$APK" ]; then
    say "no APK yet: scripts/build-apk.sh (limit ${BUILD_LIMIT}s)"
    tmo "$BUILD_LIMIT" sh "$SCRIPT_DIR/build-apk.sh" || die "APK build failed"
fi
stale=$(find "$SPIKE_DIR/src" "$SPIKE_DIR/Cargo.toml" "$SPIKE_DIR/android" -newer "$APK" -type f 2>/dev/null | head -3)
[ -n "$stale" ] && printf 'WARNING: the APK is older than %s; rerun scripts/build-apk.sh\n' "$stale"
say "builds took $(( $(now) - START ))s; test phase limit ${TOTAL_LIMIT}s"
perl -e 'sleep shift; kill "TERM", shift' "$TOTAL_LIMIT" $$ &
WATCHDOG_PID=$!

# ---- b-d. host flow: test tones, an isolated kopuzd, its library
if [ "$HOST" = 1 ]; then
    kd_make_music
    kd_start fresh
    kd_seed
    n=$KD_TRACKS
fi

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
    echo "before: accelerometer_rotation=$ORIG_ACC user_rotation=$ORIG_ROT density_override=${ORIG_DENSITY:-none} $ADDR_PROP='$(prop_get)'"
} | tee "$LOGS/device.txt"
sh_ input keyevent KEYCODE_WAKEUP

# ---- f. backend selection, install, launch from the launcher
if [ "$HOST" = 1 ]; then
    say "adb reverse tcp:$PORT, $ADDR_PROP=127.0.0.1:$PORT"
    a reverse "tcp:$PORT" "tcp:$PORT" >/dev/null || die "adb reverse failed"
    REVERSED=1
    PROP_SET=1
    prop_set "127.0.0.1:$PORT"
    [ "$(prop_get)" = "127.0.0.1:$PORT" ] || die "could not set $ADDR_PROP"
else
    say "in-process: no host daemon, no adb reverse, $ADDR_PROP cleared"
    if reverse_listed; then
        nc -z 127.0.0.1 "$PORT" 2>/dev/null \
            && die "adb reverse tcp:$PORT and a server on 127.0.0.1:$PORT exist (is run-dev.sh running?); stop it first"
        a reverse --remove "tcp:$PORT" >/dev/null 2>&1 && echo "removed a stale adb reverse tcp:$PORT"
    fi
    prop_clear
    [ -z "$(prop_get)" ] || die "could not clear $ADDR_PROP (it is '$(prop_get)')"
fi
say "install"
if at 180 install -r "$APK" >"$LOGS/install.txt" 2>&1; then
    result apk_install PASS "$(tail -1 "$LOGS/install.txt")"
else
    result apk_install FAIL "$(tail -1 "$LOGS/install.txt")"
    exit 0
fi
if [ "$HOST" = 0 ]; then
    # Only the spike's own data: its library and settings, so the first-run seeding runs.
    echo "pm clear $PKG: $(sh_ pm clear "$PKG" | tail -1)"
fi
sh_ am force-stop "$PKG"
SINCE="$(sh_ date +%s).000"
LAUNCHED=1
say "launch from the launcher; waiting up to ${LOAD_LIMIT}s for the tracks"
launch
wait_log "SPIKE tracks loaded n=[1-9]" 0 "$LOAD_LIMIT" || echo "no 'SPIKE tracks loaded n>0' within ${LOAD_LIMIT}s"
sleep 2; shot 01-launch
if [ "$HOST" = 1 ]; then want="tcp addr=127.0.0.1:$PORT"; else want=in-process; fi
if [ "$(spike_count "SPIKE backend=$want")" -gt 0 ]; then result backend PASS "$(last_line 'SPIKE backend=')"
else result backend FAIL "want backend=$want; got: $(last_line 'SPIKE backend=')"; fi
if [ "$HOST" = 1 ]; then
    if [ "$(spike_count 'SPIKE connected')" -gt 0 ]; then result connected PASS "$(last_line 'SPIKE connected')"
    else result connected FAIL "no 'SPIKE connected' line; $(last_line 'SPIKE state='); $(last_line 'SPIKE error')"; fi
else
    if [ "$(last_n 'SPIKE seeded n=')" -gt 0 ]; then result seeded PASS "$(last_line 'SPIKE seeded n=')"
    else result seeded FAIL "no 'SPIKE seeded n>0' line after pm clear; $(last_line 'SPIKE seeded'); $(last_line 'SPIKE error')"; fi
fi
line=$(last_line 'SPIKE tracks loaded')
if [ "$(last_n 'SPIKE tracks loaded n=')" -gt 0 ]; then result tracks_loaded PASS "$line${n:+ (kopuzctl: $n)}"
else result tracks_loaded FAIL "${line:-no 'SPIKE tracks loaded' line}; $(last_line 'SPIKE state='); $(last_line 'SPIKE error')"; fi
[ -n "$(app_pid)" ] || echo "WARNING: $PKG is not running after launch"

# ---- g. interactions (each only while the spike has focus)
say "tap the first row ($TAP_ROW)"
if front; then
    b=$(spike_count "SPIKE play ok"); tap "$TAP_ROW"
    if wait_log "SPIKE play ok" "$b" 10; then
        if [ "$HOST" = 1 ]; then result play_tap PASS "$(last_line 'SPIKE play ok'); daemon: $(ctl status 2>/dev/null | grep -i phase | tr -s ' ')"
        else result play_tap PASS "$(last_line 'SPIKE play ok')"; fi
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

say "suspend (HOME) and resume (from the launcher)"
pid1=$(app_pid); b=$(spike_count "SPIKE lifecycle")
sh_ input keyevent KEYCODE_HOME; sleep 5; shot 06-home
launch; sleep 2; shot 07-resumed
sleep 4; shot 07-resumed-6s
late=$(png_stats "$SHOTS/07-resumed-6s.png" 2>/dev/null); late=${late#"$SHOTS/07-resumed-6s.png "}; late=${late%% samples=*}
shot_check resume_shot 07-resumed "2 s after the launcher start; 6 s: ${late:-unread}"
pid2=$(app_pid); l=$(( $(spike_count "SPIKE lifecycle") - b ))
if [ -n "$pid1" ] && [ "$pid1" = "$pid2" ] && [ "$l" -gt 0 ]; then
    result lifecycle PASS "pid $pid1 kept, $l lifecycle lines, last: $(last_line 'SPIKE lifecycle')"
else result lifecycle FAIL "pid before=${pid1:-none} after=${pid2:-none}, $l lifecycle lines"; fi

say "display scaling: wm density 280"
b=$(spike_count "SPIKE scale="); DENSITY_CHANGED=1; sh_ wm density 280
sleep 8; shot 08-density-280
if wait_log "SPIKE scale=" "$b" 5; then result density PASS "$(last_line 'SPIKE scale='), pid ${pid2:-?} -> $(app_pid)"
else result density FAIL "no new 'SPIKE scale=' line; pid now $(app_pid)"; fi
shot_check density_shot 08-density-280 "8 s after wm density 280"
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

say "relaunch: force-stop, then the launcher again (the library must persist)"
pid3=$(app_pid)
bs=$(spike_count "SPIKE seeded"); bt=$(spike_count "SPIKE tracks loaded n=[1-9]")
sh_ am force-stop "$PKG"; sleep 1
launch
if wait_log "SPIKE tracks loaded n=[1-9]" "$bt" "$LOAD_LIMIT"; then
    pid4=$(app_pid); s=$(( $(spike_count "SPIKE seeded") - bs ))
    if [ "$s" -eq 0 ]; then
        result relaunch PASS "$(last_line 'SPIKE tracks loaded'), no new seeding, pid ${pid3:-none} -> ${pid4:-none}"
    else result relaunch FAIL "$s new 'SPIKE seeded' lines: the library did not persist"; fi
else
    pid4=$(app_pid)
    result relaunch FAIL "no new 'SPIKE tracks loaded n>0' within ${LOAD_LIMIT}s; pid ${pid4:-none}; $(last_line 'SPIKE error')"
fi
sleep 2; shot 12-relaunched

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
grep " kopuz-spike: " "$LOGS/logcat-all.txt" | grep -v 'SPIKE backend=' | grep -iE "$GFX" >"$LOGS/renderer.txt"
pids=" $pid1 $pid2 $pid3 $pid4 $pid_end "
awk -v p="$pids" 'index(p, " " $3 " ") > 0' "$LOGS/logcat-all.txt" | grep -iE "$GFX" >"$LOGS/renderer-app-pid.txt"
if grep -qi 'vulkan' "$LOGS/renderer.txt"; then
    result renderer PASS "Vulkan: $(grep -i vulkan "$LOGS/renderer.txt" | head -1 | cut -c1-160)"
elif grep -qiE 'gles|opengl|(^|[^a-z])gl([^a-z]|$)' "$LOGS/renderer.txt"; then
    result renderer FAIL "GL fallback: $(grep -iE 'gles|opengl|gl' "$LOGS/renderer.txt" | head -1 | cut -c1-160)"
else
    result renderer UNKNOWN "no backend line under kopuz-spike; $(wc -l <"$LOGS/renderer-app-pid.txt" | tr -d ' ') graphics lines from the app pid in renderer-app-pid.txt"
fi
grep -c 'SPIKE error' "$LOGS/logcat-spike.txt" | sed 's/^/SPIKE error lines: /'
[ "$HOST" = 1 ] && ctl status >"$LOGS/kopuzd-status-end.txt" 2>&1
say "done in $(( $(now) - START ))s"
exit 0
