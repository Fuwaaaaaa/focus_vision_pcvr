// The driver's side of SteamVR direct mode, run against WARP (D3D11's
// software rasterizer, available without a GPU). A second device plays
// SteamVR's compositor, which lives in another process: it opens the swap
// textures by their shared handles, fills them, and hands over through the
// sync texture's keyed mutex, as in Present.

#include <gtest/gtest.h>

#include <d3d11.h>
#include <dxgi.h>
#include <wrl/client.h>
#include <cmath>
#include <cstdint>
#include <vector>

#include "eye_blit.h"
#include "swap_textures.h"
#include "sync_texture.h"

using Microsoft::WRL::ComPtr;

namespace {

struct Device {
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
};

Device makeWarpDevice() {
    Device d;
    const D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_0;
    HRESULT hr = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr,
                                   D3D11_CREATE_DEVICE_BGRA_SUPPORT, &level, 1, D3D11_SDK_VERSION,
                                   &d.device, nullptr, &d.context);
    EXPECT_TRUE(SUCCEEDED(hr)) << "WARP device: 0x" << std::hex << hr;
    return d;
}

ComPtr<ID3D11Texture2D> openShared(ID3D11Device* device, uint64_t handle) {
    ComPtr<ID3D11Texture2D> texture;
    HRESULT hr = device->OpenSharedResource(reinterpret_cast<HANDLE>(handle), IID_PPV_ARGS(&texture));
    EXPECT_TRUE(SUCCEEDED(hr)) << "OpenSharedResource: 0x" << std::hex << hr;
    return texture;
}

/// RGBA8 pixels, row-major.
struct Rgba {
    uint8_t r, g, b, a;
};

/// Fill `texture` (4 bytes per pixel) on the compositor's device.
void upload(const Device& compositor, ID3D11Texture2D* texture, uint32_t width,
            const std::vector<Rgba>& pixels) {
    compositor.context->UpdateSubresource(texture, 0, nullptr, pixels.data(), width * 4, 0);
}

/// Read back an 8-bit, 4-channel texture of our device.
std::vector<uint8_t> readBack(const Device& d, ID3D11Texture2D* texture) {
    D3D11_TEXTURE2D_DESC desc;
    texture->GetDesc(&desc);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    desc.MiscFlags = 0;
    ComPtr<ID3D11Texture2D> staging;
    EXPECT_TRUE(SUCCEEDED(d.device->CreateTexture2D(&desc, nullptr, &staging)));
    d.context->CopyResource(staging.Get(), texture);
    D3D11_MAPPED_SUBRESOURCE mapped{};
    EXPECT_TRUE(SUCCEEDED(d.context->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapped)));
    std::vector<uint8_t> out(desc.Width * desc.Height * 4);
    for (uint32_t y = 0; y < desc.Height; y++) {
        memcpy(&out[y * desc.Width * 4], static_cast<uint8_t*>(mapped.pData) + y * mapped.RowPitch,
               desc.Width * 4);
    }
    d.context->Unmap(staging.Get(), 0);
    return out;
}

/// The compositor's sync texture: shared, with a keyed mutex.
uint64_t makeSyncTexture(const Device& compositor, ComPtr<IDXGIKeyedMutex>& mutex) {
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = desc.Height = 16;
    desc.MipLevels = desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX;
    ComPtr<ID3D11Texture2D> texture;
    EXPECT_TRUE(SUCCEEDED(compositor.device->CreateTexture2D(&desc, nullptr, &texture)));
    EXPECT_TRUE(SUCCEEDED(texture.As(&mutex)));
    ComPtr<IDXGIResource> resource;
    EXPECT_TRUE(SUCCEEDED(texture.As(&resource)));
    HANDLE handle = nullptr;
    EXPECT_TRUE(SUCCEEDED(resource->GetSharedHandle(&handle)));
    return reinterpret_cast<uint64_t>(handle);
}

/// The BGRA bytes of eye `eye`'s half of a side-by-side output
/// (`eyeWidth` × `height` per eye).
std::vector<uint8_t> eyeHalf(const std::vector<uint8_t>& bgra, uint32_t eyeWidth, uint32_t height, int eye) {
    std::vector<uint8_t> out;
    out.reserve(eyeWidth * height * 4);
    for (uint32_t y = 0; y < height; y++) {
        const uint8_t* row = &bgra[(y * eyeWidth * 2 + eye * eyeWidth) * 4];
        out.insert(out.end(), row, row + eyeWidth * 4);
    }
    return out;
}

/// One frame end to end: the compositor fills a swap texture of `format`
/// (`width` × `height`, `pixels`) under the sync texture's mutex; the driver
/// takes the mutex and blits `uv` into the left eye of an output of two
/// `outWidth` × `outHeight` eyes. Returns the left eye's BGRA bytes.
std::vector<uint8_t> runFrame(DXGI_FORMAT format, uint32_t width, uint32_t height,
                              const std::vector<Rgba>& pixels, const fvp_blit::UvRect& uv,
                              uint32_t outWidth, uint32_t outHeight) {
    Device driver = makeWarpDevice();
    Device compositor = makeWarpDevice();

    SwapTextureSets sets;
    uint64_t handles[SwapTextureSets::kTexturesPerSet] = {};
    std::string error;
    EXPECT_TRUE(sets.create(driver.device.Get(), 42, width, height, format, 1, handles, error)) << error;

    ComPtr<IDXGIKeyedMutex> compositorMutex;
    const uint64_t syncHandle = makeSyncTexture(compositor, compositorMutex);

    // Compositor: render the frame into swap texture 0, then hand over.
    ComPtr<ID3D11Texture2D> target = openShared(compositor.device.Get(), handles[0]);
    EXPECT_EQ(compositorMutex->AcquireSync(0, INFINITE), S_OK);
    upload(compositor, target.Get(), width, pixels);
    EXPECT_EQ(compositorMutex->ReleaseSync(0), S_OK);

    // Driver (Present): read it under the mutex.
    EyeBlit blit;
    EXPECT_TRUE(blit.init(driver.device.Get(), outWidth, outHeight, error)) << error;
    SyncTexture sync;
    EXPECT_TRUE(sync.acquire(driver.device.Get(), syncHandle, 1000));
    const SwapTextureSets::Texture* submitted = sets.find(handles[0]);
    EXPECT_NE(submitted, nullptr);
    EXPECT_TRUE(blit.draw(driver.context.Get(), submitted->view.Get(), format, uv, 0));
    sync.release();
    return eyeHalf(readBack(driver, blit.output()), outWidth, outHeight, 0);
}

std::vector<Rgba> halves(uint32_t width, uint32_t height, Rgba left, Rgba right) {
    std::vector<Rgba> px(width * height);
    for (uint32_t y = 0; y < height; y++)
        for (uint32_t x = 0; x < width; x++) px[y * width + x] = x < width / 2 ? left : right;
    return px;
}

/// Every output pixel (BGRA) is within `tolerance` of `expected` (RGB).
void expectAll(const std::vector<uint8_t>& bgra, Rgba expected, int tolerance, const char* what) {
    size_t bad = 0;
    for (size_t i = 0; i < bgra.size(); i += 4) {
        if (std::abs(bgra[i + 2] - expected.r) > tolerance || std::abs(bgra[i + 1] - expected.g) > tolerance ||
            std::abs(bgra[i + 0] - expected.b) > tolerance) {
            if (bad++ == 0) {
                ADD_FAILURE() << what << ": pixel " << i / 4 << " is RGB(" << int(bgra[i + 2]) << ", "
                              << int(bgra[i + 1]) << ", " << int(bgra[i]) << "), want (" << int(expected.r)
                              << ", " << int(expected.g) << ", " << int(expected.b) << ")";
            }
        }
    }
    EXPECT_EQ(bad, 0u) << what;
}

const Rgba kOrange{200, 100, 50, 255};
const Rgba kBlue{10, 20, 240, 255};

}  // namespace

TEST(SwapTextureSets, HandlesAreRealSharedHandlesTheCompositorCanOpen) {
    // REGRESSION: the handles were a counter (1, 2, 3...), which the
    // compositor cannot open.
    Device driver = makeWarpDevice();
    Device compositor = makeWarpDevice();
    SwapTextureSets sets;
    uint64_t handles[3] = {};
    std::string error;
    ASSERT_TRUE(sets.create(driver.device.Get(), 7, 64, 32, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 1, handles, error))
        << error;
    EXPECT_NE(handles[0], handles[1]);
    EXPECT_NE(handles[1], handles[2]);
    for (uint64_t h : handles) {
        ASSERT_NE(sets.find(h), nullptr);
        ComPtr<ID3D11Texture2D> opened = openShared(compositor.device.Get(), h);
        ASSERT_TRUE(opened);
        D3D11_TEXTURE2D_DESC desc;
        opened->GetDesc(&desc);
        EXPECT_EQ(desc.Width, 64u);
        EXPECT_EQ(desc.Height, 32u);
        EXPECT_EQ(desc.Format, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
        EXPECT_TRUE(desc.BindFlags & D3D11_BIND_RENDER_TARGET) << "the compositor renders into it";
    }
}

TEST(SwapTextureSets, EachSetAdvancesThroughItsTextures) {
    // GetNextSwapTextureSetIndex: the driver tracks each set's index rather
    // than trusting the values SteamVR passes in.
    Device driver = makeWarpDevice();
    SwapTextureSets sets;
    uint64_t left[3] = {}, right[3] = {};
    std::string error;
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 16, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, left, error));
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 16, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, right, error));

    // One set per eye: each advances once per frame, 1, 2, 0, ...
    uint32_t indices[2] = {77, 77};
    const uint64_t perEye[2] = {left[0], right[0]};
    for (uint32_t expected : {1u, 2u, 0u, 1u}) {
        sets.nextIndices(perEye, indices);
        EXPECT_EQ(indices[0], expected);
        EXPECT_EQ(indices[1], expected);
    }

    // Both eyes in one set (a double-wide texture): it advances once.
    uint64_t shared[3] = {};
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 32, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, shared, error));
    const uint64_t sameSet[2] = {shared[1], shared[2]};
    sets.nextIndices(sameSet, indices);
    EXPECT_EQ(indices[0], 1u);
    EXPECT_EQ(indices[1], 1u) << "not advanced a second time for the other eye";

    // An unknown handle leaves its entry alone.
    const uint64_t unknown[2] = {12345, left[0]};
    uint32_t kept[2] = {9, 9};
    sets.nextIndices(unknown, kept);
    EXPECT_EQ(kept[0], 9u);
    EXPECT_EQ(kept[1], 2u);
}

TEST(SwapTextureSets, AnyHandleDestroysItsWholeSet) {
    // REGRESSION: DestroySwapTextureSet removed one texture of the three.
    Device driver = makeWarpDevice();
    SwapTextureSets sets;
    uint64_t a[3] = {}, b[3] = {}, c[3] = {};
    std::string error;
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 16, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, a, error));
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 16, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, b, error));
    ASSERT_TRUE(sets.create(driver.device.Get(), 2, 16, 16, DXGI_FORMAT_R8G8B8A8_UNORM, 1, c, error));
    EXPECT_EQ(sets.size(), 9u);

    sets.destroySet(a[2]);
    EXPECT_EQ(sets.size(), 6u);
    for (uint64_t h : a) EXPECT_EQ(sets.find(h), nullptr);
    EXPECT_NE(sets.find(b[0]), nullptr);

    sets.destroyAll(1);
    EXPECT_EQ(sets.size(), 3u) << "only process 1's sets";
    EXPECT_NE(sets.find(c[1]), nullptr);
    sets.destroySet(12345);  // unknown handle: nothing happens
    EXPECT_EQ(sets.size(), 3u);
}

TEST(EyeBlit, CopiesTheLeftEyeOfADoubleWideSrgbTexture) {
    // REGRESSION: a CopyResource from the compositor's R8G8B8A8 texture to
    // NVENC's B8G8R8A8 input did nothing (different format groups), and it
    // copied the whole texture rather than one eye.
    const auto out = runFrame(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 128, 48, halves(128, 48, kOrange, kBlue),
                              {0.0f, 0.0f, 0.5f, 1.0f}, 64, 48);
    // The sRGB bytes survive: decoded by the sampler, encoded back.
    expectAll(out, kOrange, 1, "left eye");
}

TEST(EyeBlit, TakesTheRightEyeFromItsBounds) {
    const auto out = runFrame(DXGI_FORMAT_R8G8B8A8_UNORM, 128, 48, halves(128, 48, kOrange, kBlue),
                              {0.5f, 0.0f, 1.0f, 1.0f}, 64, 48);
    expectAll(out, kBlue, 0, "right eye");
}

TEST(EyeBlit, ScalesToTheEncodedSize) {
    // SteamVR's supersampling renders the eye larger than the stream.
    const auto out = runFrame(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 256, 96, halves(256, 96, kOrange, kBlue),
                              {0.0f, 0.0f, 0.5f, 1.0f}, 40, 30);
    ASSERT_EQ(out.size(), 40u * 30u * 4u);
    // Away from the seam, where linear filtering blends in the other eye.
    std::vector<uint8_t> inner;
    for (uint32_t y = 0; y < 30; y++)
        for (uint32_t x = 0; x < 38; x++) inner.insert(inner.end(), &out[(y * 40 + x) * 4], &out[(y * 40 + x) * 4 + 4]);
    expectAll(inner, kOrange, 1, "downscaled left eye");
}

TEST(EyeBlit, FollowsFlippedBounds) {
    // Top half orange, bottom half blue; v0 > v1 turns it upside down.
    std::vector<Rgba> px(32 * 32);
    for (uint32_t y = 0; y < 32; y++)
        for (uint32_t x = 0; x < 32; x++) px[y * 32 + x] = y < 16 ? kOrange : kBlue;
    const auto out = runFrame(DXGI_FORMAT_R8G8B8A8_UNORM, 32, 32, px, {0.0f, 1.0f, 1.0f, 0.0f}, 32, 32);
    const std::vector<uint8_t> top(out.begin(), out.begin() + 32 * 4);
    const std::vector<uint8_t> bottom(out.end() - 32 * 4, out.end());
    expectAll(top, kBlue, 0, "top row shows the source's bottom");
    expectAll(bottom, kOrange, 0, "bottom row shows the source's top");
}

TEST(EyeBlit, KeepsTheColorsOfABgraSource) {
    // Bytes B,G,R,A in memory: the sampler swizzles them to RGB, the BGRA
    // output stores them back in the same order.
    const Rgba bgra{kOrange.b, kOrange.g, kOrange.r, 255};
    const auto out = runFrame(DXGI_FORMAT_B8G8R8A8_UNORM, 16, 16, std::vector<Rgba>(16 * 16, bgra), {}, 16, 16);
    expectAll(out, kOrange, 0, "BGRA source");
}

TEST(EyeBlit, EncodesLinearFloatToSrgb) {
    // An HDR (float) eye texture holds linear light; the video wants sRGB.
    Device driver = makeWarpDevice();
    SwapTextureSets sets;
    uint64_t handles[3] = {};
    std::string error;
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 8, 8, DXGI_FORMAT_R16G16B16A16_FLOAT, 1, handles, error))
        << error;
    const float linear[4] = {0.5f, 0.5f, 0.5f, 1.0f};
    ComPtr<ID3D11RenderTargetView> rtv;
    ASSERT_TRUE(SUCCEEDED(driver.device->CreateRenderTargetView(sets.find(handles[0])->texture.Get(), nullptr, &rtv)));
    driver.context->ClearRenderTargetView(rtv.Get(), linear);
    EyeBlit blit;
    ASSERT_TRUE(blit.init(driver.device.Get(), 8, 8, error)) << error;
    ASSERT_TRUE(blit.draw(driver.context.Get(), sets.find(handles[0])->view.Get(),
                          DXGI_FORMAT_R16G16B16A16_FLOAT, {}, 0));
    expectAll(eyeHalf(readBack(driver, blit.output()), 8, 8, 0), Rgba{188, 188, 188, 255}, 1,
              "linear 0.5 → sRGB 188");
}

TEST(EyeBlit, PutsBothEyesSideBySide) {
    // REGRESSION (mono): only the left eye was streamed, and the headset
    // showed it to both eyes. The compositor's double-wide texture: left
    // eye orange, right eye blue; each lands in its half of the output.
    Device driver = makeWarpDevice();
    Device compositor = makeWarpDevice();
    SwapTextureSets sets;
    uint64_t handles[3] = {};
    std::string error;
    ASSERT_TRUE(sets.create(driver.device.Get(), 1, 128, 48, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 1, handles, error))
        << error;
    ComPtr<ID3D11Texture2D> target = openShared(compositor.device.Get(), handles[0]);
    upload(compositor, target.Get(), 128, halves(128, 48, kOrange, kBlue));
    compositor.context->Flush();

    EyeBlit blit;
    ASSERT_TRUE(blit.init(driver.device.Get(), 40, 30, error)) << error;
    const SwapTextureSets::Texture* layer = sets.find(handles[0]);
    ASSERT_TRUE(blit.draw(driver.context.Get(), layer->view.Get(), layer->format, {0.0f, 0.0f, 0.5f, 1.0f}, 0));
    ASSERT_TRUE(blit.draw(driver.context.Get(), layer->view.Get(), layer->format, {0.5f, 0.0f, 1.0f, 1.0f}, 1));
    const std::vector<uint8_t> frame = readBack(driver, blit.output());
    ASSERT_EQ(frame.size(), 80u * 30u * 4u) << "two 40-pixel eyes";

    // Inner columns only: linear filtering blends the other eye in at the
    // source's seam.
    auto inner = [](const std::vector<uint8_t>& eye) {
        std::vector<uint8_t> out;
        for (uint32_t y = 0; y < 30; y++)
            for (uint32_t x = 2; x < 38; x++) out.insert(out.end(), &eye[(y * 40 + x) * 4], &eye[(y * 40 + x) * 4 + 4]);
        return out;
    };
    expectAll(inner(eyeHalf(frame, 40, 30, 0)), kOrange, 1, "left half = left eye");
    expectAll(inner(eyeHalf(frame, 40, 30, 1)), kBlue, 1, "right half = right eye");
}

TEST(EyeBlit, OutputIsWhatNvencRegisters) {
    Device driver = makeWarpDevice();
    EyeBlit blit;
    std::string error;
    ASSERT_TRUE(blit.init(driver.device.Get(), 1832, 1920, error)) << error;
    D3D11_TEXTURE2D_DESC desc;
    blit.output()->GetDesc(&desc);
    EXPECT_EQ(desc.Width, 3664u) << "both eyes side by side";
    EXPECT_EQ(desc.Height, 1920u);
    EXPECT_EQ(desc.Format, DXGI_FORMAT_B8G8R8A8_UNORM) << "NV_ENC_BUFFER_FORMAT_ARGB";
    EXPECT_FALSE(blit.draw(driver.context.Get(), nullptr, DXGI_FORMAT_R8G8B8A8_UNORM, {}, 0))
        << "nothing to draw from";
}

TEST(SyncTexture, WaitsWhileTheCompositorHoldsTheFrame) {
    // REGRESSION: Present ignored syncTexture, so it could read a frame the
    // compositor was still drawing.
    Device driver = makeWarpDevice();
    Device compositor = makeWarpDevice();
    ComPtr<IDXGIKeyedMutex> compositorMutex;
    const uint64_t handle = makeSyncTexture(compositor, compositorMutex);

    SyncTexture sync;
    ASSERT_EQ(compositorMutex->AcquireSync(0, INFINITE), S_OK);
    EXPECT_FALSE(sync.acquire(driver.device.Get(), handle, 10)) << "compositor still drawing";
    ASSERT_EQ(compositorMutex->ReleaseSync(0), S_OK);

    EXPECT_TRUE(sync.acquire(driver.device.Get(), handle, 1000));
    EXPECT_NE(compositorMutex->AcquireSync(0, 10), S_OK) << "the driver holds it now";
    sync.release();
    EXPECT_EQ(compositorMutex->AcquireSync(0, 1000), S_OK) << "released back to the compositor";
    compositorMutex->ReleaseSync(0);
}

TEST(SyncTexture, RejectsAHandleItCannotOpen) {
    Device driver = makeWarpDevice();
    SyncTexture sync;
    EXPECT_FALSE(sync.acquire(driver.device.Get(), 0, 10));
    sync.release();  // harmless without a hold
}
