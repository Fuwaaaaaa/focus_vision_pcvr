#include "swap_textures.h"

#include <dxgi.h>
#include <algorithm>
#include <cstdio>

bool SwapTextureSets::create(ID3D11Device* device, uint32_t pid, uint32_t width, uint32_t height,
                             DXGI_FORMAT format, uint32_t sampleCount,
                             uint64_t (&outHandles)[kTexturesPerSet], std::string& error) {
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = format;
    desc.SampleDesc.Count = sampleCount > 0 ? sampleCount : 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    // A legacy (non-NT) shared handle: what the compositor opens with
    // OpenSharedResource. Synchronization is the sync texture's keyed mutex
    // in Present, so these need none of their own.
    desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED;

    const uint32_t setId = m_nextSetId++;
    std::vector<Texture> created;
    char buf[160];
    for (int i = 0; i < kTexturesPerSet; i++) {
        Texture t;
        t.pid = pid;
        t.setId = setId;
        t.format = format;
        HRESULT hr = device->CreateTexture2D(&desc, nullptr, &t.texture);
        if (FAILED(hr)) {
            snprintf(buf, sizeof(buf), "CreateTexture2D %ux%u format %u failed (hr=0x%08lx)",
                     width, height, static_cast<unsigned>(format), static_cast<unsigned long>(hr));
            error = buf;
            return false;
        }
        Microsoft::WRL::ComPtr<IDXGIResource> resource;
        HANDLE shared = nullptr;
        hr = t.texture.As(&resource);
        if (SUCCEEDED(hr)) hr = resource->GetSharedHandle(&shared);
        if (FAILED(hr) || !shared) {
            snprintf(buf, sizeof(buf), "GetSharedHandle failed (hr=0x%08lx)",
                     static_cast<unsigned long>(hr));
            error = buf;
            return false;
        }
        t.handle = reinterpret_cast<uint64_t>(shared);
        // A multisampled texture can't be sampled as a plain Texture2D;
        // EyeBlit reports the missing view instead.
        if (desc.SampleDesc.Count == 1) {
            device->CreateShaderResourceView(t.texture.Get(), nullptr, &t.view);
        }
        created.push_back(std::move(t));
    }

    for (int i = 0; i < kTexturesPerSet; i++) {
        outHandles[i] = created[i].handle;
        m_textures.push_back(std::move(created[i]));
    }
    m_current.emplace_back(setId, 0u);
    return true;
}

const SwapTextureSets::Texture* SwapTextureSets::find(uint64_t handle) const {
    for (const auto& t : m_textures) {
        if (t.handle == handle) return &t;
    }
    return nullptr;
}

void SwapTextureSets::destroySet(uint64_t handle) {
    const Texture* t = find(handle);
    if (!t) return;
    const uint32_t setId = t->setId;
    m_textures.erase(std::remove_if(m_textures.begin(), m_textures.end(),
                                    [setId](const Texture& e) { return e.setId == setId; }),
                     m_textures.end());
    m_current.erase(std::remove_if(m_current.begin(), m_current.end(),
                                   [setId](const auto& e) { return e.first == setId; }),
                    m_current.end());
}

void SwapTextureSets::destroyAll(uint32_t pid) {
    m_textures.erase(std::remove_if(m_textures.begin(), m_textures.end(),
                                    [pid](const Texture& e) { return e.pid == pid; }),
                     m_textures.end());
    // Drop the indices of sets that no longer have textures.
    m_current.erase(std::remove_if(m_current.begin(), m_current.end(),
                                   [this](const auto& e) {
                                       return std::none_of(m_textures.begin(), m_textures.end(),
                                                           [&](const Texture& t) { return t.setId == e.first; });
                                   }),
                    m_current.end());
}

void SwapTextureSets::nextIndices(const uint64_t (&handles)[2], uint32_t (&indices)[2]) {
    uint32_t advancedSet = 0;
    bool advanced = false;
    for (int eye = 0; eye < 2; eye++) {
        const Texture* t = find(handles[eye]);
        if (!t) continue;
        for (auto& [setId, index] : m_current) {
            if (setId != t->setId) continue;
            if (!advanced || setId != advancedSet) {
                index = (index + 1) % kTexturesPerSet;
                advancedSet = setId;
                advanced = true;
            }
            indices[eye] = index;
            break;
        }
    }
}
