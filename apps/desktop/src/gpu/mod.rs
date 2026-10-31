// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Graphics adapter choice. The editor draws its UI, the previews and the
//! mask overlay with wgpu. On machines with two GPUs it uses the dedicated
//! one, falling back to integrated graphics and then to a software
//! rasterizer, so it starts everywhere. Settings can prefer integrated
//! graphics, software rendering or one adapter by name (`GpuSetting`, read
//! at start-up); `PHOTONATION_GPU` overrides that (see docs/DESKTOP.md).

pub mod mask_overlay;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Environment variable overriding the adapter order.
pub const GPU_ENV: &str = "PHOTONATION_GPU";
/// Largest texture side requested from the GPU. Previews at 1:1 need the
/// photo's long edge; most GPUs offer 16384. Larger would only waste memory.
const MAX_TEXTURE_SIDE: u32 = 16384;

/// Which kind of adapter to try first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPreference {
    /// Dedicated GPU first: the fastest (the default).
    Discrete,
    /// Integrated graphics first: saves battery on laptops.
    Integrated,
    /// A software rasterizer first, to rule out driver problems.
    Software,
}

impl GpuPreference {
    /// From [`GPU_ENV`]; unset or unrecognized means [`GpuPreference::Discrete`].
    pub fn from_env() -> GpuPreference {
        let Ok(value) = std::env::var(GPU_ENV) else {
            return GpuPreference::Discrete;
        };
        GpuPreference::parse(&value).unwrap_or_else(|| {
            eprintln!(
                "{GPU_ENV}={value:?} is not one of discrete, integrated, software; using discrete"
            );
            GpuPreference::Discrete
        })
    }

    pub fn parse(value: &str) -> Option<GpuPreference> {
        match value.trim().to_ascii_lowercase().as_str() {
            "discrete" | "dedicated" | "high-performance" => Some(GpuPreference::Discrete),
            "integrated" | "low-power" => Some(GpuPreference::Integrated),
            "software" | "cpu" => Some(GpuPreference::Software),
            _ => None,
        }
    }

    /// Device types from most to least preferred. Every order ends with
    /// all types, so some adapter is always found if one can draw.
    fn order(self) -> [wgpu::DeviceType; 5] {
        use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other, VirtualGpu};
        match self {
            GpuPreference::Discrete => [DiscreteGpu, IntegratedGpu, VirtualGpu, Other, Cpu],
            GpuPreference::Integrated => [IntegratedGpu, DiscreteGpu, VirtualGpu, Other, Cpu],
            GpuPreference::Software => [Cpu, IntegratedGpu, DiscreteGpu, VirtualGpu, Other],
        }
    }
}

/// The choice in Settings, saved in `preferences.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuSetting {
    /// Dedicated GPU first, then integrated graphics, then software.
    #[default]
    Automatic,
    /// Integrated graphics first.
    Integrated,
    /// A software rasterizer first.
    Software,
    /// The adapter with this name; [`GpuSetting::Automatic`] if it is gone.
    Adapter(String),
}

impl GpuSetting {
    /// The order and adapter to start with: [`GPU_ENV`] when it is set,
    /// else this setting.
    fn effective(&self) -> (GpuPreference, Option<String>) {
        if overridden_by_env() {
            return (GpuPreference::from_env(), None);
        }
        match self {
            GpuSetting::Automatic => (GpuPreference::Discrete, None),
            GpuSetting::Integrated => (GpuPreference::Integrated, None),
            GpuSetting::Software => (GpuPreference::Software, None),
            GpuSetting::Adapter(name) => (GpuPreference::Discrete, Some(name.clone())),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            GpuSetting::Automatic => "Automatic (dedicated GPU first)",
            GpuSetting::Integrated => "Integrated graphics first",
            GpuSetting::Software => "Software rendering",
            GpuSetting::Adapter(name) => name,
        }
    }
}

/// Whether [`GPU_ENV`] decides instead of the setting.
pub fn overridden_by_env() -> bool {
    std::env::var_os(GPU_ENV).is_some()
}

/// Names of the adapters that could draw the window at start-up, for the
/// choice in Settings.
pub fn available_adapters() -> &'static [String] {
    ADAPTERS.get().map_or(&[], Vec::as_slice)
}

static ADAPTERS: OnceLock<Vec<String>> = OnceLock::new();

/// What adapter selection needs to know about one adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
    /// Whether it can draw to the window.
    pub presents: bool,
}

/// Index of the adapter to use: the one named `wanted` if it can draw to
/// the window, else the most preferred device type that can, with native
/// backends (Metal, Vulkan, DX12) before OpenGL. `None` if no adapter can
/// draw.
pub fn choose(
    candidates: &[Candidate],
    preference: GpuPreference,
    wanted: Option<&str>,
) -> Option<usize> {
    let best = |accept: &dyn Fn(&Candidate) -> bool| {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.presents && accept(c))
            // `min_by_key` keeps the first of equals: enumeration order.
            .min_by_key(|(_, c)| c.backend == wgpu::Backend::Gl)
            .map(|(index, _)| index)
    };
    wanted
        .and_then(|name| best(&|c| c.name == name))
        .or_else(|| {
            preference
                .order()
                .iter()
                .find_map(|&device_type| best(&|c| c.device_type == device_type))
        })
}

/// "AMD Radeon Pro 5500M (discrete GPU, Metal)".
pub fn describe(info: &wgpu::AdapterInfo) -> String {
    let kind = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => "discrete GPU",
        wgpu::DeviceType::IntegratedGpu => "integrated GPU",
        wgpu::DeviceType::VirtualGpu => "virtual GPU",
        wgpu::DeviceType::Cpu => "software rendering",
        wgpu::DeviceType::Other => "GPU",
    };
    format!("{} ({kind}, {:?})", info.name, info.backend)
}

/// eframe's wgpu setup with [`choose`] picking the adapter, and the largest
/// texture size that adapter supports (eframe asks every GPU for 8192, too
/// little for 1:1 previews of high-resolution cameras and more than some
/// software rasterizers offer).
pub fn wgpu_configuration(setting: &GpuSetting) -> WgpuConfiguration {
    let (preference, wanted) = setting.effective();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        let candidates: Vec<Candidate> = adapters
            .iter()
            .map(|adapter| {
                let info = adapter.get_info();
                Candidate {
                    name: info.name,
                    device_type: info.device_type,
                    backend: info.backend,
                    presents: surface.is_none_or(|s| adapter.is_surface_supported(s)),
                }
            })
            .collect();
        let mut names: Vec<String> = candidates
            .iter()
            .filter(|c| c.presents)
            .map(|c| c.name.clone())
            .collect();
        names.dedup();
        ADAPTERS.get_or_init(|| names);
        let found: Vec<String> = adapters.iter().map(|a| describe(&a.get_info())).collect();
        choose(&candidates, preference, wanted.as_deref())
            .map(|index| adapters[index].clone())
            .ok_or_else(|| {
                format!(
                    "no graphics adapter can draw the window (found: {})",
                    if found.is_empty() {
                        "none".to_string()
                    } else {
                        found.join("; ")
                    }
                )
            })
    }));
    setup.device_descriptor = Arc::new(|adapter| {
        let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };
        wgpu::DeviceDescriptor {
            label: Some("photonation device"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: adapter
                    .limits()
                    .max_texture_dimension_2d
                    .min(MAX_TEXTURE_SIDE),
                ..base_limits
            },
            ..Default::default()
        }
    });
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..WgpuConfiguration::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{Backend, DeviceType};

    fn candidate(device_type: DeviceType, backend: Backend) -> Candidate {
        Candidate {
            name: format!("{device_type:?}"),
            device_type,
            backend,
            presents: true,
        }
    }

    /// An Intel MacBook Pro: integrated Intel graphics listed first.
    fn dual_gpu_laptop() -> Vec<Candidate> {
        vec![
            candidate(DeviceType::IntegratedGpu, Backend::Metal),
            candidate(DeviceType::DiscreteGpu, Backend::Metal),
        ]
    }

    #[test]
    fn discrete_gpu_wins_by_default() {
        assert_eq!(
            choose(&dual_gpu_laptop(), GpuPreference::Discrete, None),
            Some(1)
        );
    }

    #[test]
    fn integrated_preference_saves_power() {
        assert_eq!(
            choose(&dual_gpu_laptop(), GpuPreference::Integrated, None),
            Some(0)
        );
    }

    #[test]
    fn falls_back_to_integrated_then_software() {
        let integrated_only = [
            candidate(DeviceType::Cpu, Backend::Vulkan),
            candidate(DeviceType::IntegratedGpu, Backend::Vulkan),
        ];
        assert_eq!(
            choose(&integrated_only, GpuPreference::Discrete, None),
            Some(1)
        );
        let software_only = [candidate(DeviceType::Cpu, Backend::Dx12)];
        assert_eq!(
            choose(&software_only, GpuPreference::Discrete, None),
            Some(0)
        );
        assert_eq!(
            choose(&dual_gpu_laptop(), GpuPreference::Software, None),
            Some(0)
        );
    }

    #[test]
    fn adapters_that_cannot_draw_the_window_are_skipped() {
        let mut adapters = dual_gpu_laptop();
        adapters[1].presents = false; // e.g. a headless compute GPU
        assert_eq!(choose(&adapters, GpuPreference::Discrete, None), Some(0));
        adapters[0].presents = false;
        assert_eq!(choose(&adapters, GpuPreference::Discrete, None), None);
    }

    #[test]
    fn native_backends_beat_opengl_on_the_same_gpu() {
        let adapters = [
            candidate(DeviceType::DiscreteGpu, Backend::Gl),
            candidate(DeviceType::DiscreteGpu, Backend::Vulkan),
        ];
        assert_eq!(choose(&adapters, GpuPreference::Discrete, None), Some(1));
    }

    #[test]
    fn a_named_adapter_wins_while_it_can_draw() {
        let mut adapters = dual_gpu_laptop();
        let integrated = Some("IntegratedGpu");
        assert_eq!(
            choose(&adapters, GpuPreference::Discrete, integrated),
            Some(0)
        );
        adapters[0].presents = false;
        assert_eq!(
            choose(&adapters, GpuPreference::Discrete, integrated),
            Some(1)
        );
        assert_eq!(
            choose(&adapters, GpuPreference::Discrete, Some("unplugged eGPU")),
            Some(1)
        );
    }

    #[test]
    fn preference_parsing_is_forgiving() {
        assert_eq!(
            GpuPreference::parse(" Integrated "),
            Some(GpuPreference::Integrated)
        );
        assert_eq!(
            GpuPreference::parse("dedicated"),
            Some(GpuPreference::Discrete)
        );
        assert_eq!(GpuPreference::parse("cpu"), Some(GpuPreference::Software));
        assert_eq!(GpuPreference::parse("fastest"), None);
    }
}
