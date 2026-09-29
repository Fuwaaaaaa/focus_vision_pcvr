#include "sync_texture.h"

bool SyncTexture::acquire(ID3D11Device* device, uint64_t handle, uint32_t timeoutMs) {
    if (!handle) return false;
    if (handle != m_handle || !m_mutex) {
        reset();
        Microsoft::WRL::ComPtr<ID3D11Texture2D> texture;
        if (FAILED(device->OpenSharedResource(reinterpret_cast<HANDLE>(handle), IID_PPV_ARGS(&texture)))
                || FAILED(texture.As(&m_mutex))) {
            m_mutex.Reset();
            return false;
        }
        m_handle = handle;
    }
    // S_OK only: WAIT_TIMEOUT and WAIT_ABANDONED are successes as HRESULTs.
    m_held = m_mutex->AcquireSync(0, timeoutMs) == S_OK;
    return m_held;
}

void SyncTexture::release() {
    if (m_held && m_mutex) m_mutex->ReleaseSync(0);
    m_held = false;
}

void SyncTexture::reset() {
    release();
    m_mutex.Reset();
    m_handle = 0;
}
