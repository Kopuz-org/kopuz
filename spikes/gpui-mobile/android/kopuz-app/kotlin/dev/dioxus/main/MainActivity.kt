package dev.dioxus.main

import dev.gpui.mobile.GpuiInputActivity

/**
 * The legacy app's activity name, which the shared Kotlin (`MusicService`) and the daemon
 * (`player::systemint::android`) reach statically. The GPUI host is `GpuiInputActivity`, so this
 * stand-in forwards each entry point to it.
 */
class MainActivity private constructor() {
    companion object {
        @JvmStatic
        fun requestBatteryExemptionOnce() = GpuiInputActivity.requestBatteryExemptionOnce()

        @JvmStatic
        fun moveToBack() = GpuiInputActivity.moveToBack()

        @JvmStatic
        fun pickFolder(): Boolean = GpuiInputActivity.pickFolderForDaemon()
    }
}
