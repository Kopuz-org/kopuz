#!/bin/sh
# Use the spike by hand on the emulator that is ALREADY running, against an isolated
# host kopuzd instead of the app's in-process core. This script never starts or stops
# an emulator. It
#  1. builds kopuzd and kopuzctl for the host once (into target/host), and the APK
#     with scripts/build-apk.sh if build/kopuz-gpui-spike.apk is missing;
#  2. starts kopuzd isolated exactly as test-emulator.sh --host-daemon does (env -i,
#     HOME and the database under build/daemon-home, --listen 127.0.0.1:$PORT), but
#     keeps build/daemon-home between runs; when the library is empty it adds
#     build/test-music (quiet test tones) as a folder source and scans it;
#  3. runs adb reverse tcp:$PORT tcp:$PORT and sets debug.kopuz.spike.addr to
#     127.0.0.1:$PORT, which makes the app connect to kopuzd over TCP;
#  4. installs the APK when it is not installed (or with --install), force-stops the
#     app and starts it from the launcher;
#  5. shows the app's log (logcat -s kopuz-spike) until Ctrl-C.
# On exit (Ctrl-C, TERM, HUP, or the log ending) it stops kopuzd, removes the adb
# reverse, clears debug.kopuz.spike.addr and force-stops the app, so the next start
# from the launcher uses the in-process core again. The APK stays installed and the
# emulator keeps running. kopuzd's log: build/dev-logs/kopuzd.log.
#
# usage: scripts/run-dev.sh [--install]
#   --install   reinstall build/kopuz-gpui-spike.apk (adb install -r) even if installed
# Env knobs: ANDROID_SERIAL (default: the first emulator-* in `adb devices`), PORT (50777),
# N_TRACKS (24), BUILD_LIMIT (2400 s per build), REBUILD_HOST (0).
# Exit: 130 after Ctrl-C, 2 when the environment is missing pieces, 3 when setup broke.
set -u

INSTALL=0
for arg in "$@"; do
    case $arg in
        --install) INSTALL=1 ;;
        -h | --help) sed -n '2,/^set -u/p' "$0" | sed '$d'; exit 0 ;;
        *) echo "run-dev.sh: unknown argument $arg" >&2; exit 2 ;;
    esac
done

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SPIKE_DIR=$(dirname -- "$SCRIPT_DIR")
REPO_DIR=$(CDPATH='' cd -- "$SPIKE_DIR/../.." && pwd)
# Not build/logs: test-emulator.sh empties that directory.
LOGS="$SPIKE_DIR/build/dev-logs"
. "$SCRIPT_DIR/env.sh" || exit 2
. "$SCRIPT_DIR/lib-host-daemon.sh"

REVERSED=0
PROP_SET=0
LAUNCHED=0
LOGCAT_PID=

cleanup() {
    rc=$?
    trap - EXIT INT TERM HUP
    set +e
    echo
    say "stopping (the emulator keeps running and the APK stays installed)"
    [ -n "$LOGCAT_PID" ] && kill "$LOGCAT_PID" 2>/dev/null
    kd_stop
    if [ -n "$SERIAL" ]; then
        [ "$REVERSED" = 1 ] && at 10 reverse --remove "tcp:$PORT" >/dev/null 2>&1 && echo "adb reverse tcp:$PORT removed"
        [ "$PROP_SET" = 1 ] && prop_clear && echo "$ADDR_PROP cleared"
        [ "$LAUNCHED" = 1 ] && sh_ am force-stop "$PKG" \
            && echo "spike app stopped; its next start from the launcher uses the in-process core"
    fi
    exit $rc
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

for tool in perl nc; do command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }; done
[ -x "$ADB" ] || { echo "missing $ADB" >&2; exit 2; }

kd_pick_emulator
mkdir -p "$BUILD" "$LOGS"
[ -f "$BUILD/.gitignore" ] || printf '*\n' >"$BUILD/.gitignore"

# ---- host side: builds (only what is missing), test tones, kopuzd, its library
kd_build
if [ ! -f "$APK" ]; then
    say "no APK yet: scripts/build-apk.sh (limit ${BUILD_LIMIT}s)"
    tmo "$BUILD_LIMIT" sh "$SCRIPT_DIR/build-apk.sh" || die "APK build failed"
fi
stale=$(find "$SPIKE_DIR/src" "$SPIKE_DIR/Cargo.toml" "$SPIKE_DIR/android" -newer "$APK" -type f 2>/dev/null | head -3)
[ -n "$stale" ] && printf 'WARNING: the APK is older than %s; rerun scripts/build-apk.sh and pass --install\n' "$stale"
kd_make_music
kd_start keep
kd_seed

# ---- device side: reverse, property, install, launch from the launcher
say "device $SERIAL: adb reverse tcp:$PORT, $ADDR_PROP=127.0.0.1:$PORT"
a reverse "tcp:$PORT" "tcp:$PORT" >/dev/null || die "adb reverse failed"
REVERSED=1
PROP_SET=1
prop_set "127.0.0.1:$PORT"
[ "$(prop_get)" = "127.0.0.1:$PORT" ] || die "could not set $ADDR_PROP (it is '$(prop_get)')"
if [ "$INSTALL" = 1 ] || ! installed; then
    say "adb install -r build/kopuz-gpui-spike.apk"
    at 180 install -r "$APK" || die "install failed"
else
    echo "the spike is installed (pass --install to reinstall build/kopuz-gpui-spike.apk)"
fi
sh_ am force-stop "$PKG"
SINCE="$(sh_ date +%s).000"
LAUNCHED=1
launch || die "could not start $PKG from the launcher"

rel() { printf '%s' "${1#"$SPIKE_DIR/"}"; }
cat <<EOF

======================================================================
 kopuz GPUI spike by hand, against a host kopuzd
   emulator  $SERIAL (left running)
   kopuzd    pid $KOPUZD_PID on 127.0.0.1:$PORT, $KD_TRACKS tracks, home $(rel "$DH")
             log: $(rel "$LOGS")/kopuzd.log
             CLI: HOME=$(rel "$DH") $(rel "$KOPUZCTL") --address 127.0.0.1:$PORT status
   device    adb reverse tcp:$PORT tcp:$PORT; $ADDR_PROP=127.0.0.1:$PORT
   app       $PKG, started from the launcher (backend=tcp); audio plays on this Mac
 Use the app in the emulator window. Its log follows (tag kopuz-spike).
 Press Ctrl-C to stop: that stops kopuzd, removes the reverse, clears the
 property and stops the app (its next launcher start is in-process).
======================================================================

EOF

# In the background so that Ctrl-C, TERM and HUP reach the traps at once.
"$ADB" -s "$SERIAL" logcat -v time -T "$SINCE" -s 'kopuz-spike:*' 'AndroidRuntime:E' </dev/null &
LOGCAT_PID=$!
wait "$LOGCAT_PID"
LOGCAT_PID=
echo "the log ended (did the emulator go away?)"
exit 0
