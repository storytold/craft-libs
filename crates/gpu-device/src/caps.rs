//! What an adapter can do, for choosing a pipeline and for the settings and the log.

use std::fmt;

use wgpu::{AdapterInfo, Backend, DeviceType, Features, Limits};

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

/// What an adapter reports it can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub name: String,
    pub backend: Backend,
    pub device_type: DeviceType,
    pub vendor: Vendor,
    /// The adapter draws on the CPU.
    pub software: bool,
    /// NV12 textures (8-bit 4:2:0, a luma plane and an interleaved chroma plane) can be created
    /// and sampled through plane views. wgpu 30 does not render to them: draw the planes into
    /// `R8Unorm` and `Rg8Unorm` textures instead.
    pub nv12: bool,
    /// P010 textures (10 bits in 16-bit words, 4:2:0) can be created and sampled through plane
    /// views. wgpu 30 does not render to them: draw the planes into `R16Unorm` and `Rg16Unorm`
    /// textures ([`norm16`](Self::norm16)) instead.
    pub p010: bool,
    /// `R16Unorm`, `Rg16Unorm` and `Rgba16Unorm` textures, for 10-bit and higher planes without
    /// P010.
    pub norm16: bool,
    /// `f16` in shaders.
    pub shader_f16: bool,
    /// Timestamp queries, for measuring GPU time per pass.
    pub timestamps: bool,
    /// The widest and tallest 2D texture.
    pub max_texture_2d: u32,
    /// The largest buffer, in bytes.
    pub max_buffer_size: u64,
}

impl Capabilities {
    /// From an adapter's (or a device's) info, features and limits.
    pub fn from_parts(info: &AdapterInfo, features: Features, limits: &Limits) -> Self {
        Capabilities {
            name: info.name.clone(),
            backend: info.backend,
            device_type: info.device_type,
            vendor: Vendor::from_id(info.vendor),
            software: info.device_type == DeviceType::Cpu,
            nv12: features.contains(Features::TEXTURE_FORMAT_NV12),
            p010: features.contains(Features::TEXTURE_FORMAT_P010),
            norm16: features.contains(Features::TEXTURE_FORMAT_16BIT_NORM),
            shader_f16: features.contains(Features::SHADER_F16),
            timestamps: features.contains(Features::TIMESTAMP_QUERY),
            max_texture_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
        }
    }

    /// What `adapter` can do.
    pub fn of_adapter(adapter: &wgpu::Adapter) -> Self {
        Self::from_parts(&adapter.get_info(), adapter.features(), &adapter.limits())
    }

    /// What a device opened on `adapter` can do: the features and limits it was opened with.
    pub fn of_device(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Self {
        Self::from_parts(&adapter.get_info(), device.features(), &device.limits())
    }

    /// Whether a `width` × `height` texture fits.
    pub fn fits(&self, width: u32, height: u32) -> bool {
        width > 0 && height > 0 && width <= self.max_texture_2d && height <= self.max_texture_2d
    }

    /// Whether 10-bit planes can be drawn: 16-bit normalized textures. P010 textures can only be
    /// sampled.
    pub fn ten_bit(&self) -> bool {
        self.norm16
    }

    /// The features from `wanted` this adapter has, to ask for when opening a device.
    pub fn available(adapter_features: Features, wanted: Features) -> Features {
        adapter_features & wanted
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\" ({}, {:?}", self.name, self.vendor, self.backend)?;
        if self.software {
            f.write_str(", software")?;
        }
        let flags =
            [(self.nv12, "NV12"), (self.p010, "P010"), (self.norm16, "16-bit norm"), (self.shader_f16, "f16"), (self.timestamps, "timestamps")];
        for (on, name) in flags {
            if on {
                write!(f, ", {name}")?;
            }
        }
        write!(f, ", textures up to {} px)", self.max_texture_2d)
    }
}

#[cfg(test)]
mod tests {
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
        assert!(c.nv12 && !c.p010 && c.norm16 && c.timestamps && !c.shader_f16 && !c.software);
        assert!(c.ten_bit());
        assert_eq!(c.vendor, Vendor::Nvidia);
        assert!(c.fits(7680, 4320) && c.fits(16384, 16384));
        assert!(!c.fits(16385, 1) && !c.fits(0, 1080) && !c.fits(1920, 0));
        assert_eq!(c.to_string(), "\"NVIDIA GeForce RTX 4070\" (NVIDIA, Vulkan, NV12, 16-bit norm, timestamps, textures up to 16384 px)");
    }

    #[test]
    fn a_software_adapter_without_extras() {
        let limits = Limits::downlevel_defaults();
        let c = Capabilities::from_parts(&info("llvmpipe (LLVM 19.1.0, 256 bits)", 0x10005, DeviceType::Cpu), Features::empty(), &limits);
        assert!(c.software && !c.ten_bit() && !c.nv12);
        // P010 alone is sampled, not drawn to
        let p010 = Capabilities::from_parts(&info("x", 0x8086, DeviceType::IntegratedGpu), Features::TEXTURE_FORMAT_P010, &limits);
        assert!(p010.p010 && !p010.ten_bit());
        assert_eq!(c.max_texture_2d, 2048);
        assert!(!c.fits(3840, 2160));
        assert!(c.to_string().contains("software"));
        let wanted = Features::TEXTURE_FORMAT_NV12 | Features::SHADER_F16;
        assert_eq!(Capabilities::available(Features::SHADER_F16 | Features::TIMESTAMP_QUERY, wanted), Features::SHADER_F16);
    }
}
