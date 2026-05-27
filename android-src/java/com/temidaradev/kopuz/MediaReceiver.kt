package com.temidaradev.kopuz

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

class MediaReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.getStringExtra("kopuz_action") ?: return
        nativeOnAction(action)
    }

    companion object {
        @JvmStatic
        external fun nativeOnAction(action: String)
    }
}
