#!/usr/bin/env python3
"""Patch AndroidManifest.xml and generate mipmap icons after dx build."""
import sys, os, shutil

PERMISSIONS = [
    'android.permission.POST_NOTIFICATIONS',
    'android.permission.FOREGROUND_SERVICE',
    'android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK',
    'android.permission.WAKE_LOCK',
    'android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS',
    'android.permission.READ_EXTERNAL_STORAGE',
    'android.permission.WRITE_EXTERNAL_STORAGE',
    'android.permission.READ_MEDIA_AUDIO',
    'android.permission.READ_MEDIA_IMAGES',
]

INSIDE_APPLICATION = """\
        <service
            android:name="com.temidaradev.kopuz.MusicService"
            android:foregroundServiceType="mediaPlayback"
            android:stopWithTask="false"
            android:exported="false" />
        <receiver
            android:name="com.temidaradev.kopuz.MediaReceiver"
            android:exported="false">
            <intent-filter>
                <action android:name="com.temidaradev.kopuz.ACTION_MEDIA" />
            </intent-filter>
        </receiver>"""

def patch_manifest(path):
    if not os.path.exists(path):
        print(f"  skip (not found): {path}")
        return
    with open(path) as f:
        content = f.read()
    changed = False
    for perm in PERMISSIONS:
        if f'android:name="{perm}"' not in content:
            content = content.replace(
                '    <application ',
                f'    <uses-permission android:name="{perm}" />\n    <application ', 1
            )
            changed = True
    
    if 'android:requestLegacyExternalStorage="true"' not in content:
        content = content.replace(
            '    <application ',
            '    <application android:requestLegacyExternalStorage="true" ',
            1
        )
        changed = True
    # singleTask: our foreground service keeps the process alive in the background, so
    # relaunching would otherwise create a second MainActivity and call WryActivity_create
    # twice (tao aborts with SIGABRT). singleTask reuses the existing instance.
    if 'android:launchMode=' not in content:
        content = content.replace(
            '<activity ',
            '<activity android:launchMode="singleTask" ',
            1
        )
        changed = True
    if 'MusicService' not in content:
        content = content.replace('    </application>', INSIDE_APPLICATION + '\n    </application>', 1)
        changed = True
    if changed:
        with open(path, 'w') as f:
            f.write(content)
        print(f"  patched: {path}")
    else:
        print(f"  already patched: {path}")

def generate_icons(logo_path, res_dir):
    try:
        from PIL import Image
    except ImportError:
        print("  PIL not found — skipping icon generation (pip install Pillow)")
        return

    densities = {
        'mipmap-mdpi': 48,
        'mipmap-hdpi': 72,
        'mipmap-xhdpi': 96,
        'mipmap-xxhdpi': 144,
        'mipmap-xxxhdpi': 192,
    }
    logo = Image.open(logo_path).convert('RGBA')
    for density, size in densities.items():
        out_dir = os.path.join(res_dir, density)
        if not os.path.isdir(out_dir):
            continue
        # Remove any stale PNG (from a previous patch run) then overwrite the webp in-place.
        # Keeping a single format prevents Gradle's "duplicate resources" error.
        stale_png = os.path.join(out_dir, 'ic_launcher.png')
        if os.path.exists(stale_png):
            os.remove(stale_png)
        img = logo.resize((size, size), Image.LANCZOS)
        img.save(os.path.join(out_dir, 'ic_launcher.webp'), 'WEBP')

    # Adaptive icon foreground (108×108, logo centered in 72×72 safe zone)
    fg_size = 432  # xxxhdpi scale of 108dp
    safe = 288     # xxxhdpi scale of 72dp
    pad = (fg_size - safe) // 2
    fg = Image.new('RGBA', (fg_size, fg_size), (0, 0, 0, 0))
    logo_safe = logo.resize((safe, safe), Image.LANCZOS)
    fg.paste(logo_safe, (pad, pad))
    fg_dir = os.path.join(res_dir, 'mipmap-xxxhdpi')
    # Remove stale PNG foreground if present, then save as webp to match dx's format
    stale_fg_png = os.path.join(fg_dir, 'ic_launcher_foreground.png')
    if os.path.exists(stale_fg_png):
        os.remove(stale_fg_png)
    fg.save(os.path.join(fg_dir, 'ic_launcher_foreground.webp'), 'WEBP')

    # Update adaptive icon XML to reference webp foreground
    anydpi_dir = os.path.join(res_dir, 'mipmap-anydpi-v26')
    os.makedirs(anydpi_dir, exist_ok=True)
    xml_path = os.path.join(anydpi_dir, 'ic_launcher.xml')
    xml = ('<?xml version="1.0" encoding="utf-8"?>\n'
           '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
           '    <background android:drawable="@drawable/ic_launcher_background" />\n'
           '    <foreground android:drawable="@mipmap/ic_launcher_foreground" />\n'
           '</adaptive-icon>\n')
    with open(xml_path, 'w') as f:
        f.write(xml)

    # Remove old vector foreground
    old_fg = os.path.join(res_dir, 'drawable-v24', 'ic_launcher_foreground.xml')
    if os.path.exists(old_fg):
        os.remove(old_fg)

    print(f"  icons generated at {res_dir}")

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: patch_manifest.py <manifest_path> [<manifest_path2> ...]")
        sys.exit(1)

    for path in sys.argv[1:]:
        patch_manifest(path)

    # Detect res dir relative to the manifest path
    for path in sys.argv[1:]:
        res_dir = os.path.join(os.path.dirname(path), 'res')
        logo = os.path.join(os.path.dirname(__file__), '..', 'crates', 'kopuz', 'assets', 'logo.png')
        logo = os.path.normpath(logo)
        if os.path.isdir(res_dir) and os.path.exists(logo):
            generate_icons(logo, res_dir)
            break
