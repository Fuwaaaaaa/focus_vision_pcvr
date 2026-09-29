#include "direct_mode.h"
extern "C" {
#include "streaming_engine.h"
}
#include "driver_log.h"
#include "encode_params.h"
#include "gpu_adapter.h"
#include <string>
#include <thread>

namespace {

/// How long Present waits for the compositor to finish the frame.
constexpr uint32_t kSyncTimeoutMs = 10;

}  // namespace

CDirectModeComponent::~CDirectModeComponent()
{
    m_encoder.shutdown();
    m_eyeBlit.shutdown();
    m_sync.reset();
    if (m_pacingTimer) CloseHandle(m_pacingTimer);
}

bool CDirectModeComponent::init()
{
    std::string description;
    if (!fvp_gpu::createDevice(m_device, m_adapterLuid, description)) {
        driverLog("No GPU for SteamVR to render on: %s", description.c_str());
        return false;
    }
    m_device->GetImmediateContext(&m_context);
    driverLog("Rendering on %s", description.c_str());

    // The engine's config is the single source of truth for the stream.
    FvpConfig fvp{};
    NvencEncoder::Config enc;
    bool foveated = false;
    if (fvp_get_config(&fvp) == 0) {
        // encoded_* is the native per-eye size until the downscale lands
        // (the engine's build_fvp_config keeps them equal); EyeBlit scales
        // whatever SteamVR renders to it. A frame holds both eyes side by
        // side.
        enc.eye_width = fvp.encoded_width;
        enc.height = fvp.encoded_height;
        enc.fps = static_cast<uint32_t>(fvp.refresh_rate);
        enc.bitrate_bps = fvp_encode::targetBitrateBps(fvp.bitrate_bps);
        enc.full_range = fvp.full_range != 0;
        enc.fovea_radius = fvp.fovea_radius;
        enc.mid_radius = fvp.mid_radius;
        enc.mid_qp_offset = fvp.mid_qp_offset;
        enc.peripheral_qp_offset = fvp.peripheral_qp_offset;
        foveated = fvp.foveated_enabled != 0;
    } else {
        driverLog("Streaming engine config unavailable; encoding %ux%u per eye at defaults",
                  enc.eye_width, enc.height);
        enc.bitrate_bps = fvp_encode::kDefaultBitrateBps;
    }
    enc.width = enc.eye_width * 2;
    enc.use_hevc = true;
    m_pacer.setRefreshRate(enc.fps);
    // Sleep granularity is ~15.6 ms by default, longer than a 90 Hz frame;
    // a high-resolution timer (Windows 10 1803+) waits to the millisecond.
    m_pacingTimer = CreateWaitableTimerExW(nullptr, nullptr, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                                           TIMER_ALL_ACCESS);

    std::string error;
    if (!m_eyeBlit.init(m_device.Get(), enc.eye_width, enc.height, error)) {
        driverLog("Not streaming video: frame capture failed: %s", error.c_str());
        return true;
    }
    m_encoder.setFoveatedEnabled(foveated);
    m_encoderReady = m_encoder.init(m_device.Get(), m_eyeBlit.output(), enc, driverLogMessage);
    if (!m_encoderReady) {
        driverLog("Not streaming video: the encoder could not start (reason above)");
    }
    return true;
}

void CDirectModeComponent::requestIdr()
{
    m_encoder.requestIdr();
}

void CDirectModeComponent::logSometimes(uint32_t& count, const char* what)
{
    count++;
    if ((count & (count - 1)) == 0) {
        driverLog("%s (%u so far)", what, count);
    }
}

void CDirectModeComponent::CreateSwapTextureSet(
    uint32_t unPid,
    const SwapTextureSetDesc_t* pSwapTextureSetDesc,
    SwapTextureSet_t* pOutSwapTextureSet)
{
    if (!pOutSwapTextureSet || !pSwapTextureSetDesc)
        return;
    if (!m_device) {
        driverLog("CreateSwapTextureSet: no D3D11 device");
        return;
    }

    uint64_t handles[SwapTextureSets::kTexturesPerSet] = {};
    std::string error;
    bool created;
    {
        std::lock_guard<std::mutex> lock(m_swapMutex);
        created = m_swapSets.create(m_device.Get(), unPid, pSwapTextureSetDesc->nWidth,
                                    pSwapTextureSetDesc->nHeight,
                                    static_cast<DXGI_FORMAT>(pSwapTextureSetDesc->nFormat),
                                    pSwapTextureSetDesc->nSampleCount, handles, error);
    }
    if (!created) {
        driverLog("CreateSwapTextureSet failed: %s", error.c_str());
        return;
    }
    for (int i = 0; i < SwapTextureSets::kTexturesPerSet; i++) {
        pOutSwapTextureSet->rSharedTextureHandles[i] = handles[i];
    }
    driverLog("Swap texture set %ux%u, format %u, %u samples",
              pSwapTextureSetDesc->nWidth, pSwapTextureSetDesc->nHeight,
              pSwapTextureSetDesc->nFormat, pSwapTextureSetDesc->nSampleCount);
}

void CDirectModeComponent::DestroySwapTextureSet(vr::SharedTextureHandle_t sharedTextureHandle)
{
    std::lock_guard<std::mutex> lock(m_swapMutex);
    m_swapSets.destroySet(sharedTextureHandle);
}

void CDirectModeComponent::DestroyAllSwapTextureSets(uint32_t unPid)
{
    std::lock_guard<std::mutex> lock(m_swapMutex);
    m_swapSets.destroyAll(unPid);
}

void CDirectModeComponent::GetNextSwapTextureSetIndex(
    vr::SharedTextureHandle_t sharedTextureHandles[2],
    uint32_t (*pIndices)[2])
{
    if (!pIndices)
        return;
    // Each set's index is tracked here rather than read from pIndices,
    // which the header does not promise to fill in (ALVR does the same).
    const uint64_t handles[2] = {sharedTextureHandles[0], sharedTextureHandles[1]};
    std::lock_guard<std::mutex> lock(m_swapMutex);
    m_swapSets.nextIndices(handles, *pIndices);
}

void CDirectModeComponent::SubmitLayer(const SubmitLayerPerEye_t (&perEye)[2])
{
    // One call per layer; the first is the scene. Layers above it
    // (overlays, the dashboard) are not composited yet.
    if (m_haveLayer)
        return;
    m_haveLayer = true;
    for (int eye = 0; eye < 2; eye++) {
        m_layerTexture[eye] = perEye[eye].hTexture;
        m_layerBounds[eye] = perEye[eye].bounds;
    }
}

void CDirectModeComponent::Present(vr::SharedTextureHandle_t syncTexture)
{
    m_frameIndex++;
    const bool haveLayer = m_haveLayer;
    m_haveLayer = false;
    if (!m_encoderReady || !haveLayer)
        return;

    Microsoft::WRL::ComPtr<ID3D11ShaderResourceView> views[2];
    DXGI_FORMAT formats[2] = {DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN};
    {
        std::lock_guard<std::mutex> lock(m_swapMutex);
        for (int eye = 0; eye < 2; eye++) {
            if (const SwapTextureSets::Texture* texture = m_swapSets.find(m_layerTexture[eye])) {
                views[eye] = texture->view;
                formats[eye] = texture->format;
            }
        }
    }
    if (!views[0] || !views[1]) {
        // Not one of ours, or multisampled (no shader view).
        logSometimes(m_unknownTextures, "Submitted layer texture can't be read; frame skipped");
        return;
    }

    // Read the frame only once the compositor has finished drawing it.
    if (!m_sync.acquire(m_device.Get(), syncTexture, kSyncTimeoutMs)) {
        logSometimes(m_syncTimeouts, "Compositor frame not ready in time; frame skipped");
        return;
    }
    // Both eyes side by side; often one double-wide texture, each eye's
    // half named by its bounds.
    for (int eye = 0; eye < 2; eye++) {
        const vr::VRTextureBounds_t& b = m_layerBounds[eye];
        m_eyeBlit.draw(m_context.Get(), views[eye].Get(), formats[eye], {b.uMin, b.vMin, b.uMax, b.vMax}, eye);
    }
    m_sync.release();

    bool isIdr = false;
    if (!m_encoder.encode(false, m_nal, isIdr)) {
        logSometimes(m_encodeFailures, "Encode failed; frame skipped");
        return;
    }
    if (m_nal.empty())
        return;

    // Submit encoded NAL data to Rust streaming engine for RTP packetization
    const int32_t result = fvp_submit_encoded_nal(
        m_nal.data(),
        static_cast<uint32_t>(m_nal.size()),
        m_frameIndex,
        isIdr ? 1 : 0
    );
    if (result != 0) {
        logSometimes(m_submitFailures, "fvp_submit_encoded_nal failed");
    }
}

void CDirectModeComponent::PostPresent(const Throttling_t* /*pThrottling*/)
{
    const FramePacer::Clock::time_point now = FramePacer::Clock::now();
    const FramePacer::Clock::duration wait = m_pacer.deadline(now) - now;
    if (wait <= FramePacer::Clock::duration::zero())
        return;
    const long long ticks100ns = std::chrono::duration_cast<std::chrono::nanoseconds>(wait).count() / 100;
    LARGE_INTEGER due;
    due.QuadPart = -static_cast<LONGLONG>(ticks100ns);  // negative: relative
    if (m_pacingTimer && SetWaitableTimer(m_pacingTimer, &due, 0, nullptr, nullptr, FALSE)) {
        WaitForSingleObject(m_pacingTimer, INFINITE);
    } else {
        std::this_thread::sleep_for(wait);
    }
}

void CDirectModeComponent::GetFrameTiming(vr::DriverDirectMode_FrameTiming* pFrameTiming)
{
    if (pFrameTiming)
    {
        pFrameTiming->m_nSize = sizeof(vr::DriverDirectMode_FrameTiming);
        pFrameTiming->m_nNumFramePresents = 1;
        pFrameTiming->m_nNumMisPresented = 0;
        pFrameTiming->m_nNumDroppedFrames = 0;
        pFrameTiming->m_nReprojectionFlags = 0;
    }
}
