# ============================================================================
# Focus Vision PCVR — Android R8 / ProGuard rules
# ============================================================================
#
# The Kotlin/Java layer of this app is intentionally tiny: a single
# NativeActivity subclass that loads libfocusvision_native.so. The C++ NDK
# layer (OpenXR runtime, MediaCodec H.264/H.265 decoder, RTP/FEC depacketizer,
# audio playback, TCP/TLS control channel, etc.) is not visible to R8 — it
# only sees the Kotlin surface and any JNI entry points.
#
# These rules preserve the names and signatures of anything the C++ side, the
# Android framework, or HTC's VR launcher may need to look up by string at
# runtime — those are not statically reachable so R8 would otherwise strip
# them.

# ---- App entry points -----------------------------------------------------

# MainActivity is referenced by AndroidManifest.xml via its fully qualified
# name. The Android plugin's default rules already keep activities, but
# being explicit here documents the invariant.
-keep class com.focusvision.pcvr.MainActivity { *; }

# Keep the companion-object static init that calls System.loadLibrary —
# loadLibrary itself is internal but Kotlin synthesizes a $Companion class.
-keepclassmembers class com.focusvision.pcvr.MainActivity$Companion {
    public <init>(...);
}

# ---- NativeActivity / JNI -------------------------------------------------

# android.app.NativeActivity uses reflection to discover lifecycle callbacks
# and to bridge to the native ANativeActivity_onCreate entry point. Subclasses
# must preserve their default constructor and any framework-visible methods.
-keep class * extends android.app.NativeActivity {
    public <init>();
    public *;
}

# Native methods (none today in Kotlin, but harmless and standard).
-keepclasseswithmembernames class * {
    native <methods>;
}

# ---- OpenXR loader --------------------------------------------------------

# The OpenXR loader resolves activity / context lookups via JNI when the
# runtime initializes. Keep the activity surface intact so it can find them.
-keepattributes JavascriptInterface
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes SourceFile,LineNumberTable

# ---- MediaCodec callbacks -------------------------------------------------

# MediaCodec.Callback subclasses (if any are added later) and Surface are
# accessed from native code. Currently the C++ side talks to MediaCodec via
# NDK AMediaCodec, not via Java Callback, so this is forward-compat
# protection, not a current requirement.
-keep class android.media.MediaCodec$Callback { *; }
-keep class android.view.Surface { *; }

# ---- Safety net -----------------------------------------------------------

# Don't warn about Android framework classes that may be missing on older
# SDK levels — minSdk=29 so this is mostly defensive.
-dontwarn android.**
-dontwarn androidx.**

# Strip log calls in release for a small additional size reduction.
-assumenosideeffects class android.util.Log {
    public static *** d(...);
    public static *** v(...);
}
