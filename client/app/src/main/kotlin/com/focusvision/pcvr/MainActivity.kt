package com.focusvision.pcvr

import android.app.NativeActivity
import android.content.Intent
import android.os.Bundle
import android.util.Log
import java.io.File

/**
 * The companion app starts the client over adb with the PC's address and
 * the pairing PIN:
 *
 *   am start -n com.focusvision.pcvr/.MainActivity \
 *       --es fvp_server 192.168.1.10 --es fvp_pin 012345 [--es fvp_udp_port 9945]
 *
 * The extras are written to a file in app-private storage, where the native
 * loop (OpenXRApp::checkLaunchRequest) reads and deletes it. A new launch
 * while the app runs (singleTask) arrives in onNewIntent and replaces the
 * connection. To pair with a different PC, clear the app's data
 * (`adb shell pm clear com.focusvision.pcvr`), which forgets the pinned
 * server certificate.
 */
class MainActivity : NativeActivity() {
    companion object {
        private const val TAG = "FocusVision"
        private const val EXTRA_SERVER = "fvp_server"
        private const val EXTRA_PIN = "fvp_pin"
        private const val EXTRA_UDP_PORT = "fvp_udp_port"
        private const val LAUNCH_REQUEST_FILE = "launch_request.txt" // launch_request.h

        init { System.loadLibrary("focusvision_native") }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Before the native thread starts, so its first check finds it.
        saveLaunchRequest(intent)
        super.onCreate(savedInstanceState)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        saveLaunchRequest(intent)
    }

    private fun saveLaunchRequest(intent: Intent?) {
        val server = intent?.getStringExtra(EXTRA_SERVER) ?: return
        val pin = intent.getStringExtra(EXTRA_PIN) ?: return
        val text = StringBuilder()
            .append("server=").append(server).append('\n')
            .append("pin=").append(pin).append('\n')
        intent.getStringExtra(EXTRA_UDP_PORT)?.let { text.append("udp_port=").append(it).append('\n') }
        // Hand a PIN over once: a recreated activity must not replay it (a
        // stale PIN costs an attempt against the engine's lockout).
        intent.removeExtra(EXTRA_PIN)
        // Write then rename, so the native side never reads half a file.
        val tmp = File(filesDir, "$LAUNCH_REQUEST_FILE.tmp")
        tmp.writeText(text.toString())
        if (!tmp.renameTo(File(filesDir, LAUNCH_REQUEST_FILE))) {
            Log.e(TAG, "Could not hand over the launch request")
        }
    }
}
