//! Process environment, set up before the GUI starts (Linux only: elsewhere there is no glibc nor the
//! Vulkan loader to adjust).
//!
//! glibc and the Vulkan loader read their variables only at startup, and `std::env::set_var` is
//! `unsafe` (the workspace forbids it): to change them the client re-launches itself once with `exec`, which
//! keeps the PID. Measured on Linux with lyrics open (RSS after 15 s of playback):
//!
//! | Ambiente | RSS |
//! |---|---|
//! | predefinito | 367–388 MB |
//! | `MALLOC_ARENA_MAX=2` | 295 MB |
//! | + no NVIDIA Vulkan nor OpenGL driver | 222 MB |

/// Whoever finds it set has already been through here: this avoids re-launching forever.
const MARKER: &str = "PSYTIFY_ENV_READY";

/// If one of these is set, the user has chosen the GPU and backend themselves: nothing is touched.
const GPU_OPT_OUT: [&str; 10] = [
    "PSYTIFY_ALL_GPUS",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "WGPU_BACKEND",
    "WGPU_ADAPTER_NAME",
    "WGPU_POWER_PREF",
    "__NV_PRIME_RENDER_OFFLOAD",
    "DRI_PRIME",
];

/// Vulkan drivers of integrated GPUs (RADV, ANV, AMDVLK): if there is one, iced already draws there
/// (it asks for the low-power GPU) and the NVIDIA driver loads for nothing.
const INTEGRATED_DRIVERS: [&str; 3] = ["radeon", "intel", "amd"];

/// The files that describe the installed Vulkan drivers.
const VULKAN_ICD_DIRS: [&str; 3] =
    ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d", "/usr/local/share/vulkan/icd.d"];

/// The variables to set, given the installed Vulkan drivers and those already present.
fn lean_env(manifests: &[String], is_set: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let mut changes = Vec::new();
    // One thread = one malloc arena: with 40 threads glibc opens 32 and keeps ~75 MB free but not returned.
    if !is_set("MALLOC_ARENA_MAX") {
        changes.push(("MALLOC_ARENA_MAX", "2"));
    }
    if leaves_nvidia_driver_unloaded(manifests, &is_set) {
        // Vulkan only: wgpu's OpenGL would load the NVIDIA and Mesa EGL libraries anyway.
        changes.push(("VK_LOADER_DRIVERS_DISABLE", "*nvidia*"));
        changes.push(("WGPU_BACKEND", "vulkan"));
    }
    changes
}

/// There is an NVIDIA driver and one of an integrated GPU, and the user has not chosen the GPU by hand.
fn leaves_nvidia_driver_unloaded(manifests: &[String], is_set: &impl Fn(&str) -> bool) -> bool {
    if GPU_OPT_OUT.iter().any(|name| is_set(name)) {
        return false;
    }
    let has = |vendor: &str| manifests.iter().any(|manifest| manifest.contains(vendor));
    has("nvidia") && INTEGRATED_DRIVERS.iter().any(|vendor| has(vendor))
}

/// The names (in lowercase) of the installed Vulkan driver files.
fn installed_vulkan_drivers() -> Vec<String> {
    VULKAN_ICD_DIRS
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
        .collect()
}

/// Re-launches the client with the lighter environment, if needed. It must be called first in `main`.
pub fn relaunch_with_lean_env() {
    use std::os::unix::process::CommandExt;

    if std::env::var_os(MARKER).is_some() {
        return;
    }
    let changes = lean_env(&installed_vulkan_drivers(), |name| std::env::var_os(name).is_some());
    if changes.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // `exec` returns only if it fails: in that case the client starts anyway, with the previous environment.
    let _failed = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(MARKER, "1")
        .envs(changes)
        .exec();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drivers(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn nothing_set(_: &str) -> bool {
        false
    }

    #[test]
    fn the_malloc_arenas_are_capped_unless_the_user_chose() {
        assert!(lean_env(&[], nothing_set).contains(&("MALLOC_ARENA_MAX", "2")));
        assert!(lean_env(&[], |name| name == "MALLOC_ARENA_MAX").is_empty());
    }

    #[test]
    fn the_nvidia_driver_is_skipped_when_an_integrated_one_is_there() {
        let changes = lean_env(&drivers(&["nvidia_icd.json", "radeon_icd.json"]), nothing_set);
        assert!(changes.contains(&("VK_LOADER_DRIVERS_DISABLE", "*nvidia*")));
        assert!(changes.contains(&("WGPU_BACKEND", "vulkan")));
    }

    #[test]
    fn an_nvidia_only_machine_keeps_its_driver() {
        let changes = lean_env(&drivers(&["nvidia_icd.json", "lvp_icd.json"]), nothing_set);
        assert!(!changes.iter().any(|(name, _)| *name == "VK_LOADER_DRIVERS_DISABLE"));
    }

    #[test]
    fn a_machine_without_nvidia_is_left_alone() {
        let changes = lean_env(&drivers(&["intel_icd.json"]), nothing_set);
        assert!(!changes.iter().any(|(name, _)| *name == "WGPU_BACKEND"));
    }

    #[test]
    fn the_gpu_is_not_touched_when_the_user_picked_one() {
        let manifests = drivers(&["nvidia_icd.json", "radeon_icd.json"]);
        for opt_out in ["DRI_PRIME", "__NV_PRIME_RENDER_OFFLOAD", "WGPU_BACKEND", "PSYTIFY_ALL_GPUS"] {
            let changes = lean_env(&manifests, |name| name == opt_out);
            assert!(!changes.iter().any(|(name, _)| *name == "VK_LOADER_DRIVERS_DISABLE"), "{opt_out}");
        }
    }
}
