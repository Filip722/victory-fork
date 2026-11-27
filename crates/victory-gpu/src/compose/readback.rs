//! Game-free Series S interface plane GPU readback.

use super::*;

#[test]
#[ignore = "requires the physical NVIDIA RTX 5060 Laptop GPU"]
fn physical_rtx_composites_transparent_half_alpha_and_opaque_pixels() {
    use crate::host::host;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory4,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM as RGBA8;

    let host = host().expect("the runtime D3D12 device");
    assert!(!host.software(), "a physical adapter is required");
    // SAFETY: query the adapter that owns the device actually used below.
    let adapter = unsafe {
        CreateDXGIFactory2::<IDXGIFactory4>(DXGI_CREATE_FACTORY_FLAGS(0))
            .and_then(|factory| {
                factory.EnumAdapterByLuid::<IDXGIAdapter1>(host.device.GetAdapterLuid())
            })
            .and_then(|adapter| adapter.GetDesc1())
            .expect("adapter for the runtime D3D12 device")
    };
    let name = String::from_utf16_lossy(&adapter.Description);
    let name = name.trim_end_matches('\0');
    assert_eq!(adapter.VendorId, 0x10de, "NVIDIA adapter required");
    assert!(name.contains("RTX 5060 Laptop GPU"), "RTX 5060 Laptop GPU required, got {name}");
    eprintln!("interface compositor adapter {name}");
    let texture = |width: u32, height, flags, state| {
        let mut resource: Option<ID3D12Resource> = None;
        // SAFETY: plain creation of a texture with one RGBA8 subresource.
        unsafe {
            host.device
                .CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES {
                        Type: D3D12_HEAP_TYPE_DEFAULT,
                        ..Default::default()
                    },
                    D3D12_HEAP_FLAG_NONE,
                    &D3D12_RESOURCE_DESC {
                        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                        Width: u64::from(width),
                        Height: height,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        Format: RGBA8,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Flags: flags,
                        ..Default::default()
                    },
                    state,
                    None,
                    &mut resource,
                )
                .unwrap();
        }
        resource.unwrap()
    };
    let sub = |r: &ID3D12Resource| D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: the copy location borrows this resource.
        pResource: unsafe { std::mem::transmute_copy(r) },
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: 0,
        },
    };
    let rows = |r: &ID3D12Resource| D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: the copy location borrows this resource.
        pResource: unsafe { std::mem::transmute_copy(r) },
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: 0,
                Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                    Format: RGBA8,
                    Width: 5,
                    Height: 3,
                    Depth: 1,
                    RowPitch: 256,
                },
            },
        },
    };
    assert!(!straight(), "the pixel fixture expects premultiplied alpha");
    let mut compositor = Compose::new(&host.device, RGBA8)
        .expect("the compositor shaders, root signature and pipeline");
    let rtv = D3D12_RESOURCE_STATE_RENDER_TARGET;
    let present = D3D12_RESOURCE_STATE_PRESENT;
    let game = texture(5, 3, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, rtv);
    let ui = texture(5, 3, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, rtv);
    let target = texture(5, 3, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, present);
    let readback = host.read_back_buffer(768).unwrap();
    let commands = host.new_list().unwrap();
    let list = &commands.list;
    // SAFETY: both views and all resources live through command completion.
    unsafe {
        let scene_view = host.rtv(Some(&game), None);
        let ui_view = host.rtv(Some(&ui), None);
        list.ClearRenderTargetView(scene_view, &[0.0, 0.0, 1.0, 1.0], None);
        list.ClearRenderTargetView(ui_view, &[0.0; 4], None);
        let strip = |left, right| RECT {
            left,
            right,
            top: 0,
            bottom: 3,
        };
        list.ClearRenderTargetView(
            ui_view,
            &[0.0, 128.0 / 255.0, 0.0, 128.0 / 255.0],
            Some(&[strip(2, 3)]),
        );
        list.ClearRenderTargetView(ui_view, &[1.0, 0.0, 0.0, 1.0], Some(&[strip(3, 5)]));
        compositor.record(
            &host.device,
            list,
            &target,
            (5, 3),
            &game,
            rtv.0 as u32,
            &ui,
            rtv.0 as u32,
        );
        list.ResourceBarrier(&[transition(&target, present, D3D12_RESOURCE_STATE_COPY_SOURCE)]);
        list.CopyTextureRegion(&rows(&readback), 0, 0, 0, &sub(&target), None);
    }
    commands.close();
    host.execute(&[&commands]);
    // SAFETY: query the runtime device after the completed submission.
    unsafe { host.device.GetDeviceRemovedReason() }.expect("compositor device remains ready");
    // SAFETY: execute waited for the copy. Each row contains five texels.
    let mut ptr = std::ptr::null_mut();
    let mut pixels = [[0u8; 20]; 3];
    unsafe {
        readback.Map(0, None, Some(&mut ptr)).unwrap();
        for (y, row) in pixels.iter_mut().enumerate() {
            row.copy_from_slice(std::slice::from_raw_parts(
                ptr.cast::<u8>().add(y * 256),
                20,
            ));
        }
        readback.Unmap(0, None);
    }
    let expected = [
        [0, 0, 255, 255],
        [0, 0, 255, 255],
        [0, 128, 127, 255],
        [255, 0, 0, 255],
        [255, 0, 0, 255],
    ];
    for row in pixels {
        assert_eq!(&row[..], expected.as_flattened());
    }
    eprintln!("interface compositor matched all 15 physical GPU pixels");
}
