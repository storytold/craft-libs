//! What an adapter can do, for choosing a pipeline and for the settings and the log.

use std::fmt;
use std::ops::Deref;

use craft_gpu::DeviceCaps;
use wgpu::{AdapterInfo, DeviceType, Features, Limits};

/// The maker of a GPU, from its PCI vendor id (or the Khronos vendor id some Vulkan drivers
/// report instead).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Qualcomm,
    Arm,
    ImgTec,
    /// Microsoft's software adapter (WARP) and its virtual GPUs.
    Microsoft,
    /// Mesa's software drivers (llvmpipe, lavapipe).
    Mesa,
    /// Another id, or none (0).
    Other(u32),
}

impl Vendor {
    pub fn from_id(id: u32) -> Vendor {
        match id {
            0x10de => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            0x106b => Vendor::Apple,
            0x5143 => Vendor::Qualcomm,
            0x13b5 => Vendor::Arm,
            0x1010 => Vendor::ImgTec,
            0x1414 => Vendor::Microsoft,
            0x10005 => Vendor::Mesa,
            other => Vendor::Other(other),
        }
    }
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Apple => "Apple",
            Vendor::Qualcomm => "Qualcomm",
            Vendor::Arm => "Arm",
            Vendor::ImgTec => "Imagination",
            Vendor::Microsoft => "Microsoft",
            Vendor::Mesa => "Mesa",
            Vendor::Other(0) => return f.write_str("unknown vendor"),
            Vendor::Other(id) => return write!(f, "vendor {id:#06x}"),
        };
        f.write_str(name)
    }
}

/// What a device (or an adapter) can do, read from the [`DeviceCaps`] a `craft-gpu` context
/// reports. It holds only that `DeviceCaps`: the video formats, `f16` and timestamps come from its
/// feature names, the vendor from its vendor id, and its fields (the name, the backend and the
/// limits) can be read through `Deref`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities(DeviceCaps);

impl Capabilities {
    /// From an adapter's (or a device's) info, features and limits.
    pub fn from_parts(info: &AdapterInfo, features: Features, limits: &Limits) -> Self {
        Capabilities(device_caps_of(info, features, limits))
    }

    /// What `adapter` offers, for a settings page before a device is opened. A `craft-gpu`
    /// context reports [`of_device`](Self::of_device) instead: a device can be granted less.
    pub fn of_adapter(adapter: &wgpu::Adapter) -> Self {
        Self::from_parts(&adapter.get_info(), adapter.features(), &adapter.limits())
    }

    /// What `device` can do: the features and limits it was opened with.
    pub fn of_device(device: &wgpu::Device) -> Self {
        Self::from_parts(&device.adapter_info(), device.features(), &device.limits())
    }

    /// The `DeviceCaps` these capabilities read.
    pub fn device_caps(&self) -> &DeviceCaps {
        &self.0
    }

    pub fn into_device_caps(self) -> DeviceCaps {
        self.0
    }

    /// Whether every feature in `features` is named in the caps.
    fn has(&self, features: Features) -> bool {
        features.iter_names().all(|(name, _)| self.0.has_feature(name))
    }

    /// NV12 textures (8-bit 4:2:0, a luma plane and an interleaved chroma plane) can be created
    /// and sampled through plane views. wgpu 30 does not render to them: draw the planes into
    /// `R8Unorm` and `Rg8Unorm` textures instead.
    pub fn nv12(&self) -> bool {
        self.has(Features::TEXTURE_FORMAT_NV12)
    }

    /// P010 textures (10 bits in 16-bit words, 4:2:0) can be created and sampled through plane
    /// views. wgpu 30 does not render to them: draw the planes into `R16Unorm` and `Rg16Unorm`
    /// textures ([`norm16`](Self::norm16)) instead.
    pub fn p010(&self) -> bool {
        self.has(Features::TEXTURE_FORMAT_P010)
    }

    /// `R16Unorm`, `Rg16Unorm` and `Rgba16Unorm` textures, for 10-bit and higher planes without
    /// P010.
    pub fn norm16(&self) -> bool {
        self.has(Features::TEXTURE_FORMAT_16BIT_NORM)
    }

    /// `f16` in shaders.
    pub fn shader_f16(&self) -> bool {
        self.has(Features::SHADER_F16)
    }

    /// Timestamp queries, for measuring GPU time per pass.
    pub fn timestamps(&self) -> bool {
        self.has(Features::TIMESTAMP_QUERY)
    }

    pub fn vendor(&self) -> Vendor {
        Vendor::from_id(self.0.vendor_id)
    }

    /// The adapter draws on the CPU.
    pub fn software(&self) -> bool {
        self.0.software
    }

    /// Whether a `width` × `height` texture fits.
    pub fn fits(&self, width: u32, height: u32) -> bool {
        let max = self.0.max_texture_dimension_2d;
        width > 0 && height > 0 && width <= max && height <= max
    }

    /// Whether 10-bit planes can be drawn: 16-bit normalized textures. P010 textures can only be
    /// sampled.
    pub fn ten_bit(&self) -> bool {
        self.norm16()
    }

    /// The features from `wanted` this adapter has, to ask for when opening a device.
    pub fn available(adapter_features: Features, wanted: Features) -> Features {
        adapter_features & wanted
    }
}

impl Deref for Capabilities {
    type Target = DeviceCaps;

    fn deref(&self) -> &DeviceCaps {
        &self.0
    }
}

impl From<DeviceCaps> for Capabilities {
    fn from(caps: DeviceCaps) -> Self {
        Capabilities(caps)
    }
}

impl From<Capabilities> for DeviceCaps {
    fn from(caps: Capabilities) -> Self {
        caps.0
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\" ({}, {}", self.0.device_name, self.vendor(), self.0.backend_name)?;
        if self.software() {
            f.write_str(", software")?;
        }
        let flags = [
            (self.nv12(), "NV12"),
            (self.p010(), "P010"),
            (self.norm16(), "16-bit norm"),
            (self.shader_f16(), "f16"),
            (self.timestamps(), "timestamps"),
        ];
        for (on, name) in flags {
            if on {
                write!(f, ", {name}")?;
            }
        }
        write!(f, ", textures up to {} px)", self.0.max_texture_dimension_2d)
    }
}

/// The [`DeviceCaps`] a `craft-gpu` context reports for `device`: the limits and features the
/// device was opened with, not the adapter's, in the same form `craft-gpu-wgpu` reports for a
/// device it opens itself.
pub fn device_caps(device: &wgpu::Device) -> DeviceCaps {
    Capabilities::of_device(device).into_device_caps()
}

fn device_caps_of(info: &AdapterInfo, features: Features, limits: &Limits) -> DeviceCaps {
    DeviceCaps {
        backend_name: format!("{:?}", info.backend),
        device_name: info.name.clone(),
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_buffer_size: limits.max_buffer_size,
        max_texture_dimension_2d: limits.max_texture_dimension_2d,
        features: features.iter_names().map(|(name, _)| name.to_string()).collect(),
        vendor_id: info.vendor,
        software: info.device_type == DeviceType::Cpu,
    }
}

#[cfg(test)]
mod tests {
    use wgpu::Backend;

    use super::*;

    fn info(name: &str, vendor: u32, device_type: DeviceType) -> AdapterInfo {
        AdapterInfo { name: name.into(), vendor, ..AdapterInfo::new(device_type, Backend::Vulkan) }
    }

    #[test]
    fn vendors_from_pci_and_khronos_ids() {
        assert_eq!(Vendor::from_id(0x10de), Vendor::Nvidia);
        assert_eq!(Vendor::from_id(0x1002), Vendor::Amd);
        assert_eq!(Vendor::from_id(0x8086), Vendor::Intel);
        assert_eq!(Vendor::from_id(0x10005), Vendor::Mesa);
        assert_eq!(Vendor::from_id(0x1414), Vendor::Microsoft);
        assert_eq!(Vendor::from_id(0), Vendor::Other(0));
        assert_eq!(Vendor::Other(0).to_string(), "unknown vendor");
        assert_eq!(Vendor::from_id(0x1234).to_string(), "vendor 0x1234");
    }

    #[test]
    fn features_and_limits_become_capabilities() {
        let features = Features::TEXTURE_FORMAT_NV12 | Features::TEXTURE_FORMAT_16BIT_NORM | Features::TIMESTAMP_QUERY;
        let limits = Limits { max_texture_dimension_2d: 16384, ..Limits::default() };
        let c = Capabilities::from_parts(&info("NVIDIA GeForce RTX 4070", 0x10de, DeviceType::DiscreteGpu), features, &limits);
        assert!(c.nv12() && !c.p010() && c.norm16() && c.timestamps() && !c.shader_f16() && !c.software());
        assert!(c.ten_bit());
        assert_eq!(c.vendor(), Vendor::Nvidia);
        assert!(c.fits(7680, 4320) && c.fits(16384, 16384));
        assert!(!c.fits(16385, 1) && !c.fits(0, 1080) && !c.fits(1920, 0));
        assert_eq!(c.to_string(), "\"NVIDIA GeForce RTX 4070\" (NVIDIA, Vulkan, NV12, 16-bit norm, timestamps, textures up to 16384 px)");
    }

    #[test]
    fn a_software_adapter_without_extras() {
        let limits = Limits::downlevel_defaults();
        let c = Capabilities::from_parts(&info("llvmpipe (LLVM 19.1.0, 256 bits)", 0x10005, DeviceType::Cpu), Features::empty(), &limits);
        assert!(c.software() && !c.ten_bit() && !c.nv12());
        // P010 alone is sampled, not drawn to
        let p010 = Capabilities::from_parts(&info("x", 0x8086, DeviceType::IntegratedGpu), Features::TEXTURE_FORMAT_P010, &limits);
        assert!(p010.p010() && !p010.ten_bit());
        assert_eq!(c.max_texture_dimension_2d, 2048);
        assert!(!c.fits(3840, 2160));
        assert!(c.to_string().contains("software"));
        let wanted = Features::TEXTURE_FORMAT_NV12 | Features::SHADER_F16;
        assert_eq!(Capabilities::available(Features::SHADER_F16 | Features::TIMESTAMP_QUERY, wanted), Features::SHADER_F16);
    }

    #[test]
    fn device_caps_carry_the_device_limits_and_feature_names() {
        let limits =
            Limits { max_texture_dimension_2d: 16384, max_buffer_size: 1 << 30, max_storage_buffer_binding_size: 1 << 28, ..Limits::default() };
        let features = Features::TEXTURE_FORMAT_NV12 | Features::TIMESTAMP_QUERY;
        let caps = device_caps_of(&info("NVIDIA GeForce RTX 4070", 0x10de, DeviceType::DiscreteGpu), features, &limits);
        assert_eq!(caps.backend_name, "Vulkan");
        assert_eq!(caps.device_name, "NVIDIA GeForce RTX 4070");
        assert_eq!((caps.max_texture_dimension_2d, caps.max_buffer_size, caps.max_storage_buffer_binding_size), (16384, 1 << 30, 1 << 28));
        assert_eq!((caps.vendor_id, caps.software), (0x10de, false));
        let mut names = caps.features.clone();
        names.sort();
        assert_eq!(names, ["TEXTURE_FORMAT_NV12", "TIMESTAMP_QUERY"]);
        // a device opened without features reports none
        let bare = device_caps_of(&info("llvmpipe", 0x10005, DeviceType::Cpu), Features::empty(), &Limits::downlevel_defaults());
        assert!(bare.features.is_empty());
        assert_eq!(bare.max_texture_dimension_2d, 2048);
        assert_eq!((bare.vendor_id, bare.software), (0x10005, true));
    }

    #[test]
    fn caps_from_a_craft_gpu_context_read_as_capabilities() {
        let caps = DeviceCaps {
            backend_name: "Dx12".into(),
            device_name: "Intel(R) Arc(TM) A770".into(),
            features: vec!["TEXTURE_FORMAT_P010".into(), "SHADER_F16".into(), "TEXTURE_FORMAT_16BIT_NORM".into()],
            vendor_id: 0x8086,
            ..DeviceCaps::default()
        };
        let c = Capabilities::from(caps.clone());
        assert!(c.p010() && c.shader_f16() && c.ten_bit() && !c.nv12() && !c.timestamps() && !c.software());
        assert_eq!(c.vendor(), Vendor::Intel);
        assert_eq!(c.to_string(), "\"Intel(R) Arc(TM) A770\" (Intel, Dx12, P010, 16-bit norm, f16, textures up to 8192 px)");
        assert_eq!(DeviceCaps::from(c), caps);
        // the caps of a backend with no device claim nothing
        let none = Capabilities::from(DeviceCaps::default());
        assert!(!none.nv12() && !none.p010() && !none.norm16() && !none.shader_f16() && !none.timestamps() && !none.software());
        assert_eq!(none.vendor(), Vendor::Other(0));
    }
}
