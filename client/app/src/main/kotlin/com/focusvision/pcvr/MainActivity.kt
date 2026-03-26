package com.focusvision.pcvr

import android.app.NativeActivity

class MainActivity : NativeActivity() {
    companion object {
        init { System.loadLibrary("focusvision_native") }
    }
}
