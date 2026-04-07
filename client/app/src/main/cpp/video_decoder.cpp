#include "video_decoder.h"
#include "xr_utils.h"

#include <GLES2/gl2ext.h> // GL_TEXTURE_EXTERNAL_OES
#include <cstring>
#include <chrono>
#include <unordered_map>

void VideoDecoder::cleanupSurfaceResources(JNIEnv* env) {
    if (m_surface) {
        ANativeWindow_release(m_surface);
        m_surface = nullptr;
    }
    if (m_surfaceTexture) {
        ASurfaceTexture_release(m_surfaceTexture);
        m_surfaceTexture = nullptr;
    }
    if (m_javaSurfaceTexture && env) {
        env->DeleteGlobalRef(m_javaSurfaceTexture);
        m_javaSurfaceTexture = nullptr;
    }
    m_useSurfaceOutput = false;
}

bool VideoDecoder::init(JNIEnv* env, int width, int height, const char* mimeType) {
    m_width = width;
    m_height = height;

    // Create GL_TEXTURE_EXTERNAL_OES for Surface output
    glGenTextures(1, &m_outputTexture);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, m_outputTexture);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, 0);

    if (m_outputTexture == 0) {
        LOGE("Failed to create GL_TEXTURE_EXTERNAL_OES");
        return false;
    }

    // Zero-copy SurfaceTexture path via JNI
    if (env) {
        env->GetJavaVM(&m_javaVM);

        jclass stClass = env->FindClass("android/graphics/SurfaceTexture");
        if (stClass) {
            jmethodID ctor = env->GetMethodID(stClass, "<init>", "(I)V");
            if (ctor) {
                jobject localST = env->NewObject(stClass, ctor, (jint)m_outputTexture);
                if (localST && !env->ExceptionCheck()) {
                    m_javaSurfaceTexture = env->NewGlobalRef(localST);
                    env->DeleteLocalRef(localST);

                    m_surfaceTexture = ASurfaceTexture_fromSurfaceTexture(
                        env, m_javaSurfaceTexture);
                    if (m_surfaceTexture) {
                        m_surface = ASurfaceTexture_acquireANativeWindow(m_surfaceTexture);
                        if (m_surface) {
                            m_useSurfaceOutput = true;
                            LOGI("VideoDecoder: Zero-copy SurfaceTexture enabled (texture=%u)",
                                 m_outputTexture);
                        } else {
                            LOGW("VideoDecoder: acquireANativeWindow failed, buffer fallback");
                            ASurfaceTexture_release(m_surfaceTexture);
                            m_surfaceTexture = nullptr;
                        }
                    } else {
                        LOGW("VideoDecoder: fromSurfaceTexture failed, buffer fallback");
                    }

                    if (!m_useSurfaceOutput && m_javaSurfaceTexture) {
                        env->DeleteGlobalRef(m_javaSurfaceTexture);
                        m_javaSurfaceTexture = nullptr;
                    }
                } else {
                    if (env->ExceptionCheck()) env->ExceptionClear();
                    LOGW("VideoDecoder: Java SurfaceTexture creation failed, buffer fallback");
                }
            }
            env->DeleteLocalRef(stClass);
        }
    }

    if (!m_useSurfaceOutput) {
        LOGI("VideoDecoder: Using buffer output mode");
    }

    // Create MediaCodec decoder
    m_codec = AMediaCodec_createDecoderByType(mimeType);
    if (!m_codec) {
        LOGE("Failed to create MediaCodec decoder for %s", mimeType);
        return false;
    }

    m_format = AMediaFormat_new();
    AMediaFormat_setString(m_format, AMEDIAFORMAT_KEY_MIME, mimeType);
    AMediaFormat_setInt32(m_format, AMEDIAFORMAT_KEY_WIDTH, width);
    AMediaFormat_setInt32(m_format, AMEDIAFORMAT_KEY_HEIGHT, height);
    // Low latency mode for VR streaming
    AMediaFormat_setInt32(m_format, "low-latency", 1);

    // Configure with Surface if available (zero-copy path), otherwise buffer mode
    media_status_t status = AMediaCodec_configure(
        m_codec, m_format,
        m_useSurfaceOutput ? m_surface : nullptr,
        nullptr, 0);
    if (status != AMEDIA_OK) {
        LOGE("Failed to configure MediaCodec: %d", (int)status);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
        cleanupSurfaceResources(env);
        return false;
    }

    status = AMediaCodec_start(m_codec);
    if (status != AMEDIA_OK) {
        LOGE("Failed to start MediaCodec: %d", (int)status);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
        cleanupSurfaceResources(env);
        return false;
    }

    m_initialized = true;
    LOGI("VideoDecoder initialized: %s %dx%d (texture=%u, surface=%s)",
         mimeType, width, height, m_outputTexture,
         m_useSurfaceOutput ? "yes" : "no");
    return true;
}

void VideoDecoder::shutdown() {
    if (m_codec) {
        AMediaCodec_stop(m_codec);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
    }
    if (m_format) {
        AMediaFormat_delete(m_format);
        m_format = nullptr;
    }
    if (m_surface) {
        ANativeWindow_release(m_surface);
        m_surface = nullptr;
    }
    if (m_surfaceTexture) {
        ASurfaceTexture_release(m_surfaceTexture);
        m_surfaceTexture = nullptr;
    }
    if (m_javaSurfaceTexture && m_javaVM) {
        JNIEnv* env = nullptr;
        bool didAttach = false;
        if (m_javaVM->GetEnv((void**)&env, JNI_VERSION_1_6) == JNI_EDETACHED) {
            m_javaVM->AttachCurrentThread(&env, nullptr);
            didAttach = true;
        }
        if (env) {
            env->DeleteGlobalRef(m_javaSurfaceTexture);
            m_javaSurfaceTexture = nullptr;
        }
        if (didAttach) m_javaVM->DetachCurrentThread();
    }
    if (m_outputTexture) {
        glDeleteTextures(1, &m_outputTexture);
        m_outputTexture = 0;
    }
    m_useSurfaceOutput = false;
    m_initialized = false;
    LOGI("VideoDecoder shut down");
}

bool VideoDecoder::submitPacket(const uint8_t* data, int size, int64_t timestampUs) {
    if (!m_initialized || !m_codec) return false;

    ssize_t bufIdx = AMediaCodec_dequeueInputBuffer(m_codec, 0); // non-blocking
    if (bufIdx < 0) return false; // no input buffer available

    size_t bufSize;
    uint8_t* buf = AMediaCodec_getInputBuffer(m_codec, bufIdx, &bufSize);
    if (!buf || (size_t)size > bufSize) return false;

    memcpy(buf, data, size);
    AMediaCodec_queueInputBuffer(m_codec, bufIdx, 0, size, timestampUs, 0);
    m_submitTimes.push_back(std::chrono::steady_clock::now());
    return true;
}

bool VideoDecoder::getDecodedFrame() {
    if (!m_initialized || !m_codec) return false;

    AMediaCodecBufferInfo info;
    ssize_t bufIdx = AMediaCodec_dequeueOutputBuffer(m_codec, &info, 0); // non-blocking

    if (bufIdx >= 0) {
        // Measure submit-to-output decode latency
        if (!m_submitTimes.empty()) {
            auto latency = std::chrono::steady_clock::now() - m_submitTimes.front();
            m_submitTimes.pop_front();
            uint32_t latencyUs = (uint32_t)std::chrono::duration_cast<
                std::chrono::microseconds>(latency).count();
            m_totalDecodeUs += latencyUs;
            m_decodeCount++;
            m_avgDecodeUs = (uint32_t)(m_totalDecodeUs / m_decodeCount);

            // Log every 90 frames (~1s at 90fps)
            if (m_decodeCount % 90 == 0) {
                LOGI("VideoDecoder: decode latency avg=%uus (%u frames)",
                     m_avgDecodeUs, m_decodeCount);
            }
        }

        if (m_useSurfaceOutput) {
            AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, true);
            ASurfaceTexture_updateTexImage(m_surfaceTexture);
        } else {
            AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, false);
        }
        return true;
    }

    if (bufIdx == AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED) {
        AMediaFormat* newFormat = AMediaCodec_getOutputFormat(m_codec);
        if (newFormat) {
            int32_t w = 0, h = 0;
            AMediaFormat_getInt32(newFormat, AMEDIAFORMAT_KEY_WIDTH, &w);
            AMediaFormat_getInt32(newFormat, AMEDIAFORMAT_KEY_HEIGHT, &h);
            LOGI("VideoDecoder: output format changed to %dx%d", w, h);
            AMediaFormat_delete(newFormat);
        }
    }

    return false;
}

void VideoDecoder::flush() {
    if (!m_initialized || !m_codec) return;
    AMediaCodec_flush(m_codec);
    m_submitTimes.clear();
    LOGI("VideoDecoder flushed");
}
