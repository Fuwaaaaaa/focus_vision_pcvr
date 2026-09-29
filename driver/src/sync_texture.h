#pragma once

#include <d3d11.h>
#include <dxgi.h>
#include <wrl/client.h>
#include <cstdint>

/**
 * The sync texture SteamVR's compositor passes to Present: a shared texture
 * with a keyed mutex the compositor holds while it renders the layers. The
 * driver reads the swap textures only between acquire() and release(), so
 * it never takes a frame that is still being drawn.
 */
class SyncTexture {
public:
    /// Acquire key 0 of the sync texture `handle` (opened on `device` the
    /// first time it's seen), waiting up to `timeoutMs`. False on timeout or
    /// if the handle can't be opened; release() is then not needed.
    bool acquire(ID3D11Device* device, uint64_t handle, uint32_t timeoutMs);

    /// Release key 0 after the reads are queued.
    void release();

    void reset();

private:
    uint64_t m_handle = 0;
    Microsoft::WRL::ComPtr<IDXGIKeyedMutex> m_mutex;
    bool m_held = false;
};
